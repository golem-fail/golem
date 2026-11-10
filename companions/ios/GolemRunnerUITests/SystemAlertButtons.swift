import Foundation

/// The pick among a SpringBoard alert's buttons, by the labels the host sends,
/// and the labels the UI-interruption monitor answers with on its own. Kept
/// free of XCUI so the logic tests can run it.
enum SystemAlertButtons {
    /// The UI-interruption monitor's affirmatives: it fires without the host,
    /// so it keeps its own list. Most-permissive-first. Exact XCUI labels as reported by
    /// /debug/probe on iOS 26 (e.g. "Allow Full Access" for the photo
    /// full-library prompt). Matched by label, not position: the photo
    /// prompt's affirmative is its middle button, so a last-button tap would
    /// hit "Don't Allow".
    static let accept = [
        "Allow Full Access", "Allow Access to All Photos",
        "Open", "Allow", "Allow Once", "Allow While Using App",
    ]

    /// What a tap on a system alert came to.
    enum Outcome: Equatable {
        /// No alert is up.
        case noAlert
        /// The alert is up, but its buttons have no labels yet (it is still
        /// animating in or out).
        case notReady
        /// Tap the button at `index` of the alert's buttons; `label` is how
        /// the alert spells it.
        case tap(index: Int, label: String)
        /// The alert is up and none of its buttons is one golem was asked for.
        case unmatched([String])
    }

    /// How two labels are compared, as on the host: case-insensitive, with
    /// iOS's typographic apostrophe read as a straight one, never a substring.
    static func normalize(_ label: String) -> String {
        return label.trimmingCharacters(in: .whitespaces)
            .replacingOccurrences(of: "\u{2019}", with: "'")
            .lowercased()
    }

    /// Pick the button with the first of `wanted` (the host's labels, in
    /// order) among the alert's `buttons` labels, else its only button when
    /// `onlyButton`. `nil` `buttons` means no alert is up.
    static func pick(_ wanted: [String], from buttons: [String]?, onlyButton: Bool) -> Outcome {
        guard let buttons else { return .noAlert }
        let labelled = buttons.enumerated().filter { !$0.element.isEmpty }
        if labelled.isEmpty { return .notReady }
        for label in wanted {
            let n = normalize(label)
            if let hit = labelled.first(where: { normalize($0.element) == n }) {
                return .tap(index: hit.offset, label: hit.element)
            }
        }
        if onlyButton, buttons.count == 1 {
            return .tap(index: 0, label: buttons[0])
        }
        return .unmatched(labelled.map { $0.element })
    }
}
