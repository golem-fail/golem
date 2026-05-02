use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::Step;

/// Toggle dark mode on or off.
pub(crate) async fn handle_dark_mode(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let enabled = step
        .params
        .get("enabled")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| anyhow::anyhow!("dark_mode action requires 'enabled' param"))?;
    driver.set_dark_mode(enabled).await
}

/// Set GPS coordinates on the device.
pub(crate) async fn handle_set_location(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let latitude = step
        .params
        .get("latitude")
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .ok_or_else(|| anyhow::anyhow!("set_location action requires 'latitude' param"))?;
    let longitude = step
        .params
        .get("longitude")
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .ok_or_else(|| anyhow::anyhow!("set_location action requires 'longitude' param"))?;
    driver.set_location(latitude, longitude).await
}

/// Press a hardware button (home, back, volume_up, etc.).
pub(crate) async fn handle_press(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let button = step
        .params
        .get("button")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("press action requires 'button' param"))?;
    driver.press_button(button).await
}

/// Grant an app permission.
pub(crate) async fn handle_grant_permission(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let bundle_id = step
        .app
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No app specified for {} action", step.action))?;
    let permission = step
        .params
        .get("permission")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("grant_permission action requires 'permission' param"))?;
    driver.grant_permission(bundle_id, permission).await
}

/// Revoke an app permission.
pub(crate) async fn handle_revoke_permission(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let bundle_id = step
        .app
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No app specified for {} action", step.action))?;
    let permission = step
        .params
        .get("permission")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("revoke_permission action requires 'permission' param"))?;
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

}
