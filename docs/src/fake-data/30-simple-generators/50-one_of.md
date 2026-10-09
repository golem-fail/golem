### one_of

`${fake:one_of(free|pro|enterprise)}` → one value picked at random from the set.
The natural fit for radio buttons, dropdowns, and enum fields — and a building
block for anything golem doesn't generate directly (a plan tier, a gender, a
subset of countries). Choices are `|`-delimited (commas also work, since `|`
sidesteps the param separator): `fake:one_of(yes|no)`, `fake:one_of(JP, US, GB)`.
Seeded like every other generator.
