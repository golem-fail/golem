//! Per-step `${…}` interpolation.
//!
//! Resolves variable references and inline `${fake:…}` generators in every
//! string-bearing field of a step, returning an interpolated clone. The
//! executor runs this once per step, immediately before dispatch, so
//! selectors, inputs, and external params (`url`/`body`/…) all act on
//! resolved values. Without it, `${var}` / `${fake:…}` would reach the
//! drivers verbatim.

use anyhow::Result;
use golem_parser::{Anchor, SelectorGroup, Step};
use golem_vars::interpolation::{interpolate, InterpolationContext};

/// Return an interpolated clone of `step`, resolving `${…}` in all of its
/// string fields against `ctx`. A bad reference (undefined var, object in a
/// string, malformed generator) surfaces as a coded `ParseVariable` error.
pub(crate) fn interpolate_step(step: &Step, ctx: &InterpolationContext) -> Result<Step> {
    let mut s = step.clone();

    for f in [
        &mut s.on_text,
        &mut s.on_accessibility_label,
        &mut s.on_below,
        &mut s.on_above,
        &mut s.on_right_of,
        &mut s.on_left_of,
        &mut s.input,
        &mut s.app,
    ] {
        interp_opt(f, ctx)?;
    }

    for group in [&mut s.on, &mut s.within, &mut s.start, &mut s.end]
        .into_iter()
        .flatten()
    {
        interp_group(group, ctx)?;
    }
    for group in &mut s.points {
        interp_group(group, ctx)?;
    }
    for finger in &mut s.fingers {
        for group in &mut finger.points {
            interp_group(group, ctx)?;
        }
    }

    for value in s.params.values_mut() {
        interp_toml(value, ctx)?;
    }

    Ok(s)
}

/// Interpolate a string field in place. Coded `ParseVariable` on failure.
fn interp_opt(field: &mut Option<String>, ctx: &InterpolationContext) -> Result<()> {
    if let Some(text) = field {
        if let Some(resolved) = interp_str(text, ctx)? {
            *text = resolved;
        }
    }
    Ok(())
}

/// Interpolate a `&str`, returning `Some(resolved)` only when it actually
/// contained a `$` (fast-path skips the common literal case). Errors are
/// re-coded as `ParseVariable` so they route to the test-author domain.
fn interp_str(text: &str, ctx: &InterpolationContext) -> Result<Option<String>> {
    if !text.contains('$') {
        return Ok(None);
    }
    let resolved = interpolate(text, ctx).map_err(|e| {
        golem_events::coded(
            golem_events::FailureCode::ParseVariable,
            anyhow::anyhow!("interpolating \"{text}\": {e}"),
        )
    })?;
    Ok(Some(resolved))
}

fn interp_group(group: &mut SelectorGroup, ctx: &InterpolationContext) -> Result<()> {
    interp_opt(&mut group.text, ctx)?;
    interp_opt(&mut group.accessibility_label, ctx)?;
    for anchor in [
        &mut group.below,
        &mut group.above,
        &mut group.right_of,
        &mut group.left_of,
    ]
    .into_iter()
    .flatten()
    {
        interp_anchor(anchor, ctx)?;
    }
    Ok(())
}

fn interp_anchor(anchor: &mut Anchor, ctx: &InterpolationContext) -> Result<()> {
    match anchor {
        Anchor::Text(text) => {
            if let Some(resolved) = interp_str(text, ctx)? {
                *text = resolved;
            }
        }
        Anchor::Selector(group) => interp_group(group, ctx)?,
    }
    Ok(())
}

fn interp_toml(value: &mut toml::Value, ctx: &InterpolationContext) -> Result<()> {
    match value {
        toml::Value::String(s) => {
            if let Some(resolved) = interp_str(s, ctx)? {
                *s = resolved;
            }
        }
        toml::Value::Array(items) => {
            for item in items {
                interp_toml(item, ctx)?;
            }
        }
        toml::Value::Table(table) => {
            for (_k, v) in table.iter_mut() {
                interp_toml(v, ctx)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_parser::Step;
    use golem_vars::{Scope, ScopeLevel, VarValue, VariableStore};

    fn store_with(pairs: &[(&str, &str)]) -> VariableStore {
        let mut store = VariableStore::new();
        let mut scope = Scope::new(ScopeLevel::Flow);
        for (k, v) in pairs {
            scope.set(*k, VarValue::string(*v));
        }
        store.push_scope(scope);
        store
    }

    #[test]
    fn interpolates_input_and_selector() {
        let store = store_with(&[("user", "alice"), ("field", "Email")]);
        let ctx = InterpolationContext::new(&store);
        let step = Step {
            action: "type".into(),
            on_text: Some("${field}".into()),
            input: Some("${user}@x.com".into()),
            ..Default::default()
        };
        let out = interpolate_step(&step, &ctx).expect("interpolation SHALL succeed");
        assert_eq!(out.on_text.as_deref(), Some("Email"));
        assert_eq!(out.input.as_deref(), Some("alice@x.com"));
    }

    #[test]
    fn interpolates_params_string() {
        let store = store_with(&[("host", "example.com")]);
        let ctx = InterpolationContext::new(&store);
        let mut step = Step {
            action: "http_get".into(),
            ..Default::default()
        };
        step.params.insert(
            "url".into(),
            toml::Value::String("https://${host}/x".into()),
        );
        let out = interpolate_step(&step, &ctx).expect("ok");
        assert_eq!(
            out.params.get("url").and_then(|v| v.as_str()),
            Some("https://example.com/x"),
            "params strings SHALL be interpolated"
        );
    }

    #[test]
    fn interpolates_positional_anchor() {
        let store = store_with(&[("section", "Account")]);
        let ctx = InterpolationContext::new(&store);
        let step = Step {
            action: "tap".into(),
            on_below: Some("${section}".into()),
            ..Default::default()
        };
        let out = interpolate_step(&step, &ctx).expect("ok");
        assert_eq!(out.on_below.as_deref(), Some("Account"));
    }

    #[test]
    fn undefined_var_errors_with_parse_variable_code() {
        let store = VariableStore::new();
        let ctx = InterpolationContext::new(&store);
        let step = Step {
            action: "type".into(),
            input: Some("${missing}".into()),
            ..Default::default()
        };
        let err = interpolate_step(&step, &ctx).expect_err("undefined var SHALL error");
        assert_eq!(
            golem_events::extract_code(&err),
            Some(golem_events::FailureCode::ParseVariable),
            "interpolation errors SHALL carry the ParseVariable code"
        );
    }

    #[test]
    fn literal_without_dollar_is_untouched() {
        let store = VariableStore::new();
        let ctx = InterpolationContext::new(&store);
        let step = Step {
            action: "type".into(),
            input: Some("plain text".into()),
            on_text: Some("Submit".into()),
            ..Default::default()
        };
        let out = interpolate_step(&step, &ctx).expect("ok");
        assert_eq!(out.input.as_deref(), Some("plain text"));
        assert_eq!(out.on_text.as_deref(), Some("Submit"));
    }
}
