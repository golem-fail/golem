use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::Step;
use golem_vars::{ScopeLevel, VarValue, VariableStore};

use crate::context::ExecutionContext;
use crate::resolution::resolve_element;

/// Find the target element, read its text content, and optionally save it
/// to a variable using `save_to`.
pub(crate) async fn handle_read(
    step: &Step,
    driver: &dyn PlatformDriver,
    vars: &mut VariableStore,
    ctx: &ExecutionContext<'_>,
) -> Result<()> {
    let (elem, _coords) = resolve_element(step, driver, ctx.emitter).await?;

    let text = elem.text.unwrap_or_default();

    if let Some(ref var_name) = step.save_to {
        vars.set_in_scope(ScopeLevel::Flow, var_name, VarValue::string(&text));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::test_helpers::*;
    use golem_driver::MockPlatformDriver;
    use golem_element::Bounds;

    // ── 2. read action captures text into variable ───────────────────

    #[tokio::test]
    async fn read_action_captures_text_into_variable() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "otp-code",
            "123456",
            Bounds::new(50, 300, 200, 30),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();

        let mut step = make_step("read");
        step.on_accessibility_label = Some("otp-code".to_string());
        step.save_to = Some("otp".to_string());

        let ctx = crate::context::test_ctx(std::path::Path::new("."));
        handle_read(&step, &driver, &mut vars, &ctx)
            .await
            .expect("read should succeed");

        let saved = vars.get("otp").expect("otp variable should exist");
        assert_eq!(saved, &VarValue::string("123456"));
    }

    // ── Extra: read without save_to does not error ───────────────────

    #[tokio::test]
    async fn read_without_save_to_does_not_error() {
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "info",
            "Some text",
            Bounds::new(10, 10, 100, 30),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();

        let mut step = make_step("read");
        step.on_accessibility_label = Some("info".to_string());
        // No save_to set

        let ctx = crate::context::test_ctx(std::path::Path::new("."));
        handle_read(&step, &driver, &mut vars, &ctx)
            .await
            .expect("read without save_to should succeed");
    }

    // ── 3. textless element captures the empty string into the var ───

    #[tokio::test]
    async fn read_action_captures_empty_string_for_textless_element() {
        // An element with no `text` resolves via `unwrap_or_default()` to
        // the empty string, and that empty value SHALL be saved verbatim.
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_id(
            "Label",
            "blank-field",
            Bounds::new(50, 300, 200, 30),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();

        let mut step = make_step("read");
        step.on_accessibility_label = Some("blank-field".to_string());
        step.save_to = Some("captured".to_string());

        let ctx = crate::context::test_ctx(std::path::Path::new("."));
        handle_read(&step, &driver, &mut vars, &ctx)
            .await
            .expect("read of a textless element SHALL succeed");

        let saved = vars.get("captured").expect("captured variable SHALL exist");
        assert_eq!(
            saved,
            &VarValue::string(""),
            "textless element SHALL save the empty string"
        );
    }

    // ── 4. captured text lands in the Flow scope specifically ────────

    #[tokio::test]
    async fn read_action_saves_into_flow_scope() {
        // `handle_read` writes with `ScopeLevel::Flow`. With BOTH a Flow and a
        // Project scope present, the captured var SHALL land in the Flow scope
        // ONLY — proving the write is scope-targeted, not just "somewhere".
        let mut root = make_element("View", Bounds::new(0, 0, 375, 812));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "token",
            "abc",
            Bounds::new(50, 300, 200, 30),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();
        // 1. Add a second, distinct scope so the Flow-vs-other distinction is
        //    load-bearing: a stray write would be detectable in the Project scope.
        vars.push_scope(golem_vars::Scope::new(ScopeLevel::Project));

        let mut step = make_step("read");
        step.on_accessibility_label = Some("token".to_string());
        step.save_to = Some("tok".to_string());

        let ctx = crate::context::test_ctx(std::path::Path::new("."));
        handle_read(&step, &driver, &mut vars, &ctx)
            .await
            .expect("read SHALL succeed");

        // 2. The var SHALL be present in the Flow scope with the captured text.
        let from_flow = vars
            .scopes()
            .iter()
            .find(|s| s.level == ScopeLevel::Flow)
            .and_then(|s| s.get("tok"))
            .expect("captured var SHALL be in Flow scope");
        assert_eq!(
            from_flow,
            &VarValue::string("abc"),
            "captured text SHALL be readable from the Flow scope"
        );

        // 3. The var SHALL NOT have leaked into the Project scope.
        let in_other = vars
            .scopes()
            .iter()
            .find(|s| s.level == ScopeLevel::Project)
            .and_then(|s| s.get("tok"));
        assert!(
            in_other.is_none(),
            "captured var SHALL NOT be written to any non-Flow scope"
        );
    }

    // ── 5. unresolvable selector surfaces an error ───────────────────

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn read_action_errors_when_element_not_found() {
        // When the selector matches nothing, `resolve_element` polls until
        // its deadline and then returns Err, which `handle_read` SHALL
        // propagate without writing any variable.
        let root = make_element("View", Bounds::new(0, 0, 375, 812));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();

        let mut step = make_step("read");
        step.on_accessibility_label = Some("missing".to_string());
        step.save_to = Some("never".to_string());

        let ctx = crate::context::test_ctx(std::path::Path::new("."));
        let result = handle_read(&step, &driver, &mut vars, &ctx).await;

        assert!(
            result.is_err(),
            "read SHALL error when the target element cannot be resolved"
        );
        assert!(
            vars.get("never").is_none(),
            "no variable SHALL be written when resolution fails"
        );
    }
}
