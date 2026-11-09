import Foundation

/// The labels golem answers a SpringBoard alert with, and the pick among an
/// alert's buttons. Kept free of XCUI so the logic tests can run it.
enum SystemAlertButtons {
    /// Affirmatives, most-permissive-first. Exact XCUI labels as reported by
    /// /debug/probe on iOS 26 (e.g. "Allow Full Access" for the photo
    /// full-library prompt). Matched by label, not position: the photo
    /// prompt's affirmative is its middle button, so a last-button tap would
    /// hit "Don't Allow".
    static let accept = [
        "Allow Full Access", "Allow Access to All Photos",
        "Open", "Allow", "Allow Once", "Allow While Using App",
    ]

    /// Refusals. "Ask App Not to Track" is the App Tracking Transparency
    /// prompt's.
    static let dismiss = [
        "Don’t Allow", "Don't Allow", "Ask App Not to Track",
        "Cancel", "Not Now", "Deny", "No",
    ]

    /// What a tap on a system alert came to.
    enum Outcome: Equatable {
        /// No alert is up.
        case noAlert
        /// The alert is up, but its buttons have no labels yet (it is still
        /// animating in or out).
        case notReady
        /// Tap the button with this label.
        case tap(String)
        /// The alert is up and none of its buttons is one golem knows.
        case unmatched([String])
    }

    /// Pick the first of `wanted` among the alert's `buttons` labels. `nil`
    /// `buttons` means no alert is up.
    static func pick(_ wanted: [String], from buttons: [String]?) -> Outcome {
        guard let buttons else { return .noAlert }
        let labels = buttons.filter { !$0.isEmpty }
        if labels.isEmpty { return .notReady }
        if let hit = wanted.first(where: labels.contains) { return .tap(hit) }
        return .unmatched(labels)
    }
}
