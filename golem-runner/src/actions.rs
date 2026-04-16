mod app_lifecycle;
mod assertion;
mod capture;
mod device;
mod external;
pub(crate) mod interaction;
mod media;
mod wait;

#[cfg(test)]
mod test_helpers;

use anyhow::{bail, Result};
use golem_driver::PlatformDriver;
use golem_parser::{AppConfig, Step};
use golem_vars::VariableStore;

use crate::context::ExecutionContext;

use app_lifecycle::{handle_clear_data, handle_launch, handle_stop};
use assertion::{handle_assert_alert, handle_assert_not_visible, handle_assert_visible};
use capture::handle_read;
use device::{
    handle_dark_mode, handle_grant_permission, handle_press, handle_revoke_permission,
    handle_rotate, handle_set_location,
};
use external::{
    handle_accept_alert, handle_await_email, handle_bash, handle_dismiss_alert, handle_fail,
    handle_http, handle_load_fixture, handle_open_link, handle_push_notification, handle_run,
};
use interaction::{
    handle_backspace, handle_double_tap, handle_hide_keyboard, handle_long_press, handle_scroll,
    handle_swipe, handle_tap, handle_type,
};
use media::{handle_add_media, handle_screenshot, handle_start_recording, handle_stop_recording};
use wait::{handle_wait, handle_wait_not};

/// Resolve an element using all step selectors except `text`.
///
/// Dispatch a step to the appropriate action handler.
pub async fn execute_action(
    step: &Step,
    driver: &dyn PlatformDriver,
    vars: &mut VariableStore,
    ctx: &ExecutionContext<'_>,
    apps: &[AppConfig],
) -> Result<()> {
    let action = step.action.as_str();
    match action {
        "tap" => handle_tap(step, driver).await,
        "doubleTap" => handle_double_tap(step, driver).await,
        "type" => handle_type(step, driver).await,
        "backspace" => handle_backspace(step, driver).await,
        "long_press" => handle_long_press(step, driver).await,
        "swipe" => handle_swipe(step, driver).await,
        "scroll" => handle_scroll(step, driver).await,
        "read" => handle_read(step, driver, vars).await,
        "hide_keyboard" => handle_hide_keyboard(driver).await,
        "assert_visible" | "assert_text" | "assert_enabled" | "assert_checked" =>
            handle_assert_visible(step, driver).await,
        "assert_not_visible" => handle_assert_not_visible(step, driver).await,
        "assert_alert" => handle_assert_alert(step, driver).await,
        "accept_alert" => handle_accept_alert(step, driver).await,
        "dismiss_alert" => handle_dismiss_alert(step, driver).await,
        "wait" => handle_wait(step, driver).await,
        "wait_not" => handle_wait_not(step, driver).await,
        "fail" => handle_fail(step),
        "launch" => handle_launch(step, driver, apps, ctx).await,
        "stop" => handle_stop(step, driver, apps).await,
        "clear_data" => handle_clear_data(step, driver, apps).await,
        "rotate" => handle_rotate(step, driver).await,
        "dark_mode" => handle_dark_mode(step, driver).await,
        "set_location" => handle_set_location(step, driver).await,
        "press" => handle_press(step, driver).await,
        "grant_permission" => handle_grant_permission(step, driver).await,
        "revoke_permission" => handle_revoke_permission(step, driver).await,
        "screenshot" => handle_screenshot(step, driver).await,
        "start_recording" => handle_start_recording(step, driver).await,
        "stop_recording" => handle_stop_recording(step, driver).await,
        "add_media" => handle_add_media(step, driver).await,
        "open_link" => handle_open_link(step, driver).await,
        "push_notification" => handle_push_notification(step, driver).await,
        "bash" => handle_bash(step, vars).await,
        "run" => handle_run(step, vars, ctx).await,
        "await_email" => handle_await_email(step, vars).await,
        "load_fixture" => handle_load_fixture(step, vars, ctx).await,
        "http_get" => handle_http(step, vars, "GET").await,
        "http_post" => handle_http(step, vars, "POST").await,
        "http_put" => handle_http(step, vars, "PUT").await,
        "http_patch" => handle_http(step, vars, "PATCH").await,
        "http_delete" => handle_http(step, vars, "DELETE").await,
        _ => bail!("Unknown action: {}", action),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_ctx;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;
    use std::path::Path;
    use test_helpers::*;

    // ── unknown action returns error ──────────────────────────────

    #[tokio::test]
    async fn unknown_action_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("fly_to_moon");

        let result = execute_action(&step, &driver, &mut vars, &ctx, &[]).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Unknown action"),
            "error should mention unknown action, got: {err_msg}"
        );
        assert!(
            err_msg.contains("fly_to_moon"),
            "error should mention the action name, got: {err_msg}"
        );
    }

    // ── unknown action still returns error ───────────────────────────

    #[tokio::test]
    async fn unknown_action_still_returns_error_after_new_actions() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("teleport");

        let result = execute_action(&step, &driver, &mut vars, &ctx, &[]).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.expect_err("should be error"));
        assert!(
            err_msg.contains("Unknown action"),
            "error should mention unknown action, got: {err_msg}"
        );
        assert!(
            err_msg.contains("teleport"),
            "error should mention the action name, got: {err_msg}"
        );
    }
}
