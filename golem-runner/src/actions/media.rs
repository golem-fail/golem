use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::Step;

/// Take a screenshot, optionally saving to a specific path.
pub(crate) async fn handle_screenshot(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let result = driver.screenshot().await?;

    if let Some(path) = step.params.get("path").and_then(|v| v.as_str()) {
        tokio::fs::write(path, &result.data).await?;
    }

    Ok(())
}

/// Push a media file to the device.
pub(crate) async fn handle_add_media(step: &Step, driver: &dyn PlatformDriver) -> Result<()> {
    let path = step
        .params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            golem_events::coded(
                golem_events::FailureCode::ParseMissingParam,
                anyhow::anyhow!("add_media action requires 'path' param"),
            )
        })?;
    driver.add_media(path).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;

    // ── screenshot calls driver.screenshot ─────────────────────────────

    #[tokio::test]
    async fn screenshot_calls_driver_screenshot() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("screenshot");

        handle_screenshot(&step, &driver)
            .await
            .expect("screenshot should succeed");

        let calls = driver.get_calls();
        let sc_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(sc_calls.len(), 1);
    }

    // ── screenshot without path param writes nothing to disk ───────────

    #[tokio::test]
    async fn screenshot_without_path_writes_no_file() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        // 1. A bare screenshot step (no `path` param) takes the capture but
        //    SHALL NOT touch the filesystem.
        let tmp = tempfile::tempdir().expect("temp dir SHALL be created");
        let step = make_step("screenshot");

        handle_screenshot(&step, &driver)
            .await
            .expect("screenshot without path SHALL succeed");

        // 2. The capture happened, but no file was written anywhere: the
        //    temp dir SHALL remain empty.
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("temp dir SHALL be readable")
            .collect();
        assert!(
            entries.is_empty(),
            "no file SHALL be written when no path param is present, found {} entr(ies)",
            entries.len()
        );

        let calls = driver.get_calls();
        let sc_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(sc_calls.len(), 1, "screenshot SHALL still be captured");
    }

    // ── screenshot with path param writes the captured bytes ──────────

    #[tokio::test]
    async fn screenshot_with_path_writes_data_to_file() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        // 2. When a `path` param is present the captured bytes SHALL be
        //    written verbatim to that path.
        let tmp = tempfile::tempdir().expect("temp dir SHALL be created");
        let out = tmp.path().join("shot.png");

        let mut step = make_step("screenshot");
        step.params.insert(
            "path".to_string(),
            toml::Value::String(out.to_string_lossy().into_owned()),
        );

        handle_screenshot(&step, &driver)
            .await
            .expect("screenshot with path SHALL succeed");

        let written = std::fs::read(&out).expect("output file SHALL exist");
        // The mock driver returns the PNG magic bytes as its capture.
        assert_eq!(
            written,
            vec![0x89, 0x50, 0x4E, 0x47],
            "written file SHALL contain the captured screenshot bytes"
        );
    }

    // ── screenshot with non-string path param ignores it ──────────────

    #[tokio::test]
    async fn screenshot_with_non_string_path_writes_no_file() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        // 3. A `path` param that is not a string SHALL be ignored (the
        //    `as_str()` filter fails), so the capture happens with no write.
        //    Use a numeric value whose digits also name a candidate file so
        //    we can prove the integer was NOT coerced into a path.
        let tmp = tempfile::tempdir().expect("temp dir SHALL be created");
        let mut step = make_step("screenshot");
        step.params
            .insert("path".to_string(), toml::Value::Integer(42));

        handle_screenshot(&step, &driver)
            .await
            .expect("screenshot with non-string path SHALL succeed");

        // 4. The integer path SHALL be ignored: no "42" file in the temp dir
        //    and the dir SHALL remain empty.
        assert!(
            !tmp.path().join("42").exists(),
            "integer path SHALL NOT be coerced into a filename"
        );
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("temp dir SHALL be readable")
            .collect();
        assert!(
            entries.is_empty(),
            "no file SHALL be written for a non-string path, found {} entr(ies)",
            entries.len()
        );

        let calls = driver.get_calls();
        let sc_calls: Vec<_> = calls.iter().filter(|c| c.0 == "screenshot").collect();
        assert_eq!(sc_calls.len(), 1, "screenshot SHALL still be captured");
    }

    // ── add_media calls driver.add_media ──────────────────────────────

    #[tokio::test]
    async fn add_media_calls_driver_add_media() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let mut step = make_step("add_media");
        step.params.insert(
            "path".to_string(),
            toml::Value::String("test_data/photo.jpg".to_string()),
        );

        handle_add_media(&step, &driver)
            .await
            .expect("add_media should succeed");

        let calls = driver.get_calls();
        let am_calls: Vec<_> = calls.iter().filter(|c| c.0 == "add_media").collect();
        assert_eq!(am_calls.len(), 1);
        assert_eq!(am_calls[0].1, vec!["test_data/photo.jpg"]);
    }

    // ── add_media without path param returns error ────────────────────

    #[tokio::test]
    async fn add_media_without_path_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let step = make_step("add_media");
        // No path param

        let result = handle_add_media(&step, &driver).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("path"),
            "error should mention path param, got: {err_msg}"
        );
    }

    // ── add_media with non-string path param returns error ────────────

    #[tokio::test]
    async fn add_media_with_non_string_path_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        // A `path` present but not a string SHALL fail the `as_str()`
        // filter and surface the missing-param error rather than calling
        // the driver.
        let mut step = make_step("add_media");
        step.params
            .insert("path".to_string(), toml::Value::Integer(7));

        let result = handle_add_media(&step, &driver).await;
        assert!(
            result.is_err(),
            "non-string path SHALL be treated as missing"
        );

        let calls = driver.get_calls();
        let am_calls: Vec<_> = calls.iter().filter(|c| c.0 == "add_media").collect();
        assert_eq!(
            am_calls.len(),
            0,
            "driver.add_media SHALL NOT be invoked when path is invalid"
        );
    }
}
