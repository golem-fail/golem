import Foundation
import Testing

struct SystemAlertButtonsTests {
    @Test func noAlertWhenNoButtonList() {
        #expect(SystemAlertButtons.pick(SystemAlertButtons.dismiss, from: nil) == .noAlert)
    }

    @Test func notReadyWhileButtonsHaveNoLabels() {
        #expect(SystemAlertButtons.pick(SystemAlertButtons.dismiss, from: []) == .notReady)
        #expect(SystemAlertButtons.pick(SystemAlertButtons.dismiss, from: ["", ""]) == .notReady)
    }

    @Test func dismissRefusesTheTrackingPrompt() {
        let att = ["Ask App Not to Track", "Allow"]
        #expect(SystemAlertButtons.pick(SystemAlertButtons.dismiss, from: att) == .tap("Ask App Not to Track"))
    }

    @Test func acceptAllowsTheTrackingPrompt() {
        let att = ["Ask App Not to Track", "Allow"]
        #expect(SystemAlertButtons.pick(SystemAlertButtons.accept, from: att) == .tap("Allow"))
    }

    @Test func dismissPicksTheRefusalWhereverItSits() {
        let photos = ["Limit Access…", "Allow Full Access", "Don’t Allow"]
        #expect(SystemAlertButtons.pick(SystemAlertButtons.dismiss, from: photos) == .tap("Don’t Allow"))
    }

    @Test func acceptPrefersFullPhotoAccess() {
        let photos = ["Limit Access…", "Allow Full Access", "Don’t Allow"]
        #expect(SystemAlertButtons.pick(SystemAlertButtons.accept, from: photos) == .tap("Allow Full Access"))
    }

    @Test func unknownButtonsAreListed() {
        let alert = ["Keep", "", "Discard"]
        #expect(SystemAlertButtons.pick(SystemAlertButtons.dismiss, from: alert) == .unmatched(["Keep", "Discard"]))
    }
}
