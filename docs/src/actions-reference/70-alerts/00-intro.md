## Alerts
<!-- collapsed -->

Accept or dismiss a dialog from the app or the OS.

An OS prompt, such as a permission request, is in the tree on Android and on iOS. It covers the app: while it shows, the app's elements are not in the visible tree, so a step cannot target them and `assert_visible` on them fails. `tree(full = true)` still lists them, as hints. `assert_alert` sees the prompt, and `accept_alert` or `dismiss_alert` answers it.
