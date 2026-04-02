import SwiftUI

struct ContentView: View {
    @State private var counter = 0
    @State private var status = "Ready"
    @State private var toggleOn = false

    var body: some View {
        VStack(spacing: 20) {
            Text("GOLEM Test B")
                .font(.largeTitle)
                .accessibilityIdentifier("app-b-title")

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
        }
        .padding()
    }
}
