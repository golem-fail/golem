import Foundation
import Testing

struct SystemAlertButtonsTests {
    private let refuse = ["Don't Allow", "Ask App Not to Track", "Cancel", "Not Now", "Deny", "No"]
    private let accept = ["Allow Full Access", "Allow While Using App", "Allow", "Allow Once", "OK"]

    @Test func noAlertWhenNoButtonList() {
        #expect(SystemAlertButtons.pick(refuse, from: nil, onlyButton: false) == .noAlert)
    }

    @Test func notReadyWhileButtonsHaveNoLabels() {
        #expect(SystemAlertButtons.pick(refuse, from: [], onlyButton: true) == .notReady)
        #expect(SystemAlertButtons.pick(refuse, from: ["", ""], onlyButton: true) == .notReady)
    }

    @Test func matchesIgnoringCaseAndApostropheStyle() {
        let photos = ["Limit Access…", "Allow Full Access", "Don’t Allow"]
        #expect(SystemAlertButtons.pick(["don't allow"], from: photos, onlyButton: false)
            == .tap(index: 2, label: "Don’t Allow"))
    }

    @Test func neverMatchesASubstring() {
        let ja = ["許可しない", "許可"]
        #expect(SystemAlertButtons.pick(["許可"], from: ja, onlyButton: false)
            == .tap(index: 1, label: "許可"))
    }

    @Test func theHostsOrderDecides() {
        let photos = ["Limit Access…", "Allow Full Access", "Don’t Allow"]
        #expect(SystemAlertButtons.pick(accept, from: photos, onlyButton: true)
            == .tap(index: 1, label: "Allow Full Access"))
        let location = ["Allow Once", "Allow While Using App", "Don’t Allow"]
        #expect(SystemAlertButtons.pick(accept, from: location, onlyButton: true)
            == .tap(index: 1, label: "Allow While Using App"))
    }

    @Test func refusesTheTrackingPrompt() {
        let att = ["Ask App Not to Track", "Allow"]
        #expect(SystemAlertButtons.pick(refuse, from: att, onlyButton: false)
            == .tap(index: 0, label: "Ask App Not to Track"))
    }

    @Test func theIndexSkipsNoUnlabelledButtons() {
        #expect(SystemAlertButtons.pick(["OK"], from: ["", "OK"], onlyButton: false)
            == .tap(index: 1, label: "OK"))
    }

    @Test func onlyButtonOnlyWhenAsked() {
        #expect(SystemAlertButtons.pick(accept, from: ["Got it"], onlyButton: true)
            == .tap(index: 0, label: "Got it"))
        #expect(SystemAlertButtons.pick(refuse, from: ["Got it"], onlyButton: false)
            == .unmatched(["Got it"]))
    }

    @Test func unknownButtonsAreListed() {
        #expect(SystemAlertButtons.pick(refuse, from: ["Keep", "", "Discard"], onlyButton: false)
            == .unmatched(["Keep", "Discard"]))
    }
}
