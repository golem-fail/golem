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
            "version": "0.5.7"
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
