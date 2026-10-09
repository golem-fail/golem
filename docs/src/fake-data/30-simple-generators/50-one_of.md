### one_of

`${fake:one_of(free|pro|enterprise)}` → one value picked at random from the set.
Separate choices with `|` or `,`: `fake:one_of(yes|no)`, `fake:one_of(JP, US, GB)`.
Spaces around a choice are trimmed and empty choices are dropped. A choice
cannot contain `|`, `,` or `=`. Seeded like every other generator.
