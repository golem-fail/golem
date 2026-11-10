//! Which button of an alert answers it. In order: the step's `buttons`, then
//! (Android) the button's resource id, then the built-in labels, then for
//! `accept_alert` an alert's only button. Nothing else: a button picked by
//! its position is a guess, and a wrong guess passes the step and leaves the
//! flow in a state nobody asked for.

use golem_element::Element;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Accept,
    Dismiss,
}

impl Answer {
    pub(crate) fn action(self) -> &'static str {
        match self {
            Answer::Accept => "accept_alert",
            Answer::Dismiss => "dismiss_alert",
        }
    }
}

/// Which rule picked the button. Reported in verbose output only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rule {
    Buttons,
    Id,
    BuiltIn,
    OnlyButton,
}

impl Rule {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Rule::Buttons => "buttons",
            Rule::Id => "id",
            Rule::BuiltIn => "built-in label",
            Rule::OnlyButton => "only button",
        }
    }
}

/// Labels that accept, most permissive first: the first a button matches
/// wins, so "Allow Full Access" beats "Allow", and a location prompt grants
/// "while using" before "once" on both platforms.
const ACCEPT_LABELS: &[&str] = &[
    "Allow Full Access",
    "Allow Access to All Photos",
    "Allow While Using App",
    "While using the app",
    "Allow",
    "Allow Once",
    "Only this time",
    "Open",
    "OK",
    "Yes",
    "Continue",
    "Confirm",
    "Done",
];

/// Labels that refuse, the OS prompts' first.
const DISMISS_LABELS: &[&str] = &[
    "Don't Allow",
    "Ask App Not to Track",
    "Cancel",
    "Not Now",
    "Deny",
    "No",
    "Close",
    "Dismiss",
];

/// Android buttons by resource id, in the same priority: the permission
/// prompt's, then a framework dialog's positive or negative. An id does not
/// change with the device language.
const ACCEPT_IDS: &[&str] = &[
    "permission_allow_button",
    "permission_allow_foreground_only_button",
    "permission_allow_one_time_button",
    "android:id/button1",
];
const DISMISS_IDS: &[&str] = &[
    "permission_deny_button",
    "permission_deny_and_dont_ask_again_button",
    "android:id/button2",
];

/// How two labels are compared: case-insensitive, with the typographic
/// apostrophe iOS uses ("Don’t Allow") read as a straight one. Never a
/// substring match: in Japanese 許可 is "Allow" and 許可しない "Don't Allow".
pub(crate) fn normalize(label: &str) -> String {
    label.trim().replace('’', "'").to_lowercase()
}

/// What a button says: its text, else its accessibility label.
pub(crate) fn label_of(button: &Element) -> &str {
    [&button.text, &button.accessibility_label]
        .into_iter()
        .flatten()
        .map(|s| s.as_str())
        .find(|s| !s.trim().is_empty())
        .unwrap_or("")
}

fn built_in(answer: Answer) -> &'static [&'static str] {
    match answer {
        Answer::Accept => ACCEPT_LABELS,
        Answer::Dismiss => DISMISS_LABELS,
    }
}

/// The labels to try, in order: the step's `buttons`, then the built-in
/// ones. iOS sends them to the companion, which matches SpringBoard alerts.
pub(crate) fn labels(answer: Answer, step_buttons: &[String]) -> Vec<String> {
    step_buttons
        .iter()
        .cloned()
        .chain(built_in(answer).iter().map(|s| s.to_string()))
        .collect()
}

/// The rule that a SpringBoard alert's tapped `label` came from.
pub(crate) fn rule_of_label(label: &str, step_buttons: &[String], answer: Answer) -> Rule {
    let n = normalize(label);
    if step_buttons.iter().any(|b| normalize(b) == n) {
        Rule::Buttons
    } else if built_in(answer).iter().any(|b| normalize(b) == n) {
        Rule::BuiltIn
    } else {
        Rule::OnlyButton
    }
}

fn by_label<'a>(buttons: &'a [Element], wanted: &[String]) -> Option<&'a Element> {
    wanted.iter().find_map(|w| {
        let w = normalize(w);
        buttons.iter().find(|b| normalize(label_of(b)) == w)
    })
}

/// The button of an in-tree alert that gives `answer`, and the rule that
/// picked it. `None` when no rule matches: the step fails.
pub(crate) fn pick<'a>(
    buttons: &'a [Element],
    answer: Answer,
    step_buttons: &[String],
) -> Option<(&'a Element, Rule)> {
    if let Some(b) = by_label(buttons, step_buttons) {
        return Some((b, Rule::Buttons));
    }
    let ids = match answer {
        Answer::Accept => ACCEPT_IDS,
        Answer::Dismiss => DISMISS_IDS,
    };
    if let Some(b) = ids.iter().find_map(|id| {
        buttons
            .iter()
            .find(|b| b.accessibility_id.as_deref() == Some(*id))
    }) {
        return Some((b, Rule::Id));
    }
    let built: Vec<String> = built_in(answer).iter().map(|s| s.to_string()).collect();
    if let Some(b) = by_label(buttons, &built) {
        return Some((b, Rule::BuiltIn));
    }
    match (answer, buttons) {
        (Answer::Accept, [only]) => Some((only, Rule::OnlyButton)),
        _ => None,
    }
}

/// The failure when no rule picks a button: the alert's buttons, and the
/// step with `buttons` to paste. One label goes in the example, since the
/// first match wins.
pub(crate) fn no_match_error(answer: Answer, labels: &[String]) -> anyhow::Error {
    let quoted = |l: &str| format!("\"{}\"", l.replace('\\', "\\\\").replace('"', "\\\""));
    let list = labels
        .iter()
        .map(|l| quoted(l))
        .collect::<Vec<_>>()
        .join(", ");
    let example = labels.first().map(|l| quoted(l)).unwrap_or_default();
    golem_events::coded(
        golem_events::FailureCode::FlowAlertInteraction,
        anyhow::anyhow!(
            "{action} failed: no button of the alert is one golem knows. Its buttons: {list}. \
             Name the one to tap: {{ action = \"{action}\", buttons = [{example}] }}",
            action = answer.action(),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use golem_element::Bounds;

    fn button(label: &str) -> Element {
        crate::actions::test_helpers::make_element_with_text(
            "Button",
            label,
            Bounds::new(0, 0, 10, 10),
        )
    }

    fn id_button(id: &str, label: &str) -> Element {
        let mut b = button(label);
        b.accessibility_id = Some(id.to_string());
        b
    }

    fn picked(buttons: &[Element], answer: Answer, step: &[&str]) -> Option<(String, Rule)> {
        let step: Vec<String> = step.iter().map(|s| s.to_string()).collect();
        pick(buttons, answer, &step).map(|(b, r)| (label_of(b).to_string(), r))
    }

    #[test]
    fn no_label_both_accepts_and_refuses() {
        for a in ACCEPT_LABELS {
            assert!(
                !DISMISS_LABELS.iter().any(|d| normalize(d) == normalize(a)),
                "{a:?} SHALL not be in both tables"
            );
        }
    }

    #[test]
    fn matching_ignores_case_and_apostrophe_style_but_not_substrings() {
        let b = [button("Don’t Allow")];
        assert_eq!(
            picked(&b, Answer::Dismiss, &["don't allow"]).map(|p| p.1),
            Some(Rule::Buttons)
        );
        let ja = [button("許可しない"), button("許可")];
        assert_eq!(
            picked(&ja, Answer::Accept, &["許可"]).map(|p| p.0),
            Some("許可".to_string()),
            "許可 SHALL not match 許可しない"
        );
    }

    #[test]
    fn the_steps_buttons_come_first() {
        let b = [button("OK"), button("Later"), button("Cancel")];
        assert_eq!(
            picked(&b, Answer::Accept, &["Later"]),
            Some(("Later".to_string(), Rule::Buttons))
        );
    }

    #[test]
    fn ids_come_before_built_in_labels() {
        let b = [
            id_button("android:id/button3", "Plus tard"),
            id_button("android:id/button2", "Annuler"),
            id_button("android:id/button1", "Oui"),
        ];
        assert_eq!(
            picked(&b, Answer::Dismiss, &[]),
            Some(("Annuler".to_string(), Rule::Id))
        );
        assert_eq!(
            picked(&b, Answer::Accept, &[]),
            Some(("Oui".to_string(), Rule::Id))
        );
    }

    #[test]
    fn a_stacked_alert_is_accepted_by_label_not_by_its_last_button() {
        // iOS stacks 3+ actions and puts Cancel last.
        let b = [button("OK"), button("Later"), button("Cancel")];
        assert_eq!(
            picked(&b, Answer::Accept, &[]),
            Some(("OK".to_string(), Rule::BuiltIn))
        );
        assert_eq!(
            picked(&b, Answer::Dismiss, &[]),
            Some(("Cancel".to_string(), Rule::BuiltIn))
        );
    }

    #[test]
    fn built_in_order_decides_between_two_matches() {
        let photos = [
            button("Limit Access…"),
            button("Allow Full Access"),
            button("Don’t Allow"),
        ];
        assert_eq!(
            picked(&photos, Answer::Accept, &[]).map(|p| p.0),
            Some("Allow Full Access".to_string())
        );
        let location = [
            button("Allow Once"),
            button("Allow While Using App"),
            button("Don’t Allow"),
        ];
        assert_eq!(
            picked(&location, Answer::Accept, &[]).map(|p| p.0),
            Some("Allow While Using App".to_string())
        );
    }

    #[test]
    fn only_accept_takes_an_only_button() {
        let b = [button("Got it")];
        assert_eq!(
            picked(&b, Answer::Accept, &[]),
            Some(("Got it".to_string(), Rule::OnlyButton))
        );
        assert_eq!(picked(&b, Answer::Dismiss, &[]), None);
    }

    #[test]
    fn unknown_buttons_pick_nothing() {
        let b = [button("Keep"), button("Discard")];
        assert_eq!(picked(&b, Answer::Accept, &[]), None);
        assert_eq!(picked(&b, Answer::Dismiss, &[]), None);
    }

    #[test]
    fn the_error_lists_the_buttons_and_a_step_to_paste() {
        let err = no_match_error(Answer::Dismiss, &["Keep".into(), "Say \"no\"".into()]);
        let msg = format!("{err:#}");
        assert!(
            msg.contains(r#"Its buttons: "Keep", "Say \"no\"""#),
            "{msg}"
        );
        assert!(
            msg.contains(r#"{ action = "dismiss_alert", buttons = ["Keep"] }"#),
            "{msg}"
        );
    }

    #[test]
    fn labels_sent_to_the_companion_put_the_steps_first() {
        let l = labels(Answer::Dismiss, &["Plus tard".into()]);
        assert_eq!(l[0], "Plus tard");
        assert_eq!(l[1], "Don't Allow");
    }
}
