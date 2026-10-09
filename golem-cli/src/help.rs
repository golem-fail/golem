//! The MCP `help` tool: the docs parts in `docs/src`, by topic and item.
//!
//! A topic is one docs page. An item is a part of it, named by the last
//! part of its address (`tap`), its path (`interaction/tap`) or, for an
//! action, any name in its heading (`get_http`). A part with parts below
//! it answers with its own text and a list of those parts, not their text:
//! the LLM asks for the one it needs.

pub struct HelpPart {
    /// e.g. `actions-reference/interaction/tap`.
    pub address: &'static str,
    /// The page file, e.g. `actions-reference.md`.
    pub page: &'static str,
    /// The anchor of the part's first heading in its page.
    pub anchor: &'static str,
    /// The file `text` comes from, relative to `docs/`: the part, or its
    /// `.llm.md` file. Only `docs/mcp-context.md`'s test reads it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub source: &'static str,
    pub text: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/help_parts.rs"));

/// (topic, page, what it covers).
const TOPICS: &[(&str, &str, &str)] = &[
    (
        "act",
        "actions-reference",
        "the step notation and every action, in groups",
    ),
    (
        "selectors",
        "selectors",
        "how a step finds its element: text, label, anchors, traits",
    ),
    (
        "flow",
        "test-structure",
        "the .test.toml file: blocks, branches, steps, vars, data, teardown, devices",
    ),
    (
        "fake",
        "fake-data",
        "fake data such as ${fake:email}, person, address, credit_card",
    ),
    (
        "codes",
        "error-codes",
        "failure codes such as EF404: what each means and its fix",
    ),
];

/// The answer to `help(topic, item)`, or why there is none.
pub fn help(topic: Option<&str>, item: Option<&str>) -> Result<String, String> {
    let Some(topic) = topic else {
        return Ok(index());
    };
    let Some((topic, page, _)) = TOPICS
        .iter()
        .find(|(t, p, _)| norm(t) == norm(topic) || norm(p) == norm(topic))
    else {
        return Err(format!("no topic {topic:?}.\n{}", index()));
    };
    let Some(item) = item else {
        return Ok(render(page));
    };
    if *topic == "codes" {
        if let Some(code) = failure_code(item) {
            return Ok(code);
        }
    }
    resolve(topic, page, item).map(render)
}

fn index() -> String {
    let mut out = String::from(
        "help(topic) lists a topic's items; help(topic, item) gives one item.\n\nTopics:\n",
    );
    for (topic, _, about) in TOPICS {
        out.push_str(&format!("- {topic}: {about}\n"));
    }
    out
}

/// `get_http` and `get-http`, `Tap` and `tap`, are the same name.
fn norm(s: &str) -> String {
    s.trim().to_lowercase().replace('_', "-")
}

fn part(address: &str) -> Option<&'static HelpPart> {
    HELP_PARTS.iter().find(|p| p.address == address)
}

fn topic_of(address: &str) -> Option<&'static str> {
    let page = address.split('/').next()?;
    TOPICS
        .iter()
        .find(|(_, p, _)| *p == page)
        .map(|(t, _, _)| *t)
}

/// The parts directly below `address`.
fn children(address: &str) -> impl Iterator<Item = &'static HelpPart> + '_ {
    HELP_PARTS.iter().filter(move |p| {
        p.address
            .strip_prefix(address)
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|rest| !rest.contains('/'))
    })
}

fn last(address: &str) -> &str {
    address.rsplit('/').next().unwrap_or(address)
}

/// The names in an action heading: `get_http`, `post_http` … in
/// "### `get_http`, `post_http` — HTTP requests".
fn heading_names(text: &str) -> Vec<String> {
    let Some(heading) = text.lines().next().and_then(|l| l.strip_prefix("### `")) else {
        return Vec::new();
    };
    let names = heading.split(" — ").next().unwrap_or(heading);
    format!("`{names}")
        .split('`')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, n)| n.to_string())
        .collect()
}

fn resolve(topic: &str, page: &str, item: &str) -> Result<&'static str, String> {
    let want = norm(item);
    let under: Vec<&HelpPart> = HELP_PARTS
        .iter()
        .filter(|p| p.address.starts_with(&format!("{page}/")))
        .collect();
    let by = |f: &dyn Fn(&HelpPart) -> bool| -> Vec<&'static str> {
        under.iter().filter(|p| f(p)).map(|p| p.address).collect()
    };
    for found in [
        by(&|p| norm(&p.address[page.len() + 1..]) == want),
        by(&|p| norm(last(p.address)) == want),
        by(&|p| heading_names(p.text).iter().any(|n| norm(n) == want)),
    ] {
        match found.as_slice() {
            [] => continue,
            [one] => return Ok(one),
            several => {
                let names: Vec<String> = several
                    .iter()
                    .map(|a| a[page.len() + 1..].to_string())
                    .collect();
                return Err(format!(
                    "{item:?} matches several items in {topic}: {}",
                    names.join(", ")
                ));
            }
        }
    }
    Err(format!(
        "no item {item:?} in {topic}; help(\"{topic}\") lists them"
    ))
}

/// The name that `help(topic, item)` takes for `address`: its last part,
/// or its path when another part shares that last part.
fn item_name(address: &str) -> String {
    let page = address.split('/').next().unwrap_or_default();
    let rest = address.get(page.len() + 1..).unwrap_or_default();
    let shared = HELP_PARTS
        .iter()
        .filter(|p| p.address.starts_with(&format!("{page}/")) && last(p.address) == last(address))
        .count();
    if shared > 1 {
        rest.to_string()
    } else {
        last(address).to_string()
    }
}

fn render(address: &str) -> String {
    let Some(p) = part(address) else {
        return String::new();
    };
    let mut out = clean(p.text, p.page);
    let below: Vec<&HelpPart> = children(address).collect();
    if !below.is_empty() {
        let topic = topic_of(address).unwrap_or_default();
        out.push_str(&format!("\n\nItems (help(\"{topic}\", item)):\n"));
        for c in below {
            out.push_str(&format!("- {}: {}\n", item_name(c.address), summary(c)));
        }
    }
    out.trim_end().to_string() + "\n"
}

/// One line for a part in a list: its heading's summary, and for a part
/// with parts below it, their names.
fn summary(p: &HelpPart) -> String {
    let heading = p
        .text
        .lines()
        .next()
        .unwrap_or_default()
        .trim_start_matches('#')
        .trim();
    let about = match first_sentence(p.text) {
        Some(s)
            if s.len() <= 70 && p.address.contains('/') && children(p.address).next().is_some() =>
        {
            s
        }
        _ => heading
            .split_once(" — ")
            .map_or(heading, |(_, s)| s)
            .replace('`', ""),
    };
    let names: Vec<String> = children(p.address)
        .map(|c| {
            let names = heading_names(c.text);
            if names.is_empty() {
                last(c.address).to_string()
            } else {
                names.join(", ")
            }
        })
        .collect();
    if names.is_empty() {
        about
    } else {
        format!("{about} — {}", names.join(", "))
    }
}

/// The first sentence of a part's prose, after its heading.
fn first_sentence(text: &str) -> Option<String> {
    let line = text
        .lines()
        .skip(1)
        .map(str::trim)
        .find(|l| !l.is_empty())?;
    if line.starts_with(['|', '`', '#', '<', '-', '*', '{']) {
        return None;
    }
    let end = line.find(". ").map_or(line.len(), |i| i + 1);
    Some(line[..end].trim_end_matches('.').to_string())
}

/// A part as the LLM reads it: no comments or page navigation, and each
/// link to another part as the help call that fetches it.
fn clean(text: &str, page: &str) -> String {
    let mut no_comments = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        no_comments.push_str(&rest[..start]);
        rest = rest[start..]
            .find("-->")
            .map_or("", |end| &rest[start + end + 3..]);
    }
    no_comments.push_str(rest);

    let mut out = String::new();
    let mut fence = false;
    let mut blank = 0;
    for line in no_comments.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        let tagline = line.len() > 2
            && line.starts_with('*')
            && line.ends_with('*')
            && !line.starts_with("**");
        if !fence && (line.starts_with("← ") || tagline) {
            continue;
        }
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(&if fence {
            line.to_string()
        } else {
            rewrite_links(line, page)
        });
        out.push('\n');
    }
    out.trim().to_string()
}

/// `[label](target)` → `label (help("topic", "item"))` for a target that
/// is a part; `label (url)` for a web link; else `label`. Text in
/// backticks stays as it is.
fn rewrite_links(line: &str, page: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        let tick = rest.find('`');
        let open = rest.find('[');
        match (tick, open) {
            (Some(t), o) if o.is_none_or(|o| t < o) => {
                let close = rest[t + 1..].find('`').map_or(rest.len(), |c| t + 2 + c);
                out.push_str(&rest[..close]);
                rest = &rest[close..];
            }
            (_, Some(o)) => {
                let link = rest[o..].find("](").and_then(|m| {
                    let label = &rest[o + 1..o + m];
                    let after = &rest[o + m + 2..];
                    after
                        .find(')')
                        .map(|e| (label, &after[..e], o + m + 2 + e + 1))
                });
                match link {
                    Some((label, target, end)) if !label.contains('[') => {
                        out.push_str(&rest[..o]);
                        out.push_str(&link_text(label, target, page));
                        rest = &rest[end..];
                    }
                    _ => {
                        out.push_str(&rest[..=o]);
                        rest = &rest[o + 1..];
                    }
                }
            }
            _ => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

fn link_text(label: &str, target: &str, page: &str) -> String {
    if target.contains("://") {
        return format!("{label} ({target})");
    }
    let (file, anchor) = target.split_once('#').unwrap_or((target, ""));
    let file = if file.is_empty() { page } else { file };
    let found = HELP_PARTS.iter().find(|p| {
        p.page == file
            && if anchor.is_empty() {
                !p.address.contains('/')
            } else {
                p.anchor == anchor
            }
    });
    match found {
        Some(p) => {
            let topic = topic_of(p.address).unwrap_or_default();
            if p.address.contains('/') {
                format!("{label} (help(\"{topic}\", \"{}\"))", item_name(p.address))
            } else {
                format!("{label} (help(\"{topic}\"))")
            }
        }
        None => label.to_string(),
    }
}

/// `EF404`, `WF404` or `F404`: its meaning and fix.
fn failure_code(item: &str) -> Option<String> {
    let want = item.trim().to_uppercase();
    let fragment = match want.len() {
        5 if want.starts_with(['E', 'W']) => &want[1..],
        _ => want.as_str(),
    };
    golem_events::FailureCode::ALL
        .iter()
        .find(|c| c.fragment() == fragment)
        .map(|c| {
            format!(
                "E{fragment} (W{fragment} as a warning): {}\nFix: {}\n",
                c.meaning(),
                c.fix()
            )
        })
}

/// The Help section of `docs/mcp-context.md`: each help call, linked to
/// the text it answers with, and the answer's size.
#[cfg(test)]
pub fn context_section() -> String {
    let mut out = String::from(
        "## Help\n\nEach call that `help` answers, linked to its text: the section of the docs \
         page, or the `.llm.md` file that replaces it for the LLM. The size is the answer's, in \
         characters.\n\n```text\n",
    );
    out.push_str(&index());
    out.push_str("```\n");
    for (topic, page, _) in TOPICS {
        out.push_str(&format!("\n### {topic}\n\n"));
        for p in HELP_PARTS
            .iter()
            .filter(|p| p.address == *page || p.address.starts_with(&format!("{page}/")))
        {
            let depth = p.address.matches('/').count();
            let call = if depth == 0 {
                format!("help(\"{topic}\")")
            } else {
                format!("help(\"{topic}\", \"{}\")", item_name(p.address))
            };
            let heading = p
                .text
                .lines()
                .next()
                .unwrap_or_default()
                .trim_start_matches('#')
                .trim();
            let target = if p.source.ends_with(".llm.md") {
                format!("{} (LLM text)]({})", heading, p.source)
            } else {
                format!("{heading}]({}#{})", p.page, p.anchor)
            };
            out.push_str(&format!(
                "{}- `{call}` · [{target} · {}\n",
                "  ".repeat(depth.saturating_sub(1)),
                render(p.address).len()
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every answer stays small enough to read in one go.
    const BUDGET: usize = 7_000;

    #[test]
    fn every_part_answers_within_the_budget() {
        for p in HELP_PARTS {
            let topic = topic_of(p.address).expect("a topic for every page");
            let answer = match p.address.split_once('/') {
                Some(_) => help(Some(topic), Some(&item_name(p.address))),
                None => help(Some(topic), None),
            }
            .unwrap_or_else(|e| panic!("{}: {e}", p.address));
            assert_eq!(
                answer,
                render(p.address),
                "{} resolves to another part",
                p.address
            );
            assert!(
                answer.len() <= BUDGET,
                "{}: {} chars, over {BUDGET}",
                p.address,
                answer.len()
            );
        }
    }

    #[test]
    fn no_answer_keeps_a_comment_or_a_page_link() {
        for p in HELP_PARTS {
            let answer = render(p.address);
            assert!(!answer.contains("<!--"), "{}", p.address);
            let mut fence = false;
            for line in answer.lines() {
                if line.trim_start().starts_with("```") {
                    fence = !fence;
                }
                let prose: String = line.split('`').step_by(2).collect();
                assert!(fence || !prose.contains("]("), "{}: {line}", p.address);
            }
        }
    }

    #[test]
    fn the_index_lists_every_topic() {
        let index = help(None, None).expect("index");
        for (topic, _, _) in TOPICS {
            assert!(index.contains(&format!("- {topic}: ")), "{index}");
        }
    }

    #[test]
    fn an_item_is_found_by_its_name_its_path_or_an_action_name() {
        let tap = help(Some("act"), Some("tap")).expect("tap");
        assert!(tap.starts_with("### `tap`"), "{tap}");
        assert_eq!(help(Some("act"), Some("interaction/tap")), Ok(tap.clone()));
        assert_eq!(help(Some("act"), Some("TAP")), Ok(tap));
        let http = help(Some("act"), Some("post_http")).expect("post_http");
        assert!(http.contains("`get_http`"), "{http}");
        assert!(help(Some("act"), Some("explode")).is_err());
        assert!(help(Some("nope"), None).is_err());
    }

    #[test]
    fn every_action_has_an_item() {
        for action in golem_parser::validation::known_actions() {
            assert!(
                help(Some("act"), Some(action)).is_ok(),
                "no item for {action}"
            );
        }
    }

    #[test]
    fn a_group_lists_its_items_not_their_text() {
        let browser = help(Some("act"), Some("browser")).expect("browser");
        assert!(
            browser.contains("- browse_navigate: Load a URL"),
            "{browser}"
        );
        assert!(!browser.contains("### `browse_navigate`"), "{browser}");
        let act = help(Some("act"), None).expect("act");
        assert!(act.contains("- interaction: "), "{act}");
        assert!(act.contains("tap, double_tap"), "{act}");
    }

    #[test]
    fn a_link_to_a_part_becomes_the_help_call() {
        let line = rewrite_links(
            "see [Branching](test-structure.md#branching) and [`tap`](#tap--tap-an-element) or [x](https://a.b), not `[y](z.md)`",
            "actions-reference.md",
        );
        assert_eq!(
            line,
            "see Branching (help(\"flow\", \"branching\")) and `tap` (help(\"act\", \"tap\")) or x (https://a.b), not `[y](z.md)`"
        );
        assert_eq!(
            rewrite_links("[README](../README.md)", "selectors.md"),
            "README"
        );
    }

    #[test]
    fn a_failure_code_answers_with_its_meaning_and_fix() {
        let f404 = help(Some("codes"), Some("ef404")).expect("EF404");
        assert!(f404.starts_with("EF404 (WF404 as a warning): "), "{f404}");
        assert!(f404.contains("\nFix: "), "{f404}");
        assert_eq!(help(Some("codes"), Some("F404")), Ok(f404));
        assert!(help(Some("codes"), Some("registry")).is_ok());
    }
}
