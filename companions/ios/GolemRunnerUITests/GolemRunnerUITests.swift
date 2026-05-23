import XCTest
import Foundation

final class GolemRunnerUITests: XCTestCase {
    static let defaultPort: UInt16 = 8222
    private var server: HTTPServer?

    /// Resolve the port to use:
    /// 1. Register with golem if GOLEM_REG_PORT is set
    /// 2. GOLEM_PORT env var
    /// 3. Default 8222
    private static func resolvePort() -> UInt16 {
        // Try registration first
        if let regPortStr = ProcessInfo.processInfo.environment["GOLEM_REG_PORT"],
           let regPort = UInt16(regPortStr) {
            if let port = registerWithGolem(regPort: regPort) {
                return port
            }
        }
        // Fallback to explicit port or default
        if let portStr = ProcessInfo.processInfo.environment["GOLEM_PORT"],
           let port = UInt16(portStr) {
            return port
        }
        return defaultPort
    }

    /// Register with golem's registration server to get a port allocation.
    private static func registerWithGolem(regPort: UInt16) -> UInt16? {
        let device = UIDevice.current
        // SIMULATOR_UDID is the simulator's actual identifier; fall
        // back to identifierForVendor on physical devices.
        let simulatorUdid = ProcessInfo.processInfo.environment["SIMULATOR_UDID"]
        let deviceId = simulatorUdid
            ?? device.identifierForVendor?.uuidString
            ?? "unknown"
        let body: [String: Any] = [
            "platform": "ios",
            "device_id": deviceId,
            "device_name": device.name,
            "version": "0.6.6"
        ]

        guard let jsonData = try? JSONSerialization.data(withJSONObject: body),
              let url = URL(string: "http://localhost:\(regPort)/register") else {
            return nil
        }

        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = jsonData
        request.timeoutInterval = 5

        let semaphore = DispatchSemaphore(value: 0)
        var resultPort: UInt16?

        let task = URLSession.shared.dataTask(with: request) { data, response, error in
            defer { semaphore.signal() }
            guard let data = data, error == nil,
                  let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let port = json["port"] as? Int else {
                return
            }
            resultPort = UInt16(port)
        }
        task.resume()
        _ = semaphore.wait(timeout: .now() + 5)

        return resultPort
    }

    override func setUp() {
        super.setUp()
        continueAfterFailure = true

        // UI interruption monitor for OS-owned system alerts (deep-link
        // "Open in <App>?" confirms, permission prompts, etc.). XCTest
        // fires this handler whenever a UI action against the test app
        // is blocked by an interrupting element. The handler runs in
        // the test process, so it can safely use XCUI APIs to tap the
        // dialog's positive button — no cross-app XCUI query required.
        //
        // This is the supported XCTest-native path. Our prior approach
        // — querying SpringBoard via `XCUIApplication(bundleIdentifier:)
        // .alerts.count` — terminated the harness in iOS 26 (XCTest's
        // "test done" lifecycle treats cross-app proxy attach as a
        // fatal step). The monitor is invoked only at iOS's discretion
        // and never directly queries cross-app state.
        //
        // The monitor only fires when an XCUI interaction is attempted
        // against the test app *while* the interruption is on screen.
        // Callers that have just triggered a system dialog (e.g. an
        // `open_link` immediately followed by an assert) should poke
        // a no-op XCUI query via `/poke-interruption-monitor` to give
        // XCTest a chance to invoke us.
        // Only auto-tap labels that are unambiguously system-prompt
        // confirmations. Generic affirmatives like "OK" or "Yes" also
        // appear in in-app dialogs (UIAlertController) the test wants
        // to interact with explicitly — auto-tapping those would
        // dismiss them before the test's own `tap` step had a chance,
        // which broke `alerts.test` after the monitor first landed.
        // For permission prompts that *only* expose "OK" or "Yes",
        // the test should use `accept_alert` explicitly.
        addUIInterruptionMonitor(withDescription: "golem-system-alert") { alert in
            for label in ["Open", "Allow", "Allow Once", "Allow While Using App"] {
                let btn = alert.buttons[label]
                if btn.exists {
                    btn.tap()
                    return true
                }
            }
            return false
        }

        let router = RequestRouter()

        // Try to bind, re-registering on port conflict (up to 3 attempts)
        var port = Self.resolvePort()
        let regPort: UInt16? = {
            if let s = ProcessInfo.processInfo.environment["GOLEM_REG_PORT"],
               let p = UInt16(s) { return p }
            return nil
        }()

        for attempt in 0..<3 {
            let httpServer = HTTPServer(port: port) { method, path, body in
                return router.handle(method: method, path: path, body: body)
            }
            do {
                try httpServer.start()
                server = httpServer
                return
            } catch {
                if attempt < 2, let rp = regPort,
                   let newPort = Self.registerWithGolem(regPort: rp) {
                    print("[golem] Port \(port) in use, re-registered on port \(newPort)")
                    port = newPort
                } else {
                    XCTFail("Failed to start HTTP server on port \(port): \(error)")
                    return
                }
            }
        }
    }

    override func tearDown() {
        server?.stop()
        super.tearDown()
    }

    func testCompanionServer() {
        let keepAlive = expectation(description: "Server runs until killed")
        keepAlive.isInverted = true
        wait(for: [keepAlive], timeout: .infinity)
    }
}
