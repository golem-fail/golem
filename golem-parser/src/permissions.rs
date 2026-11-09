//! Cross-platform permission vocabulary shared by launch-time
//! (`[[flow.apps]].permissions`) and per-launch (`launch` action `permissions =`)
//! maps. One authority both the parser (author-time validation) and the runner
//! (runtime application) agree on; the drivers map the validated `(permission,
//! mode)` pair to their platform primitives (`simctl`/`applesimutils`/`pm`).

/// Validate a `permissions` map entry — a `permission = mode` pair.
///
/// The value (`mode`) enum:
/// - `allow` / `deny` / `ask` — valid for **any** permission (including a raw
///   `android.permission.*` string passed through verbatim). For `location`,
///   `allow` is the foreground/when-in-use grant. `ask` clears the decision, so
///   the app's next request shows the OS prompt.
/// - `limited` — **photos only** (iOS limited-library / Android partial media).
/// - `always` — **location only** (background + foreground; `allow` is
///   foreground-only). There is no `inuse` (use `allow`) or `never` (use `deny`).
///
/// The old `location-always` *key* is rejected: use `location = "always"`.
/// Key vocabulary itself is intentionally open (drivers accept raw platform
/// permission strings), so an unknown key with `allow`/`deny` is allowed — only
/// the value-vs-key pairing is enforced here.
pub fn validate_permission_entry(permission: &str, mode: &str) -> Result<(), String> {
    if permission == "location-always" {
        return Err(
            "the `location-always` permission key was removed — use `location = \"always\"`"
                .to_string(),
        );
    }
    match mode {
        // `location` adds one extra grant mode, `always` (background) — there is
        // no `inuse` (use `allow`) and no `never` (use `deny`).
        "allow" | "deny" | "ask" => Ok(()),
        "limited" if permission == "photos" => Ok(()),
        "always" if permission == "location" => Ok(()),
        "limited" => Err(format!(
            "permission {permission:?} does not accept mode \"limited\" (only `photos` does)"
        )),
        "always" => Err(format!(
            "permission {permission:?} does not accept mode \"always\" (only `location` does)"
        )),
        other => Err(format!(
            "permission {permission:?} has invalid mode {other:?} — \
             expected \"allow\", \"deny\" or \"ask\"\
             {}",
            match permission {
                "photos" => " (or \"limited\")",
                "location" => " (or \"always\" for background)",
                _ => "",
            }
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn universal_allow_deny_ok_for_any_key() {
        assert!(validate_permission_entry("camera", "allow").is_ok());
        assert!(validate_permission_entry("camera", "deny").is_ok());
        // Raw platform permission string passes through with allow/deny.
        assert!(validate_permission_entry("android.permission.CAMERA", "allow").is_ok());
    }

    #[test]
    fn ask_is_valid_for_any_key() {
        for key in ["camera", "notifications", "tracking", "location", "photos"] {
            assert!(validate_permission_entry(key, "ask").is_ok(), "{key}");
        }
        assert!(validate_permission_entry("android.permission.CAMERA", "ask").is_ok());
    }

    #[test]
    fn unknown_mode_names_ask() {
        let err = validate_permission_entry("camera", "reset").expect_err("bad mode");
        assert!(err.contains("\"ask\""), "{err}");
    }

    #[test]
    fn limited_is_photos_only() {
        assert!(validate_permission_entry("photos", "limited").is_ok());
        assert!(validate_permission_entry("camera", "limited").is_err());
    }

    #[test]
    fn location_always_is_location_only() {
        // `always` (background) is location's one extra grant mode.
        assert!(validate_permission_entry("location", "always").is_ok());
        assert!(
            validate_permission_entry("camera", "always").is_err(),
            "camera should reject always"
        );
        // `allow`/`deny` still work for location (foreground grant / denied).
        assert!(validate_permission_entry("location", "allow").is_ok());
        assert!(validate_permission_entry("location", "deny").is_ok());
    }

    #[test]
    fn dropped_location_synonyms_error() {
        // `inuse` → use `allow` (foreground); `never` → use `deny`.
        assert!(validate_permission_entry("location", "inuse").is_err());
        assert!(validate_permission_entry("location", "never").is_err());
    }

    #[test]
    fn removed_location_always_key_errors() {
        let err = validate_permission_entry("location-always", "allow")
            .expect_err("location-always key SHALL be rejected");
        assert!(err.contains("location-always"), "err names the key: {err}");
        assert!(
            err.contains("location = \"always\""),
            "err gives migration: {err}"
        );
    }

    #[test]
    fn unknown_mode_errors_with_hint() {
        let err = validate_permission_entry("photos", "yes").expect_err("bad mode SHALL error");
        assert!(
            err.contains("limited"),
            "photos hint mentions limited: {err}"
        );
        let err = validate_permission_entry("location", "sometimes").expect_err("bad mode");
        assert!(err.contains("always"), "location hint mentions enum: {err}");
    }
}
