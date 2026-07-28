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

/// Detect a gallery-importable media type from a file's leading bytes.
///
/// Content-sniffed (magic bytes), not extension-based: the device gallery keys
/// off the actual bytes, and an honest content check catches a mislabeled or
/// corrupt file that an extension check would wave through. Returns the format
/// label for the supported set (images + common video containers) or `None`.
fn detect_media_kind(head: &[u8]) -> Option<&'static str> {
    let starts = |sig: &[u8]| head.len() >= sig.len() && &head[..sig.len()] == sig;
    // ISO base media (MP4/MOV/HEIC) put `ftyp` at bytes 4..8; the brand at
    // 8..12 distinguishes the container/codec.
    let ftyp_brand = (head.len() >= 12 && &head[4..8] == b"ftyp").then(|| &head[8..12]);

    if starts(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if starts(b"\xFF\xD8\xFF") {
        Some("jpeg")
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some("gif")
    } else if starts(b"BM") {
        Some("bmp")
    } else if starts(b"II\x2A\x00") || starts(b"MM\x00\x2A") {
        Some("tiff")
    } else if starts(b"RIFF") && head.len() >= 12 && &head[8..12] == b"WEBP" {
        Some("webp")
    } else if let Some(brand) = ftyp_brand {
        match brand {
            b"heic" | b"heix" | b"hevc" | b"heim" | b"heis" | b"mif1" | b"msf1" => Some("heif"),
            b"qt  " => Some("mov"),
            // isom/mp42/mp41/M4V / etc. — treat any other ftyp brand as MP4.
            _ => Some("mp4"),
        }
    } else {
        None
    }
}

/// Push a media file to the device gallery.
///
/// Validates the file is a supported image/video *by content* before handing
/// it to the platform (Android `adb push` happily stores a non-media file that
/// then never indexes into MediaStore; iOS `simctl addmedia` errors opaquely).
/// Failing here with `ParseUnsupportedMedia` names the problem at the
/// add_media step instead of surfacing as a mystery assert two steps later.
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

    // Read just the header — enough for every signature we check.
    let mut head = [0u8; 16];
    let n = {
        use tokio::io::AsyncReadExt;
        let mut f = tokio::fs::File::open(path).await.map_err(|e| {
            golem_events::coded(
                golem_events::FailureCode::ParseUnsupportedMedia,
                anyhow::anyhow!("add_media cannot read {path:?}: {e}"),
            )
        })?;
        f.read(&mut head).await.unwrap_or(0)
    };
    if detect_media_kind(&head[..n]).is_none() {
        return Err(golem_events::coded(
            golem_events::FailureCode::ParseUnsupportedMedia,
            anyhow::anyhow!(
                "add_media: {path:?} is not a supported media file (want an image \
                 — png/jpeg/gif/webp/heif/bmp/tiff — or a video — mp4/mov)"
            ),
        ));
    }

    driver.add_media(path).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;

    // ── detect_media_kind: accepts supported formats by magic bytes ────

    #[test]
    fn detect_media_kind_accepts_images_and_video() {
        assert_eq!(detect_media_kind(b"\x89PNG\r\n\x1a\n....."), Some("png"));
        assert_eq!(detect_media_kind(b"\xFF\xD8\xFF\xE0JFIF"), Some("jpeg"));
        assert_eq!(detect_media_kind(b"GIF89a......."), Some("gif"));
        assert_eq!(detect_media_kind(b"BM????????????"), Some("bmp"));
        assert_eq!(detect_media_kind(b"II\x2A\x00????????"), Some("tiff"));
        assert_eq!(detect_media_kind(b"RIFF????WEBPVP8 "), Some("webp"));
        // ISO base media: `ftyp` at 4..8, brand at 8..12.
        assert_eq!(detect_media_kind(b"\x00\x00\x00\x18ftypheic"), Some("heif"));
        assert_eq!(detect_media_kind(b"\x00\x00\x00\x18ftypqt  "), Some("mov"));
        assert_eq!(detect_media_kind(b"\x00\x00\x00\x18ftypisom"), Some("mp4"));
    }

    // ── detect_media_kind: rejects non-media / malformed input ─────────

    #[test]
    fn detect_media_kind_rejects_non_media() {
        assert_eq!(detect_media_kind(b"hello, this is text"), None);
        assert_eq!(detect_media_kind(b""), None);
        assert_eq!(detect_media_kind(b"\x89PN"), None); // truncated PNG signature
        assert_eq!(detect_media_kind(b"ftypmp42"), None); // no length prefix → no ftyp at 4..8
    }

    // ── handle_add_media rejects an unsupported file before the driver ─

    #[tokio::test]
    async fn add_media_rejects_unsupported_file() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);

        let dir = std::env::temp_dir();
        let bad = dir.join("golem_add_media_bad.txt");
        std::fs::write(&bad, b"definitely not an image").expect("write temp file");

        let mut step = make_step("add_media");
        step.params.insert(
            "path".to_string(),
            toml::Value::String(bad.display().to_string()),
        );

        let err = handle_add_media(&step, &driver)
            .await
            .expect_err("unsupported media SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseUnsupportedMedia)
        );
        // The driver must NOT have been asked to push a bad file.
        assert!(!driver.get_calls().iter().any(|c| c.0 == "add_media"));

        let _ = std::fs::remove_file(&bad);
    }

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

        // A real file with valid image magic bytes — handle_add_media
        // content-sniffs before delegating to the driver.
        let path = std::env::temp_dir().join("golem_add_media_ok.jpg");
        std::fs::write(&path, b"\xFF\xD8\xFF\xE0JFIF\x00\x01\x02\x03\x04\x05\x06")
            .expect("write jpg");
        let path_str = path.display().to_string();

        let mut step = make_step("add_media");
        step.params
            .insert("path".to_string(), toml::Value::String(path_str.clone()));

        handle_add_media(&step, &driver)
            .await
            .expect("add_media should succeed");

        let calls = driver.get_calls();
        let am_calls: Vec<_> = calls.iter().filter(|c| c.0 == "add_media").collect();
        assert_eq!(am_calls.len(), 1);
        assert_eq!(am_calls[0].1, vec![path_str]);

        let _ = std::fs::remove_file(&path);
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
