# App Install

*Putting the clay on the device before the word reaches it.*

← [Back to README](../README.md) · See also [CLI Reference](cli-reference.md) · [Test Structure](test-structure.md)

By default golem assumes the app under test is already installed on the target device. For fresh simulators, CI pipelines, or teams that want per-test builds, you can supply an install script that golem runs before each flow.

golem runs the script with four arguments: `$1` platform, `$2` device id, `$3` bundle id and `$4` mode (see [Script contract](#script-contract)). `golem install-script` writes one from a template. Without a terminal, `golem install-script --print <framework>` writes a template to stdout.

<!-- toc -->
