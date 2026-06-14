use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::Step;

/// Toggle dark mode on or off.
pub(crate) async fn handle_dark_mode(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let enabled = step
        .params
        .get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| golem_events::coded(golem_events::FailureCode::ParseMissingParam, anyhow::anyhow!("dark_mode action requires 'enabled' param")))?;
    driver.set_dark_mode(enabled).await
}

/// Set GPS coordinates on the device.
pub(crate) async fn handle_set_location(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let latitude = step
        .params
        .get("latitude")
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .ok_or_else(|| golem_events::coded(golem_events::FailureCode::ParseMissingParam, anyhow::anyhow!("set_location action requires 'latitude' param")))?;
    let longitude = step
        .params
        .get("longitude")
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .ok_or_else(|| golem_events::coded(golem_events::FailureCode::ParseMissingParam, anyhow::anyhow!("set_location action requires 'longitude' param")))?;
    driver.set_location(latitude, longitude).await
}

/// Press a hardware button (home, back, volume_up, etc.).
pub(crate) async fn handle_press(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let button = step
        .params
        .get("button")
        .and_then(|v| v.as_str())
        .ok_or_else(|| golem_events::coded(golem_events::FailureCode::ParseMissingParam, anyhow::anyhow!("press action requires 'button' param")))?;
    driver.press_button(button).await
}

/// Grant an app permission.
pub(crate) async fn handle_grant_permission(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let bundle_id = step
        .app
        .as_deref()
        .ok_or_else(|| golem_events::coded(golem_events::FailureCode::ParseMissingParam, anyhow::anyhow!("No app specified for {} action", step.action)))?;
    let permission = step
        .params
        .get("permission")
        .and_then(|v| v.as_str())
        .ok_or_else(|| golem_events::coded(golem_events::FailureCode::ParseMissingParam, anyhow::anyhow!("grant_permission action requires 'permission' param")))?;
    driver.grant_permission(bundle_id, permission).await
}

/// Revoke an app permission.
pub(crate) async fn handle_revoke_permission(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let bundle_id = step
        .app
        .as_deref()
        .ok_or_else(|| golem_events::coded(golem_events::FailureCode::ParseMissingParam, anyhow::anyhow!("No app specified for {} action", step.action)))?;
    let permission = step
        .params
        .get("permission")
        .and_then(|v| v.as_str())
        .ok_or_else(|| golem_events::coded(golem_events::FailureCode::ParseMissingParam, anyhow::anyhow!("revoke_permission action requires 'permission' param")))?;
    driver.revoke_permission(bundle_id, permission).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;

    // ── dark_mode enabled calls driver.set_dark_mode(true) ────────────

    #[tokio::test]
    async fn dark_mode_enabled_calls_driver_set_dark_mode_true() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("dark_mode");
        step.params
            .insert("enabled".to_string(), toml::Value::Boolean(true));

        handle_dark_mode(&step, &driver)
            .await
            .expect("dark_mode should succeed");

        let calls = driver.get_calls();
        let dm_calls: Vec<_> = calls.iter().filter(|c| c.0 == "set_dark_mode").collect();
        assert_eq!(dm_calls.len(), 1);
        assert_eq!(dm_calls[0].1, vec!["true"]);
    }

    // ── dark_mode disabled calls driver.set_dark_mode(false) ──────────

    #[tokio::test]
    async fn dark_mode_disabled_calls_driver_set_dark_mode_false() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("dark_mode");
        step.params
            .insert("enabled".to_string(), toml::Value::Boolean(false));

        handle_dark_mode(&step, &driver)
            .await
            .expect("dark_mode should succeed");

        let calls = driver.get_calls();
        let dm_calls: Vec<_> = calls.iter().filter(|c| c.0 == "set_dark_mode").collect();
        assert_eq!(dm_calls.len(), 1);
        assert_eq!(dm_calls[0].1, vec!["false"]);
    }

    // ── set_location calls driver.set_location with correct coords ────

    #[tokio::test]
    async fn set_location_calls_driver_set_location_with_correct_coords() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("set_location");
        step.params
            .insert("latitude".to_string(), toml::Value::Float(35.6762));
        step.params
            .insert("longitude".to_string(), toml::Value::Float(139.6503));

        handle_set_location(&step, &driver)
            .await
            .expect("set_location should succeed");

        let calls = driver.get_calls();
        let loc_calls: Vec<_> = calls.iter().filter(|c| c.0 == "set_location").collect();
        assert_eq!(loc_calls.len(), 1);
        assert_eq!(loc_calls[0].1, vec!["35.6762", "139.6503"]);
    }

    // ── press home calls driver.press_button("home") ──────────────────

    #[tokio::test]
    async fn press_home_calls_driver_press_button() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("press");
        step.params.insert(
            "button".to_string(),
            toml::Value::String("home".to_string()),
        );

        handle_press(&step, &driver)
            .await
            .expect("press should succeed");

        let calls = driver.get_calls();
        let press_calls: Vec<_> = calls.iter().filter(|c| c.0 == "press_button").collect();
        assert_eq!(press_calls.len(), 1);
        assert_eq!(press_calls[0].1, vec!["home"]);
    }

    // ── grant_permission calls driver.grant_permission ────────────────

    #[tokio::test]
    async fn grant_permission_calls_driver_grant_permission() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("grant_permission");
        step.app = Some("com.example.app".to_string());
        step.params.insert(
            "permission".to_string(),
            toml::Value::String("camera".to_string()),
        );

        handle_grant_permission(&step, &driver)
            .await
            .expect("grant_permission should succeed");

        let calls = driver.get_calls();
        let gp_calls: Vec<_> = calls.iter().filter(|c| c.0 == "grant_permission").collect();
        assert_eq!(gp_calls.len(), 1);
        assert_eq!(gp_calls[0].1, vec!["com.example.app", "camera"]);
    }

    // ── revoke_permission calls driver.revoke_permission ──────────────

    #[tokio::test]
    async fn revoke_permission_calls_driver_revoke_permission() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("revoke_permission");
        step.app = Some("com.example.app".to_string());
        step.params.insert(
            "permission".to_string(),
            toml::Value::String("location".to_string()),
        );

        handle_revoke_permission(&step, &driver)
            .await
            .expect("revoke_permission should succeed");

        let calls = driver.get_calls();
        let rp_calls: Vec<_> = calls
            .iter()
            .filter(|c| c.0 == "revoke_permission")
            .collect();
        assert_eq!(rp_calls.len(), 1);
        assert_eq!(rp_calls[0].1, vec!["com.example.app", "location"]);
    }

    // ── dark_mode without 'enabled' param fails with ParseMissingParam ─

    #[tokio::test]
    async fn dark_mode_missing_enabled_param_errors() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("dark_mode");

        let err = handle_dark_mode(&step, &driver)
            .await
            .expect_err("missing enabled param SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseMissingParam),
            "missing enabled param SHALL be coded ParseMissingParam"
        );
        assert!(
            format!("{err:#}").contains("requires 'enabled' param"),
            "error SHALL mention enabled param, got: {err:#}"
        );
        assert!(
            driver.get_calls().is_empty(),
            "driver SHALL NOT be called when param is missing"
        );
    }

    // ── dark_mode with non-bool 'enabled' param fails ─────────────────

    #[tokio::test]
    async fn dark_mode_non_bool_enabled_param_errors() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("dark_mode");
        step.params.insert(
            "enabled".to_string(),
            toml::Value::String("yes".to_string()),
        );

        let err = handle_dark_mode(&step, &driver)
            .await
            .expect_err("non-bool enabled param SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseMissingParam),
            "non-bool enabled param SHALL be coded ParseMissingParam"
        );
    }

    // ── set_location coerces integer coords to float ──────────────────

    #[tokio::test]
    async fn set_location_coerces_integer_coords_to_float() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("set_location");
        step.params
            .insert("latitude".to_string(), toml::Value::Integer(35));
        step.params
            .insert("longitude".to_string(), toml::Value::Integer(139));

        handle_set_location(&step, &driver)
            .await
            .expect("integer coords SHALL be accepted");

        let calls = driver.get_calls();
        let loc_calls: Vec<_> = calls.iter().filter(|c| c.0 == "set_location").collect();
        assert_eq!(loc_calls.len(), 1);
        assert_eq!(loc_calls[0].1, vec!["35", "139"]);
    }

    // ── set_location missing latitude fails with ParseMissingParam ────

    #[tokio::test]
    async fn set_location_missing_latitude_errors() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("set_location");
        step.params
            .insert("longitude".to_string(), toml::Value::Float(139.6503));

        let err = handle_set_location(&step, &driver)
            .await
            .expect_err("missing latitude SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseMissingParam),
            "missing latitude SHALL be coded ParseMissingParam"
        );
        assert!(
            format!("{err:#}").contains("requires 'latitude' param"),
            "error SHALL mention latitude param, got: {err:#}"
        );
        assert!(
            driver.get_calls().is_empty(),
            "driver SHALL NOT be called when latitude is missing"
        );
    }

    // ── set_location missing longitude fails with ParseMissingParam ───

    #[tokio::test]
    async fn set_location_missing_longitude_errors() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("set_location");
        step.params
            .insert("latitude".to_string(), toml::Value::Float(35.6762));

        let err = handle_set_location(&step, &driver)
            .await
            .expect_err("missing longitude SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseMissingParam),
            "missing longitude SHALL be coded ParseMissingParam"
        );
        assert!(
            format!("{err:#}").contains("requires 'longitude' param"),
            "error SHALL mention longitude param, got: {err:#}"
        );
    }

    // ── press without 'button' param fails with ParseMissingParam ─────

    #[tokio::test]
    async fn press_missing_button_param_errors() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("press");

        let err = handle_press(&step, &driver)
            .await
            .expect_err("missing button param SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseMissingParam),
            "missing button param SHALL be coded ParseMissingParam"
        );
        assert!(
            format!("{err:#}").contains("requires 'button' param"),
            "error SHALL mention button param, got: {err:#}"
        );
        assert!(
            driver.get_calls().is_empty(),
            "driver SHALL NOT be called when button is missing"
        );
    }

    // ── grant_permission without app fails with ParseMissingParam ─────

    #[tokio::test]
    async fn grant_permission_missing_app_errors() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("grant_permission");
        step.params.insert(
            "permission".to_string(),
            toml::Value::String("camera".to_string()),
        );

        let err = handle_grant_permission(&step, &driver)
            .await
            .expect_err("missing app SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseMissingParam),
            "missing app SHALL be coded ParseMissingParam"
        );
        assert!(
            format!("{err:#}").contains("No app specified for grant_permission action"),
            "error SHALL name the action, got: {err:#}"
        );
    }

    // ── grant_permission without 'permission' param fails ─────────────

    #[tokio::test]
    async fn grant_permission_missing_permission_param_errors() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("grant_permission");
        step.app = Some("com.example.app".to_string());

        let err = handle_grant_permission(&step, &driver)
            .await
            .expect_err("missing permission param SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseMissingParam),
            "missing permission param SHALL be coded ParseMissingParam"
        );
        assert!(
            format!("{err:#}").contains("requires 'permission' param"),
            "error SHALL mention permission param, got: {err:#}"
        );
    }

    // ── revoke_permission without app fails with ParseMissingParam ────

    #[tokio::test]
    async fn revoke_permission_missing_app_errors() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("revoke_permission");
        step.params.insert(
            "permission".to_string(),
            toml::Value::String("location".to_string()),
        );

        let err = handle_revoke_permission(&step, &driver)
            .await
            .expect_err("missing app SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseMissingParam),
            "missing app SHALL be coded ParseMissingParam"
        );
        assert!(
            format!("{err:#}").contains("No app specified for revoke_permission action"),
            "error SHALL name the action, got: {err:#}"
        );
    }

    // ── revoke_permission without 'permission' param fails ────────────

    #[tokio::test]
    async fn revoke_permission_missing_permission_param_errors() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("revoke_permission");
        step.app = Some("com.example.app".to_string());

        let err = handle_revoke_permission(&step, &driver)
            .await
            .expect_err("missing permission param SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseMissingParam),
            "missing permission param SHALL be coded ParseMissingParam"
        );
        assert!(
            format!("{err:#}").contains("requires 'permission' param"),
            "error SHALL mention permission param, got: {err:#}"
        );
    }
}
