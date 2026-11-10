import AppTrackingTransparency
import AVFoundation
import Photos
import SwiftUI
import UserNotifications

// The fixture image is tiny (7×11); real photos are large. Match the fixture
// by picking the first asset within this bound on both axes rather than
// hardcoding 7×11 in the app (the e2e asserts the exact size).
private let smallMax = 64

struct ContentView: View {
    @State private var counter = 0
    @State private var status = "Ready"
    @State private var toggleOn = false
    @State private var occTapped = "none"
    @State private var galleryDims = "pending"
    @State private var tracking = "pending"
    @State private var camera = "pending"
    @State private var notify = "pending"
    @State private var choice = "none"
    @State private var choosing = false
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

            HStack(spacing: 16) {
                Button("Refresh") {
                    status = "Refreshed"
                }
                .accessibilityIdentifier("refresh-button")

                // Three actions stack vertically, and iOS puts the cancel
                // action last: the alert e2e checks accept_alert does not
                // take the last button for the positive one.
                Button("Choose") { choosing = true }
                    .alert("Pick one", isPresented: $choosing) {
                        Button("OK") { choice = "ok" }
                        Button("Later") { choice = "later" }
                        Button("Cancel", role: .cancel) { choice = "cancel" }
                    }
                Text("Choice: \(choice)")
            }

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

            // Photo-library read → render dimensions. add_media pushes the
            // fixture image into the simulator's photo library (PhotoKit);
            // this section reads it back and renders its pixel size, the
            // observable result the add_media e2e asserts.
            Text("Dims: \(galleryDims)")
                .accessibilityIdentifier("dims-b")
                .frame(minHeight: 24)

            // Each permission request shows its OS prompt while the
            // permission is undecided, and renders the answer. The
            // permissions e2e set the state with `launch` `permissions`.
            HStack(spacing: 12) {
                Text("Camera: \(camera)").accessibilityIdentifier("camera-b")
                Text("Notifications: \(notify)").accessibilityIdentifier("notify-b")
                Text("Tracking: \(tracking)").accessibilityIdentifier("tracking-b")
            }
            .font(.caption)
            .frame(minHeight: 24)

            HStack(spacing: 24) {
                Button("Load") {
                    loadGallery()
                }
                .accessibilityIdentifier("load-gallery")
                // Meet the 24dp min touch-target (HIG minimum is 44); a bare
                // SwiftUI Button hugs its label (~20dp) and trips golem's a11y audit.
                .frame(minHeight: 44)

                Button("Camera") {
                    AVCaptureDevice.requestAccess(for: .video) { granted in
                        DispatchQueue.main.async { camera = granted ? "granted" : "denied" }
                    }
                }
                .accessibilityIdentifier("request-camera")
                .frame(minHeight: 44)

                Button("Notify") {
                    UNUserNotificationCenter.current().requestAuthorization(options: [.alert]) { granted, _ in
                        DispatchQueue.main.async { notify = granted ? "granted" : "denied" }
                    }
                }
                .accessibilityIdentifier("request-notifications")
                .frame(minHeight: 44)

                Button("Track") {
                    ATTrackingManager.requestTrackingAuthorization { status in
                        let result = status == .authorized ? "allowed" : "denied"
                        DispatchQueue.main.async { tracking = result }
                    }
                }
                .accessibilityIdentifier("request-tracking")
                .frame(minHeight: 44)
            }

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

    // Pre-granted permission (the e2e grants photos before launch) returns
    // .authorized here immediately — no prompt, no hang. .limited is treated
    // as granted since the fixture asset is still fetchable.
    private func loadGallery() {
        PHPhotoLibrary.requestAuthorization(for: .readWrite) { status in
            let result: String
            switch status {
            case .authorized, .limited:
                let assets = PHAsset.fetchAssets(with: .image, options: PHFetchOptions())
                var found: String?
                assets.enumerateObjects { asset, _, stop in
                    if asset.pixelWidth <= smallMax && asset.pixelHeight <= smallMax {
                        found = "\(asset.pixelWidth)x\(asset.pixelHeight)"
                        stop.pointee = true
                    }
                }
                result = found ?? "none"
            default:
                result = "denied"
            }
            DispatchQueue.main.async { galleryDims = result }
        }
    }
}
