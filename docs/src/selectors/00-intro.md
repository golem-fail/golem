# Selectors

*How golem finds the element a step acts on.*

← [Back to README](../README.md) · See also [Test Structure](test-structure.md) · [Actions Reference](actions-reference.md) · [Architecture: visibility model](architecture.md#visibility-model--the-visible-tree-decides-coverage-the-full-tree-only-hints)

A selector describes *which* on-screen element a step targets. golem resolves it
against the **visible tree** — the elements a human can actually see (clipped to
ancestor containers).
The same selector grammar is used everywhere an element is named: `tap`,
`assert_visible`, `read`, `scroll`'s `to`/`within`, swipe points, etc.

<!-- toc depth=2 -->
