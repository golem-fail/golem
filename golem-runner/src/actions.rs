mod app_lifecycle;
mod assertion;
mod capture;
mod device;
mod external;
pub(crate) mod interaction;
mod media;

#[cfg(test)]
mod test_helpers;

use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::{AppConfig, Step};
use golem_vars::VariableStore;

use crate::context::ExecutionContext;

use app_lifecycle::{handle_clear_data, handle_launch, handle_stop};
use assertion::{handle_assert_alert, handle_assert_not_visible, handle_assert_visible};
use capture::handle_read;
use device::{
    handle_dark_mode, handle_grant_permission, handle_press, handle_revoke_permission,
    handle_set_location,
};
use external::{
    handle_accept_alert, handle_await_email, handle_bash, handle_create_inbox,
    handle_dismiss_alert, handle_fail, handle_http, handle_load_fixture, handle_open_link,
    handle_push_notification, handle_run,
};
use interaction::{
    handle_backspace, handle_double_tap, handle_gesture, handle_hide_keyboard, handle_long_press,
    handle_pinch, handle_rotate_gesture, handle_scroll, handle_swipe, handle_tap, handle_type,
};
use media::{handle_add_media, handle_screenshot};

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
    // This match is the canonical list of action keywords. When you add, remove, or
    // rename an arm here, update docs/actions-reference.md to match — the
    // `actions_reference_doc_lists_every_action` test enforces that the two stay in sync.
    let action = step.action.as_str();
    match action {
        "tap" => handle_tap(step, driver, ctx).await,
        "double_tap" => handle_double_tap(step, driver, ctx).await,
        "type" => handle_type(step, driver, ctx).await,
        "backspace" => handle_backspace(step, driver, ctx).await,
        "long_press" => handle_long_press(step, driver, ctx).await,
        "swipe" => handle_swipe(step, driver, ctx).await,
        "pinch" => handle_pinch(step, driver).await,
        "gesture" => handle_gesture(step, driver).await,
        "scroll" => handle_scroll(step, driver, ctx).await,
        "read" => handle_read(step, driver, vars, ctx).await,
        "hide_keyboard" => handle_hide_keyboard(driver).await,
        "assert_visible" => handle_assert_visible(step, driver, ctx).await,
        "assert_not_visible" => handle_assert_not_visible(step, driver).await,
        "assert_alert" => handle_assert_alert(step, driver).await,
        "accept_alert" => handle_accept_alert(step, driver, ctx).await,
        "dismiss_alert" => handle_dismiss_alert(step, driver, ctx).await,
        "fail" => handle_fail(step),
        "launch" => handle_launch(step, driver, apps, ctx).await,
        "stop" => handle_stop(step, driver, apps, ctx).await,
        "clear_data" => handle_clear_data(step, driver, apps).await,
        "rotate" => handle_rotate_gesture(step, driver).await,
        "dark_mode" => handle_dark_mode(step, driver).await,
        "set_location" => handle_set_location(step, driver).await,
        "press" => handle_press(step, driver).await,
        "grant_permission" => handle_grant_permission(step, driver).await,
        "revoke_permission" => handle_revoke_permission(step, driver).await,
        "screenshot" => handle_screenshot(step, driver).await,
        "add_media" => handle_add_media(step, driver).await,
        "open_link" => handle_open_link(step, driver).await,
        "push_notification" => handle_push_notification(step, driver).await,
        "bash" => handle_bash(step, vars).await,
        "run" => handle_run(step, vars, ctx).await,
        "await_email" => handle_await_email(step, vars).await,
        "create_inbox" => handle_create_inbox(step, vars).await,
        "load_fixture" => handle_load_fixture(step, vars, ctx).await,
        "get_http" => handle_http(step, vars, "GET").await,
        "post_http" => handle_http(step, vars, "POST").await,
        "put_http" => handle_http(step, vars, "PUT").await,
        "patch_http" => handle_http(step, vars, "PATCH").await,
        "delete_http" => handle_http(step, vars, "DELETE").await,
        _ => crate::fail_code!(
            golem_events::FailureCode::ParseUnknownAction,
            "Unknown action: {}",
            action
        ),
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

    // ── docs/actions-reference.md stays in sync with the dispatch match ──
    //
    // Guards the *list* of action keywords, not their prose. Fails when an action is
    // added, removed, or renamed in the `match` above without the doc being updated to
    // match (or vice versa). It does NOT catch a wrong description or param — that still
    // needs human review; this only catches drift in which actions exist.
    #[test]
    fn actions_reference_doc_lists_every_action() {
        use std::collections::BTreeSet;

        // Lowercase `[a-z_]+` tokens that sit between occurrences of `delim` on a line.
        fn tokens(line: &str, delim: char) -> Vec<String> {
            line.split(delim)
                .enumerate()
                .filter(|(i, _)| i % 2 == 1) // odd segments are the bits between delimiters
                .map(|(_, s)| s.to_string())
                .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
                .collect()
        }

        // 1. Keywords from the dispatch match (between `match action {` and the `_ =>` arm).
        let code = include_str!("actions.rs");
        let mut in_code = BTreeSet::new();
        let mut in_match = false;
        for line in code.lines() {
            let t = line.trim_start();
            if t.starts_with("match action {") {
                in_match = true;
                continue;
            }
            if !in_match {
                continue;
            }
            if t.starts_with("_ =>") {
                break;
            }
            if line.contains("=>") {
                for tok in tokens(line, '"') {
                    in_code.insert(tok);
                }
            }
        }

        // 2. Keywords from doc action headers: `### `name` — ...` (every backticked token
        //    before the em dash; covers grouped headers like the *_http family).
        let doc = include_str!("../../docs/actions-reference.md");
        let mut in_doc = BTreeSet::new();
        for line in doc.lines() {
            let t = line.trim_start();
            if !t.starts_with("### `") {
                continue; // only action entries lead with a backticked keyword
            }
            let head = t.split(" — ").next().unwrap_or(t);
            for tok in tokens(head, '`') {
                in_doc.insert(tok);
            }
        }

        let code_only: Vec<_> = in_code.difference(&in_doc).collect();
        let doc_only: Vec<_> = in_doc.difference(&in_code).collect();
        assert!(
            code_only.is_empty() && doc_only.is_empty(),
            "actions.rs dispatch and docs/actions-reference.md are out of sync.\n  \
             in code but undocumented: {code_only:?}\n  \
             documented but not in code: {doc_only:?}",
        );
    }

    // ── unknown action returns error ──────────────────────────────

    #[tokio::test]
    async fn unknown_action_returns_error() {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));

        let step = make_step("fly_to_moon");

        let result = execute_action(&step, &driver, &mut vars, &ctx, &[]).await;
        let err = result.expect_err("should be error");
        let err_msg = format!("{err}");
        assert!(
            err_msg.contains("Unknown action"),
            "error should mention unknown action, got: {err_msg}"
        );
        assert!(
            err_msg.contains("fly_to_moon"),
            "error should mention the action name, got: {err_msg}"
        );
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseUnknownAction),
            "unknown action SHALL carry P400"
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

    // ── dispatch routing: each action string reaches its handler ──────
    //
    // These exercise the `match` in `execute_action` (the only logic in
    // this file). Each handler is identified by the param-validation
    // error it emits before any device/network I/O, so the assertions
    // confirm the action string routed to the intended handler without
    // requiring a live device.

    async fn dispatch(step: &Step) -> Result<()> {
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        let ctx = test_ctx(Path::new("."));
        execute_action(step, &driver, &mut vars, &ctx, &[]).await
    }

    // 3. `fail` routes to handle_fail and surfaces FlowExplicitFail.
    #[tokio::test]
    async fn fail_action_routes_to_handle_fail() {
        let err = dispatch(&make_step("fail"))
            .await
            .expect_err("fail action SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::FlowExplicitFail),
            "fail SHALL carry FlowExplicitFail",
        );
    }

    // 4. `bash` without a `run` param routes to handle_bash and reports the
    //    missing-param error before spawning a shell.
    #[tokio::test]
    async fn bash_action_routes_to_handle_bash_and_validates_run_param() {
        let err = dispatch(&make_step("bash"))
            .await
            .expect_err("bash without run SHALL error");
        let msg = format!("{err}");
        assert!(
            msg.contains("bash action requires 'run' param"),
            "bash SHALL route to handle_bash, got: {msg}",
        );
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseMissingParam),
            "missing bash param SHALL carry ParseMissingParam",
        );
    }

    // 5. `run` without a `script` param routes to handle_run.
    #[tokio::test]
    async fn run_action_routes_to_handle_run_and_validates_script_param() {
        let err = dispatch(&make_step("run"))
            .await
            .expect_err("run without script SHALL error");
        assert!(
            format!("{err}").contains("run action requires 'script' param"),
            "run SHALL route to handle_run",
        );
    }

    // 6. `await_email` without an `inbox` param routes to handle_await_email
    //    and fails before any IMAP connection.
    #[tokio::test]
    async fn await_email_action_routes_to_handle_await_email() {
        let err = dispatch(&make_step("await_email"))
            .await
            .expect_err("await_email without inbox SHALL error");
        assert!(
            format!("{err}").contains("await_email action requires 'inbox' param"),
            "await_email SHALL route to handle_await_email",
        );
    }

    // 7. `load_fixture` without a `fixture` param routes to handle_load_fixture.
    #[tokio::test]
    async fn load_fixture_action_routes_to_handle_load_fixture() {
        let err = dispatch(&make_step("load_fixture"))
            .await
            .expect_err("load_fixture without fixture SHALL error");
        assert!(
            format!("{err}").contains("load_fixture action requires 'fixture' param"),
            "load_fixture SHALL route to handle_load_fixture",
        );
    }

    // 8. `open_link` without a `url` param routes to handle_open_link.
    #[tokio::test]
    async fn open_link_action_routes_to_handle_open_link() {
        let err = dispatch(&make_step("open_link"))
            .await
            .expect_err("open_link without url SHALL error");
        assert!(
            format!("{err}").contains("open_link action requires 'url' param"),
            "open_link SHALL route to handle_open_link",
        );
    }

    // 9. Every HTTP verb alias is wired to handle_http: each of the five
    //    distinct dispatch match arms reaches the shared missing-`url` guard
    //    and surfaces handle_http's own error (keyed off `step.action`) with
    //    ParseMissingParam. Note: the per-arm `method` label ("GET"/"POST"/…)
    //    only becomes observable on the live-request path (the `HTTP {method}
    //    {url}` failure) or the unsupported-method `bail!`, neither reachable
    //    here without a network call or an invalid method — so this test proves
    //    routing + the missing-`url` error contract, not the method label.
    #[tokio::test]
    async fn http_verb_aliases_each_route_to_handle_http() {
        for action in [
            "get_http",
            "post_http",
            "put_http",
            "patch_http",
            "delete_http",
        ] {
            let result = dispatch(&make_step(action)).await;
            let err = result.expect_err("http verb without url SHALL error");
            let msg = format!("{err}");
            assert!(
                msg.contains(&format!("{action} action requires 'url' param")),
                "{action} SHALL route to handle_http and surface its own action label, got: {msg}",
            );
            assert_eq!(
                golem_events::extract_code(&err),
                Some(golem_events::FailureCode::ParseMissingParam),
                "{action} missing url SHALL carry ParseMissingParam",
            );
        }
    }
}
