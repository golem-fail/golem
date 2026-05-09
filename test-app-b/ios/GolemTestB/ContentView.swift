import SwiftUI

struct ContentView: View {
    @State private var counter = 0
    @State private var status = "Ready"
    @State private var toggleOn = false
    @EnvironmentObject var notifications: NotificationStore

    var body: some View {
        VStack(spacing: 20) {
            Text("GOLEM Test B")
                .font(.largeTitle)
                .accessibilityIdentifier("app-b-title")

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
