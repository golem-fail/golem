### `browse_mcp_call` — Call a WebMCP tool

```toml
{ action = "browse_mcp_call", tool = "fulfil_order", arguments = { order_id = "${order_id}" }, save_to = "receipt" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `tool` | — | Tool name, as listed by the page. Required |
| `arguments` | `{}` | Inline table passed to the tool as JSON. Golem `${…}` variables resolve inside it |

Driving a page's declared tools beats clicking through its UI where they exist:
the page states its own contract, so the flow isn't coupled to a layout that may
be redesigned next quarter. A tool the page doesn't register fails with `F404`.

The `{ content: [{ type: "text", … }] }` envelope MCP tools return is unwrapped
— a flow gets the answer, not the scaffolding — and a result that is itself JSON
nests, so `${receipt.order_id}` works.

**Availability.** WebMCP ships switched off. golem turns it on automatically for
flows containing a `browse_mcp_*` step (it launches the browser, so nothing
needs toggling in `chrome://flags`), and leaves it off otherwise, since an
experimental browser feature changes what every page can feature-detect. It also
needs a **secure origin**: an `https://` or `localhost` page. A browser too old
to support it fails with `H505`.
