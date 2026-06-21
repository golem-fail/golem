import SwiftUI

struct ContentView: View {
    @State private var counter = 0
    @State private var status = "Ready"
    @State private var toggleOn = false
    @State private var occTapped = "none"
    @EnvironmentObject var notifications: NotificationStore

    var body: some View {
        VStack(spacing: 20) {
            Text("GOLEM Test B")
                .font(.largeTitle)
                .accessibilityIdentifier("app-b-title")

            // Native occlusion routing fixture (mirrors test-app-b Android). An
            // opaque overlay (drawn on top in the ZStack, tappable) covers the
            // centre of the button, leaving the edges clear. Unlike Android,
            // iOS does NOT prune the occluded button from the snapshot — it
            // stays at full bounds, so the host-side hit-test must route the
            // tap to a clear edge ("occ:button"), not the overlay ("occ:overlay").
            Text("occ:\(occTapped)")
            ZStack {
                // Label fills the frame so the button's accessibility element
                // is the full 240×80 (a bare SwiftUI Button hugs its text, ~89×20,
                // which the centre overlay would fully cover, leaving no clear
                // sample point to route to).
                Button(action: { occTapped = "button" }) {
                    Text("OCC Native")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                .frame(width: 240, height: 80)
                .accessibilityIdentifier("occ-button")
                Color.red.opacity(0.8)
                    .frame(width: 80, height: 80)
                    .onTapGesture { occTapped = "overlay" }
                    .accessibilityIdentifier("occ-overlay")
            }

            // Updated by AppDelegate's UNUserNotificationCenterDelegate
            // on every foreground push delivery. push_notification.test
            // asserts the body text shows up here.
            HStack {
                Text("Notification:")
                Text(notifications.latestBody)
                    .accessibilityIdentifier("notification-display-b")
            }

            Text(status)
                .accessibilityIdentifier("status-label")

            Text("Shared Data")
                .accessibilityIdentifier("shared-data-display")

            Button("Refresh") {
                status = "Refreshed"
            }
            .accessibilityIdentifier("refresh-button")

            Divider()

            // Elements for accessibility_id testing
            Text("\(counter)")
                .font(.title)
                .accessibilityIdentifier("counter-b")

            HStack(spacing: 16) {
                Button("Increment") {
                    counter += 1
                }
                .accessibilityIdentifier("increment-b")

                Button("Decrement") {
                    counter -= 1
                }
                .accessibilityIdentifier("decrement-b")
            }

            Toggle("Test Toggle", isOn: $toggleOn)
                .accessibilityIdentifier("toggle-b")
                .padding(.horizontal)

            Divider()

            Text("Native Scroll List")
                .font(.headline)
                .accessibilityIdentifier("native-list-title")

            // Native List in a fixed-height frame — items beyond 200pt are clipped
            List(0..<50, id: \.self) { index in
                Text("Native Item \(index)")
                    .accessibilityIdentifier("native-item-\(index)")
            }
            .frame(height: 200)
            .accessibilityIdentifier("native-list")
        }
        .padding()
    }
}
