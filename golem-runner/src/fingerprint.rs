//! Source-tree fingerprint for the persistent install cache.
//!
//! Two tiers, both pure-Rust:
//!
//! 1. **Git tier** — when the project is a git repo, fingerprint =
//!    `(HEAD rev, sha1(porcelain status))`. ~10ms, gitignore-aware free.
//! 2. **Content-hash tier** — fallback for non-git projects. Uses the
//!    `ignore` crate (same engine as ripgrep) to walk the tree honouring
//!    `.gitignore` if present, then sha1's the (path, content-sha1) pairs.
//!    ~50-500ms depending on tree size.
//!
//! When neither tier produces a fingerprint (no git repo + walk fails),
//! [`Fingerprint::None`] is returned. A `None` fingerprint matches no other
//! fingerprint, so the cache becomes effectively disabled — every run
//! rebuilds. That's the graceful-degrade path when we can't be sure the
//! source state hasn't changed.

use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

/// Source-tree fingerprint at a point in time. Compared by value to decide
/// whether a previously-cached install is still valid.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fingerprint {
    /// Project is a git repo. `rev` = `git rev-parse HEAD`; `porcelain` =
    /// sha1 of `git status --porcelain` output (catches uncommitted edits,
    /// new untracked files honouring `.gitignore`).
    Git { rev: String, porcelain: String },
    /// Project is not a git repo (or `git` is absent). `hash` = sha1 over
    /// `(relative_path, sha1(content))` pairs in sorted order, walking via
    /// the `ignore` crate.
    Content { hash: String },
    /// Fingerprint could not be computed. Matches no other fingerprint —
    /// the cache treats this as "always miss".
    None,
}

impl Fingerprint {
    /// Compute the fingerprint for `project_root`.
    ///
    /// Tries git first; if `git rev-parse` fails (binary absent, not a
    /// repo), falls back to the content tier. If the content walk fails
    /// (unreadable root, etc.), returns `Fingerprint::None`.
    ///
    /// This is a blocking call. Both tiers are quick enough (≤500ms in
    /// practice) that callers run it on the runtime thread without spawn-
    /// blocking. If a future project hits ~1s walks, switch to a
    /// `tokio::task::spawn_blocking` wrapper.
    pub fn compute(project_root: &Path) -> Self {
        if let Some(g) = git_fingerprint(project_root) {
            return g;
        }
        if let Some(c) = content_fingerprint(project_root) {
            return c;
        }
        Fingerprint::None
    }

    /// `true` when this fingerprint can be meaningfully compared. `None`
    /// returns `false` so callers can short-circuit cache reads.
    pub fn is_some(&self) -> bool {
        !matches!(self, Fingerprint::None)
    }

    /// Short label for verbose logging. Carries enough of the fingerprint
    /// identity that two distinct fingerprints render distinctly, so users
    /// can see *why* a cache hit / miss occurred from the log alone.
    ///
    /// - `git:<rev7>` — clean working tree, only the commit identifies the
    ///   source state
    /// - `git:<rev7>+<porcelain4>` — dirty tree, the porcelain hash is the
    ///   second identity bit the cache compares against
    /// - `content:<hash8>` — non-git content fingerprint
    /// - `none` — fingerprint disabled
    ///
    /// Note: cache hits require all three integrity gates (device-present,
    /// install-time match, fingerprint match), not just this label match.
    /// The label is the *source* identity; gate composition is documented
    /// in [`crate::installer::PersistedInstall`].
    pub fn short_label(&self) -> String {
        match self {
            Fingerprint::Git { rev, porcelain } => {
                let r = &rev[..rev.len().min(7)];
                // sha1("") = da39a3ee... — clean working tree. Skip the
                // porcelain suffix in that case so clean-tree labels stay
                // short.
                const CLEAN_PORCELAIN: &str = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
                if porcelain == CLEAN_PORCELAIN {
                    format!("git:{r}")
                } else {
                    let p = &porcelain[..porcelain.len().min(4)];
                    format!("git:{r}+{p}")
                }
            }
            Fingerprint::Content { hash } => {
                let n = hash.len().min(8);
                format!("content:{}", &hash[..n])
            }
            Fingerprint::None => "none".to_string(),
        }
    }
}

fn git_fingerprint(project_root: &Path) -> Option<Fingerprint> {
    let rev_out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(project_root)
        .output()
        .ok()?;
    if !rev_out.status.success() {
        return None;
    }
    let rev = String::from_utf8_lossy(&rev_out.stdout).trim().to_string();
    if rev.is_empty() {
        return None;
    }

    // `git status --porcelain` lists tracked-modified + untracked-not-ignored
    // entries one per line. Hashing the output catches every relevant
    // working-tree change without enumerating files ourselves.
    let porc_out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(project_root)
        .output()
        .ok()?;
    if !porc_out.status.success() {
        return None;
    }
    let porcelain = sha1_hex(&porc_out.stdout);

    Some(Fingerprint::Git { rev, porcelain })
}

fn content_fingerprint(project_root: &Path) -> Option<Fingerprint> {
    use ignore::WalkBuilder;

    // Build a walker that:
    //  - honours `.gitignore` if present (the default)
    //  - skips common build / vendored directories explicitly so a non-git
    //    project without an ignore file still gets a sane fingerprint
    let mut wb = WalkBuilder::new(project_root);
    wb.standard_filters(true)
        .hidden(false)
        // Treat `.gitignore` as a regular ignore source so non-git
        // projects still honour the same rules. The `ignore` crate's
        // built-in gitignore handling only kicks in inside a real repo.
        .add_custom_ignore_filename(".gitignore")
        .add_custom_ignore_filename(".golemignore");
    // Build-output dirs that we never want in the fingerprint.
    let extra_ignores = [
        "target", "node_modules", ".golem", "build", "DerivedData",
        ".gradle", "dist", ".next", ".cache", "Pods", ".git",
        ".idea", ".vscode",
    ];
    let walker = wb.build();

    let mut entries: Vec<(String, [u8; 20])> = Vec::new();
    for dent in walker {
        let dent = match dent {
            Ok(d) => d,
            Err(_) => continue,
        };
        let path = dent.path();
        // Skip directories — only file content goes into the hash.
        if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        // Manual prune for the extra ignores. `ignore::WalkBuilder` does
        // most of this via `standard_filters`, but a stray `target/` in a
        // non-git project without `.gitignore` would slip through.
        if path
            .components()
            .any(|c| extra_ignores.contains(&c.as_os_str().to_string_lossy().as_ref()))
        {
            continue;
        }
        let rel = path
            .strip_prefix(project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            // Skip unreadable files (broken symlinks, permission denied)
            // rather than failing the whole fingerprint.
            Err(_) => continue,
        };
        let mut hasher = Sha1::new();
        hasher.update(&bytes);
        let digest: [u8; 20] = hasher.finalize().into();
        entries.push((rel, digest));
    }

    if entries.is_empty() {
        return None;
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    // Final hash = sha1 over each (path, content-hash) pair.
    let mut h = Sha1::new();
    for (path, digest) in &entries {
        h.update(path.as_bytes());
        h.update([0u8]);
        h.update(digest);
        h.update([0u8]);
    }
    let final_hash: [u8; 20] = h.finalize().into();
    Some(Fingerprint::Content {
        hash: hex_lower(&final_hash),
    })
}

fn sha1_hex(input: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(input);
    let digest: [u8; 20] = h.finalize().into();
    hex_lower(&digest)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn fingerprint_none_label() {
        assert_eq!(Fingerprint::None.short_label(), "none");
        assert!(!Fingerprint::None.is_some());
    }

    #[test]
    fn content_fingerprint_stable_across_calls() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("b.txt"), "world").unwrap();
        let a = Fingerprint::compute(dir.path());
        let b = Fingerprint::compute(dir.path());
        assert_eq!(a, b, "fingerprint SHALL be stable for unchanged tree");
        assert!(matches!(a, Fingerprint::Content { .. }));
    }

    #[test]
    fn content_fingerprint_changes_when_file_edited() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let a = Fingerprint::compute(dir.path());
        std::fs::write(dir.path().join("a.txt"), "hello!").unwrap();
        let b = Fingerprint::compute(dir.path());
        assert_ne!(a, b, "edit SHALL change fingerprint");
    }

    #[test]
    fn content_fingerprint_changes_when_file_added() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let a = Fingerprint::compute(dir.path());
        std::fs::write(dir.path().join("b.txt"), "world").unwrap();
        let b = Fingerprint::compute(dir.path());
        assert_ne!(a, b, "new file SHALL change fingerprint");
    }

    #[test]
    fn content_fingerprint_skips_target_dir() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let a = Fingerprint::compute(dir.path());
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/blob.bin"), vec![0u8; 1024]).unwrap();
        let b = Fingerprint::compute(dir.path());
        assert_eq!(a, b, "target/ SHALL NOT contribute to fingerprint");
    }

    #[test]
    fn content_fingerprint_honours_gitignore() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "ignored.bin\n").unwrap();
        let a = Fingerprint::compute(dir.path());
        std::fs::write(dir.path().join("ignored.bin"), vec![0u8; 1024]).unwrap();
        let b = Fingerprint::compute(dir.path());
        assert_eq!(a, b, "gitignored file SHALL NOT contribute to fingerprint");
    }

    #[test]
    fn fingerprint_serde_roundtrip() {
        let g = Fingerprint::Git {
            rev: "abc123".into(),
            porcelain: "deadbeef".into(),
        };
        let s = serde_json::to_string(&g).unwrap();
        let back: Fingerprint = serde_json::from_str(&s).unwrap();
        assert_eq!(g, back);
        let n: Fingerprint = serde_json::from_str(&serde_json::to_string(&Fingerprint::None).unwrap()).unwrap();
        assert_eq!(n, Fingerprint::None);
    }

    #[test]
    fn short_label_clean_tree_no_porcelain_suffix() {
        let g = Fingerprint::Git {
            rev: "abc1234567890".into(),
            porcelain: "da39a3ee5e6b4b0d3255bfef95601890afd80709".into(), // sha1("")
        };
        assert_eq!(g.short_label(), "git:abc1234",
            "clean working tree SHALL omit the porcelain suffix");
    }

    #[test]
    fn short_label_dirty_tree_includes_porcelain() {
        let g = Fingerprint::Git {
            rev: "abc1234567890".into(),
            porcelain: "0a1b2c3d4e5f6789".into(),
        };
        assert_eq!(g.short_label(), "git:abc1234+0a1b",
            "dirty working tree SHALL include 4-char porcelain suffix");
    }

    #[test]
    fn short_label_content_truncates() {
        let c = Fingerprint::Content {
            hash: "8f2a1bcd123456".into(),
        };
        assert_eq!(c.short_label(), "content:8f2a1bcd");
    }

    // 1. is_some() distinguishes the disabled None variant from real ones.
    #[test]
    fn is_some_true_for_git_and_content() {
        let g = Fingerprint::Git {
            rev: "abc".into(),
            porcelain: "def".into(),
        };
        let c = Fingerprint::Content { hash: "0123".into() };
        assert!(g.is_some(), "Git fingerprint SHALL be is_some");
        assert!(c.is_some(), "Content fingerprint SHALL be is_some");
        assert!(!Fingerprint::None.is_some(), "None SHALL NOT be is_some");
    }

    // 2. A rev shorter than 7 chars SHALL be emitted whole (min() guards the slice).
    #[test]
    fn short_label_short_rev_not_truncated() {
        let g = Fingerprint::Git {
            rev: "abc".into(),
            porcelain: "da39a3ee5e6b4b0d3255bfef95601890afd80709".into(),
        };
        assert_eq!(g.short_label(), "git:abc",
            "rev shorter than 7 chars SHALL be emitted whole");
    }

    // 3. A dirty porcelain shorter than 4 chars SHALL be emitted whole.
    #[test]
    fn short_label_short_porcelain_not_truncated() {
        let g = Fingerprint::Git {
            rev: "abcdef0".into(),
            porcelain: "12".into(),
        };
        assert_eq!(g.short_label(), "git:abcdef0+12",
            "porcelain shorter than 4 chars SHALL be emitted whole");
    }

    // 4. A content hash shorter than 8 chars SHALL be emitted whole.
    #[test]
    fn short_label_short_content_hash_not_truncated() {
        let c = Fingerprint::Content { hash: "abc".into() };
        assert_eq!(c.short_label(), "content:abc",
            "content hash shorter than 8 chars SHALL be emitted whole");
    }

    // 5. An empty tree yields no entries, so compute SHALL fall through to None.
    #[test]
    fn compute_empty_dir_is_none() {
        let dir = tempdir().expect("tempdir");
        let f = Fingerprint::compute(dir.path());
        assert_eq!(f, Fingerprint::None,
            "empty non-git tree SHALL produce None");
        assert!(!f.is_some());
    }

    // 6. A directory whose only contents are inside ignored dirs SHALL be None.
    #[test]
    fn compute_only_ignored_content_is_none() {
        let dir = tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("node_modules")).expect("mkdir");
        std::fs::write(dir.path().join("node_modules/dep.js"), "x").expect("write");
        let f = Fingerprint::compute(dir.path());
        assert_eq!(f, Fingerprint::None,
            "tree with only ignored content SHALL produce None");
    }

    // 7. The .golemignore custom ignore file SHALL exclude matching files.
    #[test]
    fn content_fingerprint_honours_golemignore() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), "hello").expect("write a");
        std::fs::write(dir.path().join(".golemignore"), "secret.bin\n").expect("write ignore");
        let a = Fingerprint::compute(dir.path());
        std::fs::write(dir.path().join("secret.bin"), vec![0u8; 64]).expect("write secret");
        let b = Fingerprint::compute(dir.path());
        assert_eq!(a, b, ".golemignore'd file SHALL NOT contribute to fingerprint");
    }

    // 8. Renaming a file (same content) changes the (path,hash) pair set, so
    //    the fingerprint SHALL change even though byte content is identical.
    #[test]
    fn content_fingerprint_changes_when_file_renamed() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), "payload").expect("write a");
        let a = Fingerprint::compute(dir.path());
        std::fs::remove_file(dir.path().join("a.txt")).expect("rm a");
        std::fs::write(dir.path().join("b.txt"), "payload").expect("write b");
        let b = Fingerprint::compute(dir.path());
        assert_ne!(a, b, "rename SHALL change fingerprint (path is part of the hash)");
    }

    // 9. Path is folded into the hash alongside content: two trees that carry
    //    identical file *content* but differ only in how the path string is
    //    split across files SHALL produce distinct fingerprints. With identical
    //    20-byte content digests in both trees, the only thing that can drive a
    //    difference is the path bytes participating in the fold.
    #[test]
    fn content_fingerprint_path_participates_in_hash() {
        // 9a. Tree 1: files "a" and "bc", both holding the same content.
        let dir1 = tempdir().expect("tempdir1");
        std::fs::write(dir1.path().join("a"), "shared").expect("write a");
        std::fs::write(dir1.path().join("bc"), "shared").expect("write bc");
        let f1 = Fingerprint::compute(dir1.path());

        // 9b. Tree 2: files "ab" and "c" with the same shared content, so the
        //     per-file content digests match tree 1's exactly — only the path
        //     split differs.
        let dir2 = tempdir().expect("tempdir2");
        std::fs::write(dir2.path().join("ab"), "shared").expect("write ab");
        std::fs::write(dir2.path().join("c"), "shared").expect("write c");
        let f2 = Fingerprint::compute(dir2.path());

        // 9c. Both are Content fingerprints (non-git tempdirs).
        assert!(matches!(f1, Fingerprint::Content { .. }),
            "tree 1 SHALL be a Content fingerprint");
        assert!(matches!(f2, Fingerprint::Content { .. }),
            "tree 2 SHALL be a Content fingerprint");
        // 9d. Equal content, different paths => different fingerprint, proving
        //     the path string is hashed in, not just the content.
        assert_ne!(f1, f2,
            "trees with equal content but distinct path splits SHALL differ");
    }

    // 10. The persisted-cache on-disk form for Content SHALL be the
    //     snake_case-tagged shape `{"kind":"content","hash":"…"}`. This is a
    //     load-bearing contract: the install cache reads these back across
    //     runs, so the exact tag + field layout is pinned to a literal here.
    //     (The derive round-trip itself is already covered by
    //     fingerprint_serde_roundtrip; this asserts the wire format instead.)
    #[test]
    fn content_fingerprint_serialized_wire_format() {
        let c = Fingerprint::Content { hash: "8f2a1bcd".into() };
        let s = serde_json::to_string(&c).expect("serialize");
        assert_eq!(s, r#"{"kind":"content","hash":"8f2a1bcd"}"#,
            "Content SHALL serialize to the snake_case kind-tagged wire form");
    }

    // 11. Git tier: a fresh repo with one commit and a clean tree SHALL
    //     produce a Git fingerprint whose porcelain is the empty-output sha1,
    //     so its label omits the porcelain suffix.
    #[test]
    fn compute_git_repo_clean_tree_is_git_variant() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(root)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .expect("git runs");
            assert!(out.status.success(),
                "git {:?} SHALL succeed: {}", args, String::from_utf8_lossy(&out.stderr));
        };
        run(&["init", "-q"]);
        std::fs::write(root.join("a.txt"), "hello").expect("write");
        run(&["add", "a.txt"]);
        run(&["commit", "-q", "-m", "init"]);

        let f = Fingerprint::compute(root);
        match &f {
            Fingerprint::Git { rev, porcelain } => {
                assert!(!rev.is_empty(), "git rev SHALL be non-empty");
                assert_eq!(porcelain, "da39a3ee5e6b4b0d3255bfef95601890afd80709",
                    "clean tree SHALL hash empty porcelain output");
                assert!(f.short_label().starts_with("git:"),
                    "git fingerprint SHALL render a git: label");
                assert!(!f.short_label().contains('+'),
                    "clean tree label SHALL omit the porcelain suffix");
            }
            other => panic!("git repo SHALL produce Git variant, got {other:?}"),
        }
    }

    // 12. Git tier: an uncommitted edit SHALL flip the porcelain hash away
    //     from the clean value, producing a dirty label with the + suffix.
    #[test]
    fn compute_git_repo_dirty_tree_changes_porcelain() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(root)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .expect("git runs");
            assert!(out.status.success(),
                "git {:?} SHALL succeed: {}", args, String::from_utf8_lossy(&out.stderr));
        };
        run(&["init", "-q"]);
        std::fs::write(root.join("a.txt"), "hello").expect("write");
        run(&["add", "a.txt"]);
        run(&["commit", "-q", "-m", "init"]);
        let clean = Fingerprint::compute(root);

        // Add an untracked file — porcelain output is now non-empty.
        std::fs::write(root.join("dirty.txt"), "x").expect("write dirty");
        let dirty = Fingerprint::compute(root);

        match (&clean, &dirty) {
            (Fingerprint::Git { rev: r1, porcelain: p1 },
             Fingerprint::Git { rev: r2, porcelain: p2 }) => {
                assert_eq!(r1, r2, "rev SHALL be unchanged by a working-tree edit");
                assert_ne!(p1, p2, "uncommitted change SHALL change the porcelain hash");
                assert!(dirty.short_label().contains('+'),
                    "dirty tree label SHALL include the porcelain suffix");
            }
            _ => panic!("both computations SHALL be Git variants"),
        }
    }
}
