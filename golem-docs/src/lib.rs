//! The user docs pages, built from the parts in `docs/src`.
//!
//! `docs/src/<page>/` holds one Markdown file per section. A file name is
//! `<order>-<name>.md`; files sort by the number, and `00-intro.md` is the
//! text of its folder before the folder's other parts. The page
//! `docs/<page>.md` is the parts in order, with the table of contents put
//! where the intro has `<!-- toc -->` (or `<!-- toc depth=N -->`).
//!
//! A part's address is its path without the numbers and the `.md`, e.g.
//! `actions-reference/interaction/tap`. A `<name>.llm.md` next to a part is
//! the LLM's text for that part; the page never shows it.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

/// The first line of every generated page.
pub fn header(page: &str) -> String {
    format!("<!-- Generated from docs/src/{page}/ — edit the parts there, then run `GOLEM_UPDATE_DOCS=1 cargo nextest run -p golem-docs`. -->\n")
}

#[derive(Debug, Clone)]
pub struct Part {
    /// e.g. `actions-reference/interaction/tap`.
    pub address: String,
    pub path: PathBuf,
    pub text: String,
    /// The `.llm.md` file next to `path`, if there is one.
    pub llm: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub name: String,
    /// In page order.
    pub parts: Vec<Part>,
}

/// Every page under `src`, by name.
pub fn pages(src: &Path) -> io::Result<Vec<Page>> {
    let mut out = Vec::new();
    for entry in sorted_dir(src)? {
        if entry.is_dir() {
            let name = file_name(&entry);
            let mut parts = Vec::new();
            collect(&entry, &name, &mut parts)?;
            out.push(Page { name, parts });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn sorted_dir(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<io::Result<_>>()?;
    v.sort();
    Ok(v)
}

/// `10-tap.md` → (10, "tap"); `00-intro.md` → (0, "intro").
fn split_order(file: &str) -> io::Result<(u32, String)> {
    let stem = file.strip_suffix(".md").unwrap_or(file);
    let (n, name) = stem
        .split_once('-')
        .ok_or_else(|| bad(file, "no <order>- prefix"))?;
    let n = n
        .parse()
        .map_err(|_| bad(file, "the prefix is not a number"))?;
    Ok((n, name.to_string()))
}

fn bad(file: &str, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("docs/src: {file}: {why}"),
    )
}

fn collect(dir: &Path, address: &str, out: &mut Vec<Part>) -> io::Result<()> {
    let mut entries = Vec::new();
    for p in sorted_dir(dir)? {
        let file = file_name(&p);
        if file.ends_with(".llm.md") || file.starts_with('.') {
            continue;
        }
        if p.is_file() && !file.ends_with(".md") {
            return Err(bad(&file, "not a .md file"));
        }
        let (n, name) = split_order(&file)?;
        entries.push((n, name, p));
    }
    entries.sort_by_key(|(n, _, _)| *n);
    for (n, name, p) in entries {
        if p.is_dir() {
            collect(&p, &format!("{address}/{name}"), out)?;
            continue;
        }
        let at = if n == 0 && name == "intro" {
            address.to_string()
        } else {
            format!("{address}/{name}")
        };
        let llm = p.with_extension("llm.md");
        out.push(Part {
            address: at,
            text: std::fs::read_to_string(&p)?,
            llm: llm.is_file().then_some(llm),
            path: p,
        });
    }
    Ok(())
}

impl Page {
    /// The page as `docs/<name>.md` holds it.
    pub fn render(&self) -> String {
        let body = self
            .parts
            .iter()
            .map(|p| p.text.trim_end())
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut out = header(&self.name);
        for line in body.lines() {
            match toc_depth(line) {
                Some(depth) => out.push_str(&toc(&body, depth)),
                None => {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        out
    }
}

/// `<!-- toc -->` → 3, `<!-- toc depth=N -->` → N.
fn toc_depth(line: &str) -> Option<usize> {
    let inner = line
        .trim()
        .strip_prefix("<!-- toc")?
        .strip_suffix("-->")?
        .trim();
    if inner.is_empty() {
        return Some(3);
    }
    inner.strip_prefix("depth=")?.parse().ok()
}

/// The headings outside code fences: (level, text).
pub fn headings(markdown: &str) -> Vec<(usize, String)> {
    let mut fence = false;
    let mut out = Vec::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        let level = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&level) && line[level..].starts_with(' ') {
            out.push((level, line[level + 1..].trim().to_string()));
        }
    }
    out
}

/// Each heading's anchor as GitHub makes it, in page order.
pub fn anchors(markdown: &str) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    headings(markdown)
        .into_iter()
        .map(|(_, h)| {
            let base = slug(&h);
            let n = seen.entry(base.clone()).or_insert(0);
            let anchor = if *n == 0 {
                base.clone()
            } else {
                format!("{base}-{n}")
            };
            *n += 1;
            anchor
        })
        .collect()
}

/// GitHub's heading anchor: link targets become their text, then lower
/// case, keep letters, digits, `-`, `_` and spaces, spaces become `-`.
pub fn slug(heading: &str) -> String {
    strip_links(heading)
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// `[text](target)` → `text`.
fn strip_links(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|i| open + i) else {
            break;
        };
        let Some(end) = rest[close..].find(')').map(|i| close + i) else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn toc(body: &str, depth: usize) -> String {
    let mut out = String::from("## Contents\n\n");
    for ((level, text), anchor) in headings(body).into_iter().zip(anchors(body)) {
        if (2..=depth).contains(&level) {
            let label = text.split(" — ").next().unwrap_or(&text);
            let _ = writeln!(out, "{}- [{label}](#{anchor})", "  ".repeat(level - 2));
        }
    }
    out
}

/// Each link in `markdown` outside code: its target.
pub fn links(markdown: &str) -> Vec<String> {
    let mut fence = false;
    let mut out = Vec::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        let prose: String = line
            .split('`')
            .enumerate()
            .filter(|(i, _)| i % 2 == 0)
            .map(|(_, s)| s)
            .collect::<Vec<_>>()
            .join(" ");
        let mut rest = prose.as_str();
        while let Some(i) = rest.find("](") {
            rest = &rest[i + 2..];
            let end = rest.find(')').unwrap_or(rest.len());
            out.push(rest[..end].trim().to_string());
            rest = &rest[end..];
        }
    }
    out
}

/// Each link in `pages` (file name in `docs` → text) whose file or anchor
/// does not exist. Links to other pages read the page from `pages` when it
/// is there, else from `docs`.
pub fn broken_links(docs: &Path, pages: &BTreeMap<String, String>) -> Vec<String> {
    let mut cache: HashMap<PathBuf, Option<Vec<String>>> = HashMap::new();
    let mut out = Vec::new();
    for (file, text) in pages {
        for link in links(text) {
            if link.contains("://") || link.starts_with("mailto:") {
                continue;
            }
            let (path, anchor) = link.split_once('#').unwrap_or((&link, ""));
            let target = if path.is_empty() {
                docs.join(file)
            } else {
                docs.join(path)
            };
            let in_pages = target
                .strip_prefix(docs)
                .ok()
                .and_then(|p| pages.get(&p.to_string_lossy().into_owned()));
            if in_pages.is_none() && !target.exists() {
                out.push(format!("{file}: ({link}): no such file"));
                continue;
            }
            if anchor.is_empty() || !path.is_empty() && !path.ends_with(".md") {
                continue;
            }
            let known = cache.entry(target.clone()).or_insert_with(|| {
                in_pages
                    .cloned()
                    .or_else(|| std::fs::read_to_string(&target).ok())
                    .map(|t| anchors(&t))
            });
            if !known
                .as_ref()
                .is_some_and(|a| a.iter().any(|a| a == anchor))
            {
                out.push(format!("{file}: ({link}): no such anchor"));
            }
        }
    }
    out
}

/// Each part's address and the file the LLM reads: the `.llm.md` file
/// when there is one, else the part.
pub fn help_entries(src: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    Ok(pages(src)?
        .into_iter()
        .flat_map(|p| p.parts)
        .map(|p| (p.address, p.llm.unwrap_or(p.path)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_matches_github() {
        assert_eq!(slug("`tap` — Tap an element"), "tap--tap-an-element");
        assert_eq!(
            slug("Geometric containment: `contains` / `inside`"),
            "geometric-containment-contains--inside"
        );
        assert_eq!(
            slug("Project config (`golem.toml`)"),
            "project-config-golemtoml"
        );
        assert_eq!(slug("[Link](x.md) here"), "link-here");
    }

    #[test]
    fn a_repeated_heading_gets_a_number() {
        assert_eq!(
            anchors("# A\n## Fields\n## Fields\n"),
            ["a", "fields", "fields-1"]
        );
    }

    #[test]
    fn a_heading_in_a_code_fence_is_not_a_heading() {
        assert_eq!(headings("## A\n```toml\n# comment\n```\n## B\n").len(), 2);
    }

    #[test]
    fn the_toc_lists_headings_to_its_depth_by_their_label() {
        let body = "# T\n<!-- toc depth=2 -->\n## `tap` — Tap\n### Deep\n## Two\n";
        let page = Page {
            name: "p".into(),
            parts: vec![Part {
                address: "p".into(),
                path: "p".into(),
                text: body.into(),
                llm: None,
            }],
        };
        let out = page.render();
        assert!(out.starts_with("<!-- Generated from docs/src/p/"));
        assert!(
            out.contains("## Contents\n\n- [`tap`](#tap--tap)\n- [Two](#two)\n"),
            "{out}"
        );
        assert!(!out.contains("(#deep)"));
    }

    #[test]
    fn links_skip_code() {
        let md = "[a](x.md) `[b](y.md)`\n```\n[c](z.md)\n```\n[d](#e)";
        assert_eq!(links(md), ["x.md", "#e"]);
    }

    #[test]
    fn the_llm_reads_the_llm_file_and_the_page_does_not() {
        let src = std::env::temp_dir().join(format!("golem-docs-{}", std::process::id()));
        let page = src.join("p");
        std::fs::create_dir_all(page.join("10-group")).expect("mkdir");
        for (file, text) in [
            ("00-intro.md", "# P\n"),
            ("10-group/00-intro.md", "## Group\n"),
            ("10-group/10-one.md", "### One, for humans\n"),
            ("10-group/10-one.llm.md", "### One, for the LLM\n"),
            ("20-two.md", "## Two\n"),
        ] {
            std::fs::write(page.join(file), text).expect("write");
        }
        let pages = pages(&src).expect("pages");
        let entries = help_entries(&src).expect("entries");
        let _ = std::fs::remove_dir_all(&src);

        let addresses: Vec<_> = entries.iter().map(|(a, _)| a.as_str()).collect();
        assert_eq!(addresses, ["p", "p/group", "p/group/one", "p/two"]);
        assert!(entries[2].1.ends_with("10-one.llm.md"));
        let page = pages[0].render();
        assert!(page.contains("### One, for humans"), "{page}");
        assert!(!page.contains("for the LLM"), "{page}");
    }

    #[test]
    fn the_order_prefix_is_a_number() {
        assert_eq!(split_order("10-tap.md").ok(), Some((10, "tap".to_string())));
        assert!(split_order("tap.md").is_err());
        assert!(split_order("x-tap.md").is_err());
    }
}
