# Error Codes

A failed or warned step carries a code such as `EF404`: `<severity><domain><number>`.

- Severity: `E` for a failure, `W` for a warning (the step set `if_fail = "warn"`).
- Domain, the party that most likely owns the fix:
  - `F`: flow at run time; the test or the app is wrong.
  - `P`: the flow file, its fields or the suite config.
  - `A`: the app's build, install or launch.
  - `D`: the device, its boot or the companion.
  - `H`: the host: tools, ports, the golem daemon.
  - `X`: golem did not classify the error.
- Number: the cause, the same for `E` and `W`.

help("codes", "EF404") gives one code's meaning and fix.
