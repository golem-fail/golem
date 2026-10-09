### `browse_mcp_call` — Call a WebMCP tool

```toml
{ action = "browse_mcp_call", tool = "fulfil_order", arguments = { order_id = "${order_id}" }, save_to = "receipt" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `tool` | — | Tool name, as listed by the page. Required |
| `arguments` | `{}` | Inline table passed to the tool as JSON. Golem `${…}` variables resolve inside it |

A tool the page doesn't register fails with `F404`.

The `{ content: [{ type: "text", … }] }` envelope MCP tools return is unwrapped
— a flow gets the answer, not the scaffolding — and a result that is itself JSON
nests, so `${receipt.order_id}` works.

**Availability.** golem enables WebMCP automatically for flows that contain a
`browse_mcp_*` step. It needs an `https://` or `localhost` page. A browser too
old to support it fails with `H505`.
