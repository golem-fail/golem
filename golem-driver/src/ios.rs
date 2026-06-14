use crate::common::{
    build_backspace_body, build_gesture_body, build_long_press_body, build_swipe_body,
    build_tap_body, build_type_body, find_webview_bounds, parse_hierarchy,
    replace_webview_children, CompanionClient,
};
use crate::{PlatformDriver, ScreenshotResult};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use golem_element::Element;

/// Map a cross-platform permission shorthand to the corresponding
/// `simctl privacy` service token. The set deliberately matches the
/// Android normalizer's shorthand vocabulary so flows stay
/// platform-agnostic; the values are simctl tokens (which mostly
/// coincide with the shorthand names by design).
///
/// Notifications aren't here: iOS doesn't accept a `notifications`
/// simctl token and Android prompts identically when the app first
/// requests `POST_NOTIFICATIONS` — let the prompt fire and use
/// `accept_alert` / the companion's UIInterruptionMonitor instead.
fn normalize_ios_permission(permission: &str) -> Result<&str> {
    match permission {
        "camera" => Ok("camera"),
        "microphone" => Ok("microphone"),
        "location" => Ok("location"),
        "location-always" => Ok("location-always"),
        "contacts" => Ok("contacts"),
        "calendar" => Ok("calendar"),
        "photos" => Ok("photos"),
        other => bail!(
            "Unknown iOS permission shorthand: {other:?}. Known shorthands: \
             camera, microphone, location, location-always, contacts, calendar, \
             photos. (Notifications: don't pre-grant — trigger the prompt from \
             the app and use `accept_alert` for cross-platform parity.)"
        ),
    }
}

/// Post-stop grace period on iOS. `simctl terminate` returns when the
/// kill signal is dispatched; the OS still needs time to release the
/// WKWebView's surface/GPU resources before a fresh launch can claim
/// them cleanly. 500ms is empirically enough to avoid the partial-init
/// race observed when stop and launch are back-to-back.
///
/// (The companion's `/launch` already blocks on
/// `XCUIApplication.wait(.runningForeground)` + first window +
/// staticTexts existence, so we don't need a Rust-side post-launch
/// grace — the HTTP response already waits for DOM render.)
const IOS_POST_STOP_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// iOS driver that communicates with an XCUITest companion server via HTTP.
///
/// The companion server runs inside the iOS simulator and exposes
/// endpoints for UI automation (tap, type, swipe, screenshot, etc.).
pub struct IosDriver {
    client: CompanionClient,
    device_id: String,
    bundle_id: String,
    /// True for real hardware, false for a simulator. Set from the
    /// resolved `DeviceInfo.physical` at driver-construction time so
    /// actions that have a sim-only OS backdoor (currently
    /// `push_notification`) can refuse loudly on real devices instead
    /// of getting an opaque error from `simctl push`.
    physical: bool,
    /// WebKit Inspector lifecycle for WKWebView DOM access.
    webkit: std::sync::Mutex<WebKitLifecycle>,
    /// Active `simctl io recordVideo` child + output path. Mirrors
    /// `AndroidDriver.recording` — `start_recording` spawns detached,
    /// `stop_recording` signals + waits for flush.
    recording: tokio::sync::Mutex<Option<IosRecordingState>>,
}

struct IosRecordingState {
    host_path: String,
    child: tokio::process::Child,
}

/// WebKit Inspector connection lifecycle — mirrors `CdpLifecycle` in android.rs.
enum WebKitLifecycle {
    /// Haven't seen a WKWebView yet — no inspector needed.
    Idle,
    /// Background setup task is running.
    SetupInProgress(tokio::sync::oneshot::Receiver<Option<WebKitState>>),
    /// Inspector is connected and ready.
    Ready(WebKitState),
    /// Setup failed — will retry on next WebView sighting.
    Failed,
}

struct WebKitState {
    inspector: crate::webkit::WebKitInspector,
}

/// Convert a `Direction` to swipe coordinate deltas.
///
/// Uses a standard iPhone screen size (390x844) with the center as
/// the origin point and a 200pt gesture distance.
impl IosDriver {
    /// Create a new iOS driver targeting the companion server at the given port.
    pub fn new(device_id: String, bundle_id: String, port: u16, physical: bool) -> Self {
        let client = CompanionClient::new(port);
        Self {
            client,
            device_id,
            bundle_id,
            physical,
            webkit: std::sync::Mutex::new(WebKitLifecycle::Idle),
            recording: tokio::sync::Mutex::new(None),
        }
    }

    /// Return the base URL for the companion server.
    pub fn base_url(&self) -> &str {
        &self.client.base_url
    }

    /// Check companion server health and return device info.
    pub async fn check_health(&self) -> anyhow::Result<crate::common::CompanionHealth> {
        self.client.check_health().await
    }

    /// Wait for companion to become healthy, polling with timeout.
    pub async fn wait_for_health(&self, timeout: std::time::Duration) -> anyhow::Result<crate::common::CompanionHealth> {
        self.client.wait_for_health(timeout).await
    }

    /// Return the device ID.
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Return the bundle ID.
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    /// Evaluate JavaScript in the foreground WKWebView (best-effort).
    ///
    /// Returns `Ok(None)` when there's no inspectable WebView (native
    /// screen, app not yet launched, inspector connection not yet up,
    /// or eval failed). Used to push native-state changes into the
    /// page — e.g. `set_location` calling `__golemSetLocation` so the
    /// test app's UI reflects the new GPS coordinate.
    async fn eval_in_webview(&self, expression: &str) -> Option<String> {
        let mut state = {
            let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
            match std::mem::replace(&mut *wk, WebKitLifecycle::Failed) {
                WebKitLifecycle::Ready(s) => s,
                other => {
                    *wk = other;
                    return None;
                }
            }
        };
        let result = state.inspector.evaluate_js(expression).await;
        let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
        match result {
            Ok(s) => {
                *wk = WebKitLifecycle::Ready(state);
                Some(s)
            }
            Err(e) => {
                if golem_common::is_debug() {
                    eprintln!("  [webkit] eval_in_webview failed: {e}");
                }
                *wk = WebKitLifecycle::Failed;
                None
            }
        }
    }

    /// Run an `xcrun simctl` subcommand.
    async fn simctl(&self, args: &[&str]) -> Result<String> {
        let output = tokio::process::Command::new("xcrun")
            .arg("simctl")
            .args(args)
            .output()
            .await
            .context("failed to spawn xcrun simctl")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(golem_events::coded(
                golem_events::FailureCode::DeviceDriverOpFailed,
                anyhow::anyhow!("xcrun simctl {args:?} failed: {stderr}"),
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// What to do with WebKit Inspector on this hierarchy call.
enum WebKitAction {
    Skip,
    Enrich(WebKitState),
    /// Background setup is mid-flight — block briefly to give it a chance
    /// to finish so the first few hierarchy fetches after a fresh launch
    /// actually see the WebView DOM instead of skipping past it.
    AwaitSetup(tokio::sync::oneshot::Receiver<Option<WebKitState>>),
}

/// Set up WebKit Inspector: discover socket, connect, handshake.
///
/// `target_udid` constrains discovery to one simulator's socket when
/// multiple sims are booted concurrently — without it, two drivers can
/// pick the same socket and end up driving each other's WebView.
async fn setup_webkit(target_udid: &str) -> Option<WebKitState> {
    match crate::webkit::WebKitInspector::connect(Some(target_udid)).await {
        Ok(inspector) => Some(WebKitState { inspector }),
        Err(e) => {
            if golem_common::is_debug() { eprintln!("  [webkit] setup failed: {e}"); }
            None
        }
    }
}

/// Try to enrich a WebView node with WebKit Inspector DOM data.
/// Returns the WebKitState back if successful (for reuse), None if failed.
async fn try_enrich(
    raw: &mut serde_json::Value,
    mut state: WebKitState,
    wv_x: i32,
    wv_y: i32,
) -> Option<WebKitState> {
    match crate::webkit::fetch_webview_dom(&mut state.inspector, wv_x, wv_y).await {
        Some(dom) => {
            replace_webview_children(raw, dom);
            Some(state)
        }
        None => None, // Inspector connection lost
    }
}

#[async_trait]
impl PlatformDriver for IosDriver {
    fn set_request_timeout(&self, timeout: std::time::Duration) {
        self.client.set_request_timeout(timeout);
    }

    async fn get_hierarchy(&self) -> Result<(Element, crate::common::HierarchyMeta)> {
        let text = self.client.get_text("/hierarchy").await?;
        let wrapper: serde_json::Value = serde_json::from_str(&text)
            .context("failed to parse hierarchy JSON")?;

        // Extract tree from wrapper (companion sends {"tree": [...], ...})
        let mut raw = wrapper.get("tree").cloned().unwrap_or(wrapper.clone());

        // Check if hierarchy contains a WKWebView
        // CSS getBoundingClientRect() returns coordinates relative to the web
        // viewport, which starts below the safe area. Add safe_area_top so DOM
        // coordinates match the native accessibility tree (screen coordinates).
        let safe_area_top = wrapper
            .get("safe_area_top")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        if let Some((wv_x, wv_y)) = find_webview_bounds(&raw) {
            let wv_y = wv_y + safe_area_top;
            // Check WebKit state (short lock, no async while held)
            let webkit_action = {
                let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                match &mut *wk {
                    WebKitLifecycle::Idle => {
                        // First WebView sighting — kick off background setup
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let udid = self.device_id.clone();
                        tokio::spawn(async move {
                            let result = setup_webkit(&udid).await;
                            let _ = tx.send(result);
                        });
                        *wk = WebKitLifecycle::SetupInProgress(rx);
                        WebKitAction::Skip
                    }
                    WebKitLifecycle::SetupInProgress(_) => {
                        // Take ownership of the receiver so we can await it
                        // outside the mutex. We restore it (or transition)
                        // after the bounded wait below.
                        let placeholder = WebKitLifecycle::Failed;
                        let old = std::mem::replace(&mut *wk, placeholder);
                        if let WebKitLifecycle::SetupInProgress(rx) = old {
                            WebKitAction::AwaitSetup(rx)
                        } else {
                            WebKitAction::Skip
                        }
                    }
                    WebKitLifecycle::Ready(_) => {
                        // Take ownership of the state for async work
                        let old = std::mem::replace(&mut *wk, WebKitLifecycle::Failed);
                        if let WebKitLifecycle::Ready(state) = old {
                            WebKitAction::Enrich(state)
                        } else {
                            WebKitAction::Skip
                        }
                    }
                    WebKitLifecycle::Failed => {
                        // Retry — the app may have been relaunched
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let udid = self.device_id.clone();
                        tokio::spawn(async move {
                            let result = setup_webkit(&udid).await;
                            let _ = tx.send(result);
                        });
                        *wk = WebKitLifecycle::SetupInProgress(rx);
                        WebKitAction::Skip
                    }
                }
            }; // mutex dropped here

            // If setup is mid-flight, give it a bounded window to land so
            // the first hierarchy fetch after launch can actually enrich.
            // Without this, every fetch during setup returned Skip and the
            // caller's retry budget (typically 10s / ~4 fetches) often
            // expired before the inspector handshake completed on cold boot.
            let webkit_action = if let WebKitAction::AwaitSetup(mut rx) = webkit_action {
                match tokio::time::timeout(std::time::Duration::from_secs(3), &mut rx).await {
                    Ok(Ok(Some(state))) => WebKitAction::Enrich(state),
                    Ok(Ok(None)) | Ok(Err(_)) => {
                        let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                        *wk = WebKitLifecycle::Failed;
                        WebKitAction::Skip
                    }
                    Err(_) => {
                        // Timed out — restore the in-flight receiver so a
                        // later fetch can continue waiting on the same
                        // background task instead of starting a new one.
                        let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                        *wk = WebKitLifecycle::SetupInProgress(rx);
                        WebKitAction::Skip
                    }
                }
            } else {
                webkit_action
            };

            // Now do async WebKit work outside the lock
            if let WebKitAction::Enrich(state) = webkit_action {
                if let Some(state) = try_enrich(&mut raw, state, wv_x, wv_y).await {
                    // Put state back
                    let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                    *wk = WebKitLifecycle::Ready(state);
                } else {
                    // Inspector failed — reconnect immediately
                    if let Some(new_state) = setup_webkit(&self.device_id).await {
                        if let Some(new_state) =
                            try_enrich(&mut raw, new_state, wv_x, wv_y).await
                        {
                            let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                            *wk = WebKitLifecycle::Ready(new_state);
                        } else {
                            let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                            *wk = WebKitLifecycle::Failed;
                        }
                    } else {
                        let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
                        *wk = WebKitLifecycle::Failed;
                    }
                }
            }
        }

        // Reconstruct wrapper with enriched tree for parse_hierarchy
        let original: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let mut response = serde_json::json!({ "tree": raw });
        if let Some(obj) = original.as_object() {
            for key in [
                "keyboard_height",
                "safe_area_top",
                "safe_area_bottom",
                "device_model",
            ] {
                if let Some(val) = obj.get(key) {
                    response[key] = val.clone();
                }
            }
        }
        let enriched_str =
            serde_json::to_string(&response).context("failed to serialize hierarchy")?;
        parse_hierarchy(&enriched_str)
    }

    async fn poke_for_system_alert(&self) -> Result<()> {
        // No-op XCUI query on the test app. iOS only invokes the
        // harness's UI-interruption-monitor when an XCUI query is
        // attempted against the app and a foreign dialog is blocking
        // it. Polling /hierarchy via WebKit Inspector doesn't count;
        // a real XCUI query does, without synthesising any input.
        self.client.post_json("/poke-interruption-monitor", "{}").await?;
        Ok(())
    }

    async fn tap(&self, x: i32, y: i32) -> Result<()> {
        let body = build_tap_body(x, y)?;
        self.client.post_json("/tap", &body).await?;
        Ok(())
    }

    async fn long_press(&self, x: i32, y: i32, duration_ms: u64) -> Result<()> {
        let body = build_long_press_body(x, y, duration_ms)?;
        self.client.post_json("/longpress", &body).await?;
        Ok(())
    }

    async fn type_text(&self, text: &str) -> Result<()> {
        let body = build_type_body(text)?;
        self.client.post_json("/type", &body).await?;
        Ok(())
    }

    async fn backspace(&self, count: u32) -> Result<()> {
        let body = build_backspace_body(count)?;
        self.client.post_json("/backspace", &body).await?;
        Ok(())
    }

    async fn swipe_coords(&self, from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> Result<()> {
        let body = build_swipe_body(from_x, from_y, to_x, to_y, 300)?;
        self.client.post_json("/swipe", &body).await?;
        Ok(())
    }

    async fn pinch(&self, x: i32, y: i32, scale: f64, velocity: f64) -> Result<()> {
        let body = serde_json::json!({ "x": x, "y": y, "scale": scale, "velocity": velocity }).to_string();
        self.client.post_json("/pinch", &body).await?;
        Ok(())
    }

    async fn gesture(&self, fingers: Vec<crate::GestureFinger>) -> Result<()> {
        let body = build_gesture_body(&fingers)?;
        self.client.post_json("/gesture", &body).await?;
        Ok(())
    }

    async fn screenshot(&self) -> Result<ScreenshotResult> {
        let data = self.client.get_bytes("/screenshot").await?;
        Ok(ScreenshotResult {
            path: String::new(),
            data,
        })
    }

    async fn hide_keyboard(&self) -> Result<()> {
        // Prefer blurring the focused DOM element via the WebKit Inspector:
        // it directly tells the WebView to relinquish focus, which iOS
        // honours by dismissing the soft keyboard. The companion's
        // coordinate-tap fallback is best-effort — a tap inside a Tauri
        // WebView lands on whatever HTML is at that point, which may be
        // the status bar inset (no effect) or another focusable element
        // (refocuses instead of blurring).
        if let Some(_) = self
            .eval_in_webview("(()=>{const a=document.activeElement;if(a&&a.blur)a.blur();return ''})()")
            .await
        {
            return Ok(());
        }
        self.client.post_json("/hide-keyboard", "{}").await?;
        Ok(())
    }

    async fn launch_app(&self, bundle_id: &str) -> Result<Option<String>> {
        let body = serde_json::json!({ "bundle_id": bundle_id }).to_string();
        let response = self.client.post_json("/launch", &body).await?;
        // Companion's /launch returns a `warning` field when the
        // staticTexts settle probe times out — the launch still
        // succeeded but the WebView's first paint may not be ready.
        // Surface as a DriverWarning substep upstream so when the
        // next step fails, the root cause isn't a mystery. Returns
        // ok so canvas-only apps (no static text on first paint)
        // can still proceed.
        let warning = serde_json::from_str::<serde_json::Value>(&response)
            .ok()
            .and_then(|json| {
                json.get("warning")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            });
        // Reset WebKit Inspector — the target app may have changed, or
        // the inspector session may be stale after an app switch.
        {
            let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
            *wk = WebKitLifecycle::Idle;
        }
        // Settle gate: companion's /launch returns when the OS has spawned
        // the app, not when the UI is interactive. Wait for the first
        // interactive frame so the next action doesn't race accessibility
        // tree population.
        self.await_first_frame().await?;
        Ok(warning)
    }

    async fn stop_app(&self, bundle_id: &str) -> Result<()> {
        // `simctl terminate` actually kills the process. The companion's
        // in-process `/stop` only suspends, which leaves the WKWebView
        // alive — repeated stop+launch cycles then stack new WebViews
        // on top of the suspended ones, and taps land on a frontmost
        // dead surface that never dispatches events to the live DOM.
        self.simctl(&["terminate", &self.device_id, bundle_id]).await?;
        // simctl returns when the kill signal is sent, not when the
        // OS has actually torn down the process + its WKWebView
        // resources. A `launch` that races that teardown sometimes
        // gets a half-initialised WebView. Small grace is cheap.
        tokio::time::sleep(IOS_POST_STOP_GRACE).await;
        // App process is gone — the old inspector connection is dead.
        let mut wk = self.webkit.lock().expect("webkit mutex poisoned");
        *wk = WebKitLifecycle::Idle;
        Ok(())
    }

    async fn clear_app_data(&self, bundle_id: &str) -> Result<()> {
        // Uninstall and reinstall is the standard way to clear data on iOS simulator
        self.simctl(&["uninstall", &self.device_id, bundle_id])
            .await?;
        Ok(())
    }

    async fn press_button(&self, button: &str) -> Result<()> {
        // `xcrun simctl ui <udid> home` was removed in Xcode 26 / iOS
        // 26.4 — `simctl ui` only takes `appearance`, `increase_contrast`
        // etc. now. Route through the companion's `XCUIDevice.shared
        // .press(.home)` instead, which is version-stable.
        let body = serde_json::json!({ "button": button }).to_string();
        self.client.post_json("/press", &body).await?;
        Ok(())
    }

    async fn set_dark_mode(&self, enabled: bool) -> Result<()> {
        let style = if enabled { "dark" } else { "light" };
        self.simctl(&["ui", &self.device_id, "appearance", style])
            .await?;
        Ok(())
    }

    async fn set_location(&self, lat: f64, lon: f64) -> Result<()> {
        self.simctl(&[
            "location",
            &self.device_id,
            "set",
            &format!("{lat},{lon}"),
        ])
        .await?;
        // The test app's `DeviceState.svelte` exposes a manual hook
        // (`window.__golemSetLocation`) rather than subscribing to
        // `navigator.geolocation` (which would need a permission grant
        // first). Poke the hook so the rendered "Location:" row reflects
        // the simctl coords. Best-effort: native screens / apps without
        // the hook quietly no-op.
        let _ = self
            .eval_in_webview(&format!(
                "window.__golemSetLocation && window.__golemSetLocation({lat}, {lon})"
            ))
            .await;
        Ok(())
    }

    async fn open_url(&self, url: &str) -> Result<()> {
        self.simctl(&["openurl", &self.device_id, url]).await?;
        Ok(())
    }

    async fn push_notification(
        &self,
        title: &str,
        body: &str,
        payload: Option<&str>,
    ) -> Result<()> {
        // Refuse on physical devices — `simctl push` is sim-only. Real
        // APNS delivery needs provisioning + Apple's push servers + a
        // device token, which is outside this action's scope. See
        // README §push_notification for the cross-device pattern using
        // `branch` + `http_*` to your own APNS backend.
        if self.physical {
            bail!(
                "push_notification is sim-only on iOS — `{}` is a \
                 physical device. Compose phys delivery via http_post \
                 to your APNS backend, gated by a branch on `_hardware`.",
                self.device_id,
            );
        }
        // Build an APNS payload JSON and push via simctl
        let apns_payload = if let Some(custom) = payload {
            format!(
                r#"{{"aps":{{"alert":{{"title":"{title}","body":"{body}"}}}},"custom":{custom}}}"#
            )
        } else {
            format!(r#"{{"aps":{{"alert":{{"title":"{title}","body":"{body}"}}}}}}"#)
        };

        // Write payload to a temp file, then push
        let tmp = std::env::temp_dir().join(format!("golem_push_{}.json", std::process::id()));
        tokio::fs::write(&tmp, &apns_payload)
            .await
            .context("writing push payload to temp file")?;

        let tmp_str = tmp
            .to_str()
            .context("temp path is not valid UTF-8")?;

        let result = self
            .simctl(&["push", &self.device_id, &self.bundle_id, tmp_str])
            .await;

        // Clean up temp file regardless of result
        let _ = tokio::fs::remove_file(&tmp).await;

        result?;

        // Mirror the `set_location` pattern: simctl push delivers an
        // APNS payload to the OS, but the test app's
        // `DeviceState.svelte` reads the rendered "Notification:" row
        // from a `window.__golemSetNotification` hook (Tauri's
        // notification plugin gets us tokens but doesn't surface
        // foreground deliveries to JS). Poke the hook so the asserted
        // text appears. Best-effort: native screens / apps without
        // the hook quietly no-op. Pass the body — that's what the
        // test asserts on with `right_of "Notification:"`.
        let body_escaped = body.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = self
            .eval_in_webview(&format!(
                r#"window.__golemSetNotification && window.__golemSetNotification("{body_escaped}")"#
            ))
            .await;
        Ok(())
    }

    async fn add_media(&self, path: &str) -> Result<()> {
        self.simctl(&["addmedia", &self.device_id, path]).await?;
        Ok(())
    }

    async fn grant_permission(&self, bundle_id: &str, permission: &str) -> Result<()> {
        let token = normalize_ios_permission(permission)?;
        self.simctl(&[
            "privacy",
            &self.device_id,
            "grant",
            token,
            bundle_id,
        ])
        .await?;
        // simctl returns as soon as the TCC change is written; iOS still
        // needs a moment to settle it across the privacy daemons. Without
        // this sleep an immediately-following `launch` races and the app
        // is killed mid-startup when the entitlement check resolves the
        // stale state. ~750ms is the smallest value that survived a
        // repeated-run flake hunt on iPhone 17 sim.
        tokio::time::sleep(std::time::Duration::from_millis(750)).await;
        Ok(())
    }

    async fn revoke_permission(&self, bundle_id: &str, permission: &str) -> Result<()> {
        let token = normalize_ios_permission(permission)?;
        self.simctl(&[
            "privacy",
            &self.device_id,
            "revoke",
            token,
            bundle_id,
        ])
        .await?;
        // Same TCC settle window as `grant_permission` — a `launch`
        // immediately after revoke would otherwise race the same way.
        tokio::time::sleep(std::time::Duration::from_millis(750)).await;
        Ok(())
    }

    async fn start_recording(&self, name: &str) -> Result<()> {
        if self.physical {
            anyhow::bail!(
                "iOS screen recording only works on simulators today \
                 — `simctl io recordVideo` is sim-only"
            );
        }
        let mut guard = self.recording.lock().await;
        if guard.is_some() {
            anyhow::bail!("start_recording called while a recording is already in progress");
        }
        // Reject shell-special chars in the name. Caller-side
        // `sanitize_filename` already strips these; belt-and-braces.
        if name.chars().any(|c| c == '\'' || c == '"' || c == '$' || c == '`' || c == '/') {
            anyhow::bail!("invalid recording name {name:?}");
        }
        let host_path = std::env::temp_dir()
            .join(format!("golem-rec-{}-{}.mp4", self.device_id, name))
            .to_string_lossy()
            .to_string();
        // Best-effort cleanup of any stale file at this path so simctl
        // doesn't refuse to overwrite.
        let _ = std::fs::remove_file(&host_path);
        let child = tokio::process::Command::new("xcrun")
            .args(["simctl", "io", &self.device_id, "recordVideo", &host_path])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning simctl recordVideo for {host_path}"))?;
        *guard = Some(IosRecordingState { host_path, child });
        Ok(())
    }

    async fn stop_recording(&self) -> Result<String> {
        let mut guard = self.recording.lock().await;
        let Some(mut state) = guard.take() else {
            // No active recording — `cleanup.rs` calls this as an
            // idempotent safety net, so absence is not an error.
            return Ok(String::new());
        };
        // simctl listens for SIGINT to flush the mp4 trailer cleanly.
        // `tokio::process::Child::kill` sends SIGKILL on Unix, which
        // truncates the file — so signal explicitly via `kill -INT`.
        if let Some(pid) = state.child.id() {
            let _ = tokio::process::Command::new("kill")
                .args(["-INT", &pid.to_string()])
                .status()
                .await;
        }
        // Wait for simctl to finish writing. `wait` reaps the child.
        let _ = state.child.wait().await;
        // simctl can take a moment to flush the moov atom after exit.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        if !std::path::Path::new(&state.host_path).exists() {
            anyhow::bail!("simctl recordVideo exited but {} is missing", state.host_path);
        }
        Ok(state.host_path)
    }

    async fn remove_port_forwards(&self) -> Result<()> {
        Ok(()) // Not applicable to iOS
    }
}


// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use golem_element::Bounds;

    // -----------------------------------------------------------------------
    // 1. Parse hierarchy JSON into Element tree
    // -----------------------------------------------------------------------
    #[test]
    fn parse_hierarchy_basic() {
        let json = r#"{
            "element_type": "View",
            "text": null,
            "id": "root",
            "placeholder": null,
            "enabled": true,
            "checked": false,
            "clickable": false,
            "focused": false,
            "bounds": { "x": 0, "y": 0, "width": 390, "height": 844 },
            "children": [
                {
                    "element_type": "Button",
                    "text": "Login",
                    "id": "login_btn",
                    "placeholder": null,
                    "enabled": true,
                    "checked": false,
                    "clickable": true,
                    "focused": false,
                    "bounds": { "x": 100, "y": 400, "width": 190, "height": 44 },
                    "children": []
                }
            ]
        }"#;

        let element = parse_hierarchy(json).expect("should parse");
        assert_eq!(element.0.element_type, "View");
        assert_eq!(element.0.accessibility_label.as_deref(), Some("root"));
        assert_eq!(element.0.children.len(), 1);

        let btn = &element.0.children[0];
        assert_eq!(btn.element_type, "Button");
        assert_eq!(btn.text.as_deref(), Some("Login"));
        assert_eq!(btn.accessibility_label.as_deref(), Some("login_btn"));
        assert!(btn.clickable);
        assert_eq!(btn.bounds, Bounds::new(100, 400, 190, 44));
    }

    // -----------------------------------------------------------------------
    // 2. Parse empty hierarchy (leaf element with no children)
    // -----------------------------------------------------------------------
    #[test]
    fn parse_hierarchy_empty_children() {
        let json = r#"{
            "element_type": "View",
            "text": null,
            "id": null,
            "placeholder": null,
            "enabled": true,
            "checked": false,
            "clickable": false,
            "focused": false,
            "bounds": { "x": 0, "y": 0, "width": 375, "height": 812 }
        }"#;

        let element = parse_hierarchy(json).expect("should parse");
        assert_eq!(element.0.element_type, "View");
        assert!(element.0.children.is_empty());
    }

    // -----------------------------------------------------------------------
    // 3. Parse hierarchy with nested children
    // -----------------------------------------------------------------------
    #[test]
    fn parse_hierarchy_deeply_nested() {
        let json = r#"{
            "element_type": "Window",
            "text": null,
            "id": null,
            "placeholder": null,
            "enabled": true,
            "checked": false,
            "clickable": false,
            "focused": false,
            "bounds": { "x": 0, "y": 0, "width": 390, "height": 844 },
            "children": [{
                "element_type": "View",
                "text": null,
                "id": "container",
                "placeholder": null,
                "enabled": true,
                "checked": false,
                "clickable": false,
                "focused": false,
                "bounds": { "x": 0, "y": 44, "width": 390, "height": 800 },
                "children": [{
                    "element_type": "Label",
                    "text": "Hello",
                    "id": null,
                    "placeholder": null,
                    "enabled": true,
                    "checked": false,
                    "clickable": false,
                    "focused": false,
                    "bounds": { "x": 20, "y": 100, "width": 350, "height": 24 },
                    "children": []
                }]
            }]
        }"#;

        let element = parse_hierarchy(json).expect("should parse");
        assert_eq!(element.0.element_type, "Window");
        assert_eq!(element.0.children.len(), 1);
        assert_eq!(element.0.children[0].element_type, "View");
        assert_eq!(element.0.children[0].children.len(), 1);
        assert_eq!(element.0.children[0].children[0].element_type, "Label");
        assert_eq!(
            element.0.children[0].children[0].text.as_deref(),
            Some("Hello")
        );
    }

    // -----------------------------------------------------------------------
    // 4. Parse screenshot response (bytes)
    // -----------------------------------------------------------------------
    #[test]
    fn screenshot_result_from_bytes() {
        let png_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let result = ScreenshotResult {
            path: String::new(),
            data: png_bytes.clone(),
        };
        assert_eq!(result.data, png_bytes);
        assert!(result.path.is_empty());
    }

    // -----------------------------------------------------------------------
    // 5. Tap request serialization
    // -----------------------------------------------------------------------
    #[test]
    fn tap_request_serialization() {
        let body = build_tap_body(150, 300).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["x"], 150);
        assert_eq!(parsed["y"], 300);
    }

    // -----------------------------------------------------------------------
    // 6. Type text request serialization
    // -----------------------------------------------------------------------
    #[test]
    fn type_text_request_serialization() {
        let body = build_type_body("hello world").expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["text"], "hello world");
    }

    #[test]
    fn type_text_request_with_special_chars() {
        let body = build_type_body("line1\nline2\ttab").expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["text"], "line1\nline2\ttab");
    }

    // -----------------------------------------------------------------------
    // 7. Swipe body serialization
    // -----------------------------------------------------------------------
    #[test]
    fn swipe_body_serialization() {
        let body = build_swipe_body(10, 20, 30, 40, 500).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["from_x"], 10);
        assert_eq!(parsed["from_y"], 20);
        assert_eq!(parsed["to_x"], 30);
        assert_eq!(parsed["to_y"], 40);
        assert_eq!(parsed["duration_ms"], 500);
    }

    // -----------------------------------------------------------------------
    // 8. Long press request serialization
    // -----------------------------------------------------------------------
    #[test]
    fn long_press_request_serialization() {
        let body = build_long_press_body(200, 400, 1500).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["x"], 200);
        assert_eq!(parsed["y"], 400);
        assert_eq!(parsed["duration_ms"], 1500);
    }

    // -----------------------------------------------------------------------
    // 9. IosDriver new() sets correct base URL
    // -----------------------------------------------------------------------
    #[test]
    fn ios_driver_new_sets_base_url() {
        let driver = IosDriver::new(
            "ABCD-1234".to_string(),
            "com.example.app".to_string(),
            8222,
            false,
        );
        assert_eq!(driver.base_url(), "http://localhost:8222");
        assert_eq!(driver.device_id(), "ABCD-1234");
        assert_eq!(driver.bundle_id(), "com.example.app");
    }

    #[test]
    fn ios_driver_new_custom_port() {
        let driver = IosDriver::new(
            "device-99".to_string(),
            "com.test.bundle".to_string(),
            9999,
            true,
        );
        assert_eq!(driver.base_url(), "http://localhost:9999");
    }

    // -----------------------------------------------------------------------
    // Additional: backspace request serialization
    // -----------------------------------------------------------------------
    #[test]
    fn backspace_request_serialization() {
        let body = build_backspace_body(5).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse");
        assert_eq!(parsed["count"], 5);
    }

    // -----------------------------------------------------------------------
    // normalize_ios_permission — shorthand → simctl token
    // -----------------------------------------------------------------------
    #[test]
    fn normalize_known_shorthands_match_simctl_tokens() {
        // simctl `privacy` accepts these exact strings as service tokens —
        // most coincide with the shorthand names by design.
        for (shorthand, expected) in [
            ("camera", "camera"),
            ("microphone", "microphone"),
            ("location", "location"),
            ("location-always", "location-always"),
            ("contacts", "contacts"),
            ("calendar", "calendar"),
            ("photos", "photos"),
        ] {
            assert_eq!(
                normalize_ios_permission(shorthand).expect("known shorthand"),
                expected,
                "iOS shorthand {shorthand:?} SHALL map to simctl token {expected:?}",
            );
        }
    }

    #[test]
    fn normalize_unknown_shorthand_errors_loudly() {
        let err = normalize_ios_permission("locaiton")
            .expect_err("unknown shorthand SHALL error");
        let msg = format!("{err}");
        assert!(msg.contains("locaiton"), "should echo the bad shorthand, got: {msg}");
        assert!(msg.contains("Known shorthands"), "should list known set, got: {msg}");
    }

    #[test]
    fn normalize_rejects_dropped_synonyms() {
        // location-when-in-use / photo-library were Android-only aliases;
        // they were dropped to keep the cross-platform vocabulary single-
        // canonical-name per intent. iOS rejects them too.
        assert!(normalize_ios_permission("location-when-in-use").is_err());
        assert!(normalize_ios_permission("photo-library").is_err());
    }

    #[test]
    fn normalize_rejects_notifications_with_accept_alert_hint() {
        let err = normalize_ios_permission("notifications")
            .expect_err("notifications SHALL not be a shorthand on iOS");
        let msg = format!("{err}");
        assert!(
            msg.contains("accept_alert"),
            "error should point at the cross-platform pattern, got: {msg}",
        );
    }

    // 1. Empty string is not a known shorthand and SHALL error (the
    //    match has no empty-string arm — it falls through to `other`).
    #[test]
    fn normalize_rejects_empty_string() {
        let err = normalize_ios_permission("")
            .expect_err("empty shorthand SHALL error");
        let msg = format!("{err}");
        assert!(
            msg.contains("Known shorthands"),
            "empty input SHALL still list the known set, got: {msg}",
        );
    }

    // 2. Matching is exact and case-sensitive — an uppercased token is
    //    NOT accepted (simctl tokens are lowercase by design).
    #[test]
    fn normalize_is_case_sensitive() {
        assert!(
            normalize_ios_permission("Camera").is_err(),
            "uppercased `Camera` SHALL NOT match the lowercase `camera` token",
        );
        assert!(
            normalize_ios_permission("CAMERA").is_err(),
            "all-caps `CAMERA` SHALL NOT match",
        );
    }

    // 3. No trimming — surrounding whitespace makes the token unknown
    //    rather than being silently normalized.
    #[test]
    fn normalize_does_not_trim_whitespace() {
        assert!(
            normalize_ios_permission(" camera").is_err(),
            "leading space SHALL NOT be trimmed away",
        );
        assert!(
            normalize_ios_permission("camera ").is_err(),
            "trailing space SHALL NOT be trimmed away",
        );
    }

    // 4. The unknown-shorthand error echoes the offending input with
    //    debug quoting (`{other:?}`), so a token containing a quote is
    //    rendered escaped — confirms the `:?` formatting path.
    #[test]
    fn normalize_unknown_error_uses_debug_quoting() {
        let err = normalize_ios_permission("we\"ird")
            .expect_err("unknown shorthand SHALL error");
        let msg = format!("{err}");
        assert!(
            msg.contains("we\\\"ird"),
            "debug-quoted input SHALL escape the embedded quote, got: {msg}",
        );
    }

    // 5. Post-stop grace SHALL be a real, bounded teardown window: nonzero
    //    (a zero grace would race the WKWebView release and reintroduce the
    //    half-initialised WebView bug the sleep guards against) yet short
    //    enough to keep stop+launch cycles cheap. Asserting the property
    //    rather than restating the literal keeps the invariant meaningful.
    #[test]
    fn ios_post_stop_grace_is_a_bounded_nonzero_window() {
        // 5a. Nonzero — the sleep must actually yield teardown time.
        assert!(
            IOS_POST_STOP_GRACE > std::time::Duration::ZERO,
            "post-stop grace SHALL be nonzero so simctl teardown can complete \
             before the next launch (got {IOS_POST_STOP_GRACE:?})",
        );
        // 5b. Bounded — a grace this large would make stop+launch cycles
        //     needlessly slow; the documented window is sub-second.
        assert!(
            IOS_POST_STOP_GRACE < std::time::Duration::from_secs(1),
            "post-stop grace SHALL stay under 1s to keep stop+launch cycles \
             cheap (got {IOS_POST_STOP_GRACE:?})",
        );
    }
}
