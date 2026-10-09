### Screen recording — per-block via `record = true`

Recording is configured at the project, flow, or block level — not as a
step action. Cascade (highest priority wins): `--no-record` >
`--record` > `[[block]] record` > `[flow.options] record` >
`[options] record`. Output: `{output_dir}/{flow}/{device}/recordings/{block}_{iter}.mp4`.

```toml
[[block]]
name = "login"
record = true     # record this block only
steps = [ ... ]
```

**Simulator-only on iOS** — `simctl io recordVideo` has no physical-device equivalent, so a recording request on a real iPhone fails. It degrades rather than breaking the run: the block records a warning and its steps execute normally, just without a video. Android records on physical devices and emulators alike. Tracked in [#60](https://github.com/golem-fail/golem/issues/60).
