//! `docs/mcp-context.md`: everything an MCP client receives from
//! `golem mcp`, on one page, for golem developers.

use crate::mcp::{GolemMcp, INSTRUCTIONS};

/// The command that writes the page.
pub const UPDATE: &str = "GOLEM_UPDATE_DOCS=1 cargo nextest run -p golem-cli mcp_context";

/// The page as `docs/mcp-context.md` holds it.
pub fn page() -> String {
    let tools = GolemMcp::served_tools();
    let list = serde_json::to_string(&tools).unwrap_or_default();
    let mut out = format!(
        "<!-- Generated from golem-cli (mcp.rs, help.rs) and docs/src. Run `{UPDATE}`. -->\n\
         # MCP context\n\n\
         What an MCP client receives from `golem mcp`, on one page: the instructions, the tool \
         list, and each answer of `help`. For golem developers; [MCP server](mcp.md) is the \
         user guide.\n\n\
         | Text | Characters |\n|------|-----------:|\n\
         | Instructions | {} |\n| Tool list ({} tools, as `tools/list` JSON) | {} |\n\
         | Both, kept in context by most clients | {} |\n\n\
         ## Instructions\n\n```text\n{INSTRUCTIONS}\n```\n\n## Tools\n",
        INSTRUCTIONS.len(),
        tools.len(),
        list.len(),
        INSTRUCTIONS.len() + list.len()
    );
    for tool in &tools {
        out.push_str(&format!(
            "\n### `{}`\n\n{}\n",
            tool.name,
            tool.description.as_deref().unwrap_or_default()
        ));
        let schema = serde_json::Value::Object((*tool.input_schema).clone());
        let props = schema["properties"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        if props.is_empty() {
            continue;
        }
        let required: Vec<&str> = schema["required"]
            .as_array()
            .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        out.push_str("\n| Parameter | Type | Description |\n|-----------|------|-------------|\n");
        for (name, p) in props {
            let kind = match &p["type"] {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(a) => a
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" or "),
                _ => "object".to_string(),
            };
            out.push_str(&format!(
                "| `{name}`{} | {kind} | {} |\n",
                if required.contains(&name.as_str()) {
                    " (required)"
                } else {
                    ""
                },
                p["description"]
                    .as_str()
                    .unwrap_or_default()
                    .replace('|', "\\|")
                    .replace('\n', " ")
            ));
        }
    }
    out.push('\n');
    out.push_str(&crate::help::context_section());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path;

    #[test]
    fn mcp_context_page_matches_the_server() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/mcp-context.md");
        let page = page();
        if std::env::var_os("GOLEM_UPDATE_DOCS").is_some() {
            std::fs::write(&path, &page).expect("write docs/mcp-context.md");
        }
        assert!(
            std::fs::read_to_string(&path).ok().as_deref() == Some(page.as_str()),
            "docs/mcp-context.md differs from what the server sends. Run: {UPDATE}"
        );
    }

    #[test]
    fn mcp_context_links_resolve() {
        let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs");
        let pages = BTreeMap::from([("mcp-context.md".to_string(), page())]);
        let broken = golem_docs::broken_links(&docs, &pages);
        assert!(broken.is_empty(), "broken links:\n{}", broken.join("\n"));
    }
}
