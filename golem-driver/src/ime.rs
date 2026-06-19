//! Host-side lifecycle for golem's Unicode input IME (Android only).
//!
//! `adb shell input text` is ASCII-only, so to type Unicode we activate
//! a headless IME bundled in the companion's main app (see
//! `GolemImeService`) and commit text by broadcasting to it. This module
//! owns that lifecycle: lazy set-once activation, the durable
//! original-IME record used to restore the device's keyboard, and the
//! restore paths.
//!
//! Restore is layered so the user's keyboard is always put back:
//!   1. **In-session (primary):** every activated device is tracked in a
//!      process-global registry; [`restore_all`] runs at suite teardown.
//!   2. **Next-run self-heal:** the original IME is also persisted to
//!      `.golem/`. [`self_heal`] runs at device init — if the current
//!      default IME is golem's and a record exists, it restores; if the
//!      record is missing, it falls back to `ime reset` (system default).
//!   3. **Backstop:** whenever the original is unknown, `ime reset`
//!      restores the platform default without needing to know the user's
//!      specific keyboard.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Fully-qualified component id of the bundled IME.
pub fn golem_ime_id() -> &'static str {
    "fail.golem.companion/.GolemImeService"
}

/// Broadcast action the IME's receiver listens on.
const ACTION_INPUT: &str = "fail.golem.companion.INPUT_TEXT";

/// Process-global map of `device serial → original default IME` for
/// every device whose keyboard we switched this run. Drained by
/// [`restore_all`] at suite teardown.
fn activations() -> &'static Mutex<HashMap<String, String>> {
    static MAP: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Run an `adb -s <serial> <args>` command, returning stdout. Mirrors
/// `AndroidDriver::adb` but standalone so the restore paths (which have
/// only a serial, not a driver) can use it.
async fn adb(serial: &str, args: &[&str]) -> Result<String> {
    let output = tokio::process::Command::new("adb")
        .arg("-s")
        .arg(serial)
        .args(args)
        .output()
        .await
        .context("failed to spawn adb")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("adb -s {serial} {args:?} failed: {stderr}");
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Path to the persisted original-IME record for a device, under the
/// project-local `.golem/` dir (CWD-relative, like the install cache).
fn record_path(serial: &str) -> PathBuf {
    let safe: String = serial
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    PathBuf::from(".golem").join(format!("ime-original-{safe}"))
}

/// Parse the `settings get secure default_input_method` output into a
/// normalised IME id. The command prints the id followed by a newline,
/// or the literal `null` when no default is set.
pub fn parse_default_ime(output: &str) -> Option<String> {
    let t = output.trim();
    if t.is_empty() || t == "null" {
        None
    } else {
        Some(t.to_string())
    }
}

/// Standard base64 (with padding, no line breaks). Used to carry the
/// UTF-8 text in an `am broadcast` string extra, sidestepping shell
/// quoting and argv charset ambiguity. Decoded device-side via
/// `android.util.Base64.DEFAULT`.
pub fn base64_encode(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Ensure golem's Unicode IME is the active input method on this device.
///
/// Set-once per driver via `flag`. On first call: record the current
/// default IME (unless it's already golem's — never poison the record),
/// enable + set golem's IME, then poll until the switch is observed so
/// the subsequent focus tap lands under the new IME.
pub async fn ensure_active(serial: &str, flag: &AtomicBool) -> Result<()> {
    if flag.load(Ordering::Relaxed) {
        return Ok(());
    }
    let golem = golem_ime_id();
    let current = parse_default_ime(
        &adb(serial, &["shell", "settings", "get", "secure", "default_input_method"]).await?,
    );
    match current.as_deref() {
        Some(c) if c == golem => {
            // Already golem's (a prior run left it active and self-heal
            // hasn't run, or we're re-entering). Don't record it as the
            // original — keep any existing record intact.
        }
        Some(c) => {
            // Persist the durable record (next-run self-heal) and register
            // for in-session restore.
            if let Some(parent) = record_path(serial).parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let _ = tokio::fs::write(record_path(serial), c).await;
            activations()
                .lock()
                .expect("ime activations mutex poisoned")
                .insert(serial.to_string(), c.to_string());
        }
        None => {
            // No default IME recorded on the device — register with an
            // empty original so restore falls back to `ime reset`.
            activations()
                .lock()
                .expect("ime activations mutex poisoned")
                .entry(serial.to_string())
                .or_default();
        }
    }
    // `ime enable` then `ime set` — both block until the
    // InputMethodManagerService applies the change, so no confirmation
    // poll is needed (it would just burn the type step's timeout
    // budget). A short fixed settle covers the service bind + the
    // focused field's input-connection rebind to the new IME.
    adb(serial, &["shell", "ime", "enable", golem]).await?;
    adb(serial, &["shell", "ime", "set", golem]).await?;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    flag.store(true, Ordering::Relaxed);
    Ok(())
}

/// One step of committing a typed string: either a (non-empty) line to
/// commit via the IME, or a newline to send as `KEYCODE_ENTER`.
#[derive(Debug, PartialEq, Eq)]
enum TypeOp {
    Commit(String),
    Enter,
}

/// Decompose `text` into the ordered ops needed to type it: each line is
/// committed (empty lines skipped), with an `Enter` between consecutive
/// lines. Mirrors the `input text` path's newline handling so Unicode and
/// ASCII typing behave identically.
fn type_ops(text: &str) -> Vec<TypeOp> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut ops = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.is_empty() {
            ops.push(TypeOp::Commit((*line).to_string()));
        }
        if i < lines.len() - 1 {
            ops.push(TypeOp::Enter);
        }
    }
    ops
}

/// Commit `text` into the focused field via the golem IME, one line at
/// a time with `KEYCODE_ENTER` between lines (matching the `input text`
/// path's newline handling). Assumes [`ensure_active`] already ran.
pub async fn commit_text(serial: &str, text: &str) -> Result<()> {
    for op in type_ops(text) {
        match op {
            TypeOp::Commit(line) => commit_line(serial, &line).await?,
            TypeOp::Enter => {
                adb(serial, &["shell", "input", "keyevent", "KEYCODE_ENTER"]).await?;
            }
        }
    }
    Ok(())
}

/// Broadcast a single line to the IME receiver and verify it committed.
/// Retries once if the receiver reports "no input connection" (the
/// focus/bind may still be settling on the first attempt).
async fn commit_line(serial: &str, line: &str) -> Result<()> {
    let b64 = base64_encode(line.as_bytes());
    for attempt in 0..2 {
        let out = adb(
            serial,
            &["shell", "am", "broadcast", "-a", ACTION_INPUT, "--es", "msg_b64", &b64],
        )
        .await?;
        match broadcast_result(&out) {
            Some(0) => return Ok(()),
            Some(2) if attempt == 0 => {
                // No input connection yet — let the rebind settle, retry.
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
            Some(2) => anyhow::bail!(
                "golem IME had no focused input connection to type into (field not focused?)"
            ),
            other => anyhow::bail!(
                "golem IME broadcast did not confirm commit (result={other:?}): {}",
                out.trim()
            ),
        }
    }
    unreachable!("loop returns or bails on both attempts")
}

/// Parse the `result=N` field from `am broadcast` output
/// ("Broadcast completed: result=0, data=ok").
fn broadcast_result(output: &str) -> Option<i32> {
    let idx = output.find("result=")?;
    let rest = &output[idx + "result=".len()..];
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != '-').unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Restore the original keyboard on a device whose IME we switched, then
/// clear its record. `original` empty → `ime reset` (system default).
async fn restore_device(serial: &str, original: &str) {
    let result = if original.is_empty() {
        adb(serial, &["shell", "ime", "reset"]).await
    } else {
        adb(serial, &["shell", "ime", "set", original]).await
    };
    if let Err(e) = result {
        eprintln!("  [ime] restore on {serial} failed: {e}");
    }
    let _ = tokio::fs::remove_file(record_path(serial)).await;
}

/// Restore every device whose IME this process switched. Called at suite
/// teardown — the primary in-session restore.
pub async fn restore_all() {
    let entries: Vec<(String, String)> = {
        let mut map = activations().lock().expect("ime activations mutex poisoned");
        map.drain().collect()
    };
    for (serial, original) in entries {
        restore_device(&serial, &original).await;
    }
}

/// Next-run self-heal: if this device's current default IME is golem's,
/// restore the original from the `.golem/` record (or `ime reset` if the
/// record is gone — e.g. after a hard kill). Best-effort; runs at device
/// init. No-op when the active IME isn't golem's.
pub async fn self_heal(serial: &str) {
    let current = match adb(serial, &["shell", "settings", "get", "secure", "default_input_method"]).await {
        Ok(out) => parse_default_ime(&out),
        Err(_) => return,
    };
    if current.as_deref() != Some(golem_ime_id()) {
        return;
    }
    let original = tokio::fs::read_to_string(record_path(serial))
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match original {
        Some(orig) => {
            eprintln!("  [ime] self-heal on {serial}: restoring original keyboard");
            restore_device(serial, &orig).await;
        }
        None => {
            eprintln!("  [ime] self-heal on {serial}: no record — ime reset");
            restore_device(serial, "").await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_encodes_utf8_unicode() {
        // "こんにちは" — the canonical demo string.
        assert_eq!(base64_encode("こんにちは".as_bytes()), "44GT44KT44Gr44Gh44Gv");
    }

    #[test]
    fn parse_default_ime_handles_id_null_and_blank() {
        assert_eq!(
            parse_default_ime("com.android.inputmethod/.Latin\n").as_deref(),
            Some("com.android.inputmethod/.Latin")
        );
        assert_eq!(parse_default_ime("null\n"), None);
        assert_eq!(parse_default_ime("  \n"), None);
        assert_eq!(parse_default_ime(""), None);
    }

    #[test]
    fn broadcast_result_extracts_code() {
        assert_eq!(
            broadcast_result("Broadcasting: Intent { ... }\nBroadcast completed: result=0, data=ok"),
            Some(0)
        );
        assert_eq!(
            broadcast_result("Broadcast completed: result=2, data=no input connection"),
            Some(2)
        );
        assert_eq!(broadcast_result("Broadcast completed: result=-1"), Some(-1));
        assert_eq!(broadcast_result("no result here"), None);
    }

    #[test]
    fn record_path_sanitises_serial() {
        assert_eq!(
            record_path("emulator-5554"),
            PathBuf::from(".golem/ime-original-emulator-5554")
        );
        // Network serials carry a colon — must not escape the dir.
        assert_eq!(
            record_path("192.168.1.5:5555"),
            PathBuf::from(".golem/ime-original-192_168_1_5_5555")
        );
    }

    #[test]
    fn type_ops_single_line_commits_once_no_enter() {
        assert_eq!(type_ops("hello"), vec![TypeOp::Commit("hello".into())]);
    }

    #[test]
    fn type_ops_multiline_inserts_enter_between() {
        assert_eq!(
            type_ops("a\nb"),
            vec![TypeOp::Commit("a".into()), TypeOp::Enter, TypeOp::Commit("b".into())]
        );
    }

    #[test]
    fn type_ops_empty_middle_line_is_enter_only() {
        // Blank line → just the Enter, no empty commit.
        assert_eq!(
            type_ops("a\n\nb"),
            vec![
                TypeOp::Commit("a".into()),
                TypeOp::Enter,
                TypeOp::Enter,
                TypeOp::Commit("b".into()),
            ]
        );
    }

    #[test]
    fn type_ops_trailing_and_leading_newline() {
        assert_eq!(
            type_ops("a\n"),
            vec![TypeOp::Commit("a".into()), TypeOp::Enter]
        );
        assert_eq!(
            type_ops("\nb"),
            vec![TypeOp::Enter, TypeOp::Commit("b".into())]
        );
    }

    #[test]
    fn type_ops_empty_string_is_noop() {
        assert_eq!(type_ops(""), vec![]);
    }

    #[test]
    fn type_ops_preserves_unicode_line() {
        assert_eq!(
            type_ops("行一 line\nline two"),
            vec![
                TypeOp::Commit("行一 line".into()),
                TypeOp::Enter,
                TypeOp::Commit("line two".into()),
            ]
        );
    }

    #[test]
    fn golem_ime_id_is_main_app_component() {
        // The IME lives in the main (non-test) package so it's installable
        // and discoverable as a system IME.
        assert_eq!(golem_ime_id(), "fail.golem.companion/.GolemImeService");
    }
}
