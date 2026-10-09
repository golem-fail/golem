### `browse_execute_js` — Run JavaScript in the page

```toml
{ action = "browse_execute_js", script = "return document.title", save_to = "title" }
{ action = "browse_execute_js", file = "portal-helpers.js", script = "return fulfilOrder('${order_id}')" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `script` | — | Inline JavaScript. `${…}` variables are interpolated |
| `file` | — | `.js` file. Leading `/` = project root, else relative to the flow file. `..` is rejected. `${…}` is **not** interpolated |
| `save_to` | — | Save the returned value. Objects nest (`${result.total}`) |

Give `script`, `file`, or both. The file runs first, in the same evaluation, so the script sees what the file declared. The body runs inside an async function: use `return` to produce the value and `await` freely.
