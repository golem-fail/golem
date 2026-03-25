use anyhow::Result;
use golem_driver::PlatformDriver;
use golem_parser::Step;
use golem_vars::{ScopeLevel, VarValue, VariableStore};

use crate::resolution::resolve_element;

/// Find the target element, read its text content, and optionally save it
/// to a variable using `save_to`.
pub(crate) async fn handle_read(
    step: &Step,
    driver: &dyn PlatformDriver,
    vars: &mut VariableStore,
) -> Result<()> {
    let (elem, _coords) = resolve_element(step, driver).await?;

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
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "otp-code",
            "123456",
            Bounds::new(50.0, 300.0, 200.0, 30.0),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();

        let mut step = make_step("read");
        step.id = Some("otp-code".to_string());
        step.save_to = Some("otp".to_string());

        handle_read(&step, &driver, &mut vars)
            .await
            .expect("read should succeed");

        let saved = vars.get("otp").expect("otp variable should exist");
        assert_eq!(saved, &VarValue::string("123456"));
    }

    // ── Extra: read without save_to does not error ───────────────────

    #[tokio::test]
    async fn read_without_save_to_does_not_error() {
        let mut root = make_element("View", Bounds::new(0.0, 0.0, 375.0, 812.0));
        root.children.push(make_element_with_id_and_text(
            "Label",
            "info",
            "Some text",
            Bounds::new(10.0, 10.0, 100.0, 30.0),
        ));
        let driver = MockPlatformDriver::new(root);
        let mut vars = make_vars();

        let mut step = make_step("read");
        step.id = Some("info".to_string());
        // No save_to set

        handle_read(&step, &driver, &mut vars)
            .await
            .expect("read without save_to should succeed");
    }
}
