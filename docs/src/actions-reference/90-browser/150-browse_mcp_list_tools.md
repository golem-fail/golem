### `browse_mcp_list_tools` — List the page's WebMCP tools

```toml
{ action = "browse_mcp_list_tools", save_to = "tools" }
```

A page that opts into [WebMCP](https://developer.chrome.com/docs/ai/webmcp)
describes what it can do — "fulfil an order", "issue a refund" — as named tools
with argument schemas. Saved as an object keyed by tool name, so
`${tools.fulfil_order}` is both a description and a presence check.
