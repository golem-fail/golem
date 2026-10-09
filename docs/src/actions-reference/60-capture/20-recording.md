### Screen recording — per-block via `record = true`

Recording is configured with `record`, not as a step action (see [Flow Options](test-structure.md#flow-options)). Highest priority wins: `--no-record` > `--record` > `[[block]] record` > `[flow.options] record` > `[options] record`. Output: `{output_dir}/{flow}/{device}/recordings/{block}_{iter}.mp4`.

```toml
[[block]]
name = "login"
record = true     # record this block only
steps = [ ... ]
```

iOS: simulator only. On a physical iPhone the block logs a warning and runs without a video.
