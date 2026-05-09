import SwiftUI
import UserNotifications

/// Lives across the app's lifetime and holds the most recently
/// received notification body. ContentView observes it for the
/// `notification-display-b` element that push_notification.test
/// asserts against.
class NotificationStore: ObservableObject {
    @Published var latestBody: String = ""
}

/// UNUserNotificationCenterDelegate hooked to a `NotificationStore`
/// so foreground push deliveries (from `simctl push`) update the
/// SwiftUI tree and become observable by golem. `willPresent`
/// returns an empty option set so iOS doesn't render its own
/// banner over the test UI — the test asserts the body text
/// inside the app, not the system banner.
class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    static let store = NotificationStore()

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        let body = notification.request.content.body
        DispatchQueue.main.async {
            AppDelegate.store.latestBody = body
        }
        completionHandler([])
    }
}

@main
struct GolemTestBApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) var delegate

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(AppDelegate.store)
        }
    }
}
