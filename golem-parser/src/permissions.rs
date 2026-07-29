//! Cross-platform permission vocabulary shared by launch-time
//! (`[[flow.apps]].permissions`) and per-launch (`launch` action `permissions =`)
//! maps. One authority both the parser (author-time validation) and the runner
//! (runtime application) agree on; the drivers map the validated `(permission,
//! mode)` pair to their platform primitives (`simctl`/`applesimutils`/`pm`).

/// Validate a `permissions` map entry — a `permission = mode` pair.
///
/// The value (`mode`) enum:
/// - `allow` / `deny` — valid for **any** permission (including a raw
///   `android.permission.*` string passed through verbatim).
/// - `limited` — **photos only** (iOS limited-library / Android partial media).
/// - `always` / `inuse` / `never` — **location only** (`never` ≡ deny).
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
        // `deny` is the one denial spelling (explicit-denied — `simctl privacy
        // revoke` / `pm revoke`, distinct from a not-determined reset). Location
        // scope lives in the grant modes only; there is no `never` (use `deny`).
        "allow" | "deny" => Ok(()),
        "limited" if permission == "photos" => Ok(()),
        "always" | "inuse" if permission == "location" => Ok(()),
        "limited" => Err(format!(
            "permission {permission:?} does not accept mode \"limited\" (only `photos` does)"
        )),
        "always" | "inuse" => Err(format!(
            "permission {permission:?} does not accept mode {mode:?} \
             (only `location` takes always/inuse)"
        )),
        other => Err(format!(
            "permission {permission:?} has invalid mode {other:?} — \
             expected \"allow\" or \"deny\"\
             {}",
            match permission {
                "photos" => " (or \"limited\")",
                "location" => " (or \"always\"/\"inuse\")",
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
    fn limited_is_photos_only() {
        assert!(validate_permission_entry("photos", "limited").is_ok());
        assert!(validate_permission_entry("camera", "limited").is_err());
    }

    #[test]
    fn location_scope_modes_are_location_only() {
        for m in ["always", "inuse"] {
            assert!(
                validate_permission_entry("location", m).is_ok(),
                "location={m}"
            );
            assert!(
                validate_permission_entry("camera", m).is_err(),
                "camera should reject {m}"
            );
        }
    }

    #[test]
    fn never_is_not_a_mode_use_deny() {
        // `never` was dropped — `deny` is the single denial spelling.
        assert!(validate_permission_entry("location", "never").is_err());
        assert!(validate_permission_entry("location", "deny").is_ok());
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
