### `browse_execute_js` — Run JavaScript in the page

```toml
{ action = "browse_execute_js", script = "return document.title", save_to = "title" }
{ action = "browse_execute_js", file = "portal-helpers.js", script = "return fulfilOrder('${order_id}')" }
{ action = "browse_execute_js", script = "const r = await fetch('/api/orders'); return (await r.json()).length", save_to = "count" }
```

| Field | Default | Description |
|-------|---------|-------------|
| `script` | — | Inline JavaScript. Golem `${…}` variables are interpolated here |
| `file` | — | A `.js` file to run first. A leading `/` resolves from the project root; anything else from the flow file's directory. `..` is rejected |
| `save_to` | — | Save the result. Objects nest, so `${result.total}` works; anything else is stored as text |

Give either, or both. **The file runs first** — it's the natural home for
reusable functions, and the inline script is then the one-liner that calls one.
They run as a single evaluation, so the inline script sees whatever the file
declared.

The script body runs inside an async function: `return` what you want to save,
and `await` is available for anything the page has to fetch.

Golem variables are interpolated into `script` but **not** into `file`: a shared
helper shouldn't change meaning depending on which flow imported it. Both are
JavaScript, not TypeScript.
