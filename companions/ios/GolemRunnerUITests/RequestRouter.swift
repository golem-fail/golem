import XCTest

/// Routes HTTP requests to XCUITest actions.
final class RequestRouter {

    /// Device model identifier (e.g. "iPhone17,3").
    /// On physical devices, reads from uname(). On simulators, reads from
    /// SIMULATOR_MODEL_IDENTIFIER environment variable.
    private static let deviceModel: String = {
        if let simModel = ProcessInfo.processInfo.environment["SIMULATOR_MODEL_IDENTIFIER"] {
            return simModel
        }
        var systemInfo = utsname()
        uname(&systemInfo)
        return withUnsafePointer(to: &systemInfo.machine) {
            $0.withMemoryRebound(to: CChar.self, capacity: 1) {
                String(validatingUTF8: $0) ?? "unknown"
            }
        }
    }()

    /// Handle an incoming HTTP request and return an HTTPResponse.
    func handle(method: String, path: String, body: Data?) -> HTTPResponse {
        // Parse path and query string.
        let components = path.split(separator: "?", maxSplits: 1)
        let route = String(components[0])
        let query = components.count > 1 ? parseQuery(String(components[1])) : [:]

        switch (method, route) {
        case ("GET", "/health"):
            return handleHealth()
        case ("GET", "/hierarchy"):
            return handleHierarchy(query: query)
        case ("POST", "/tap"):
            return handleTap(body: body, query: query)
        case ("POST", "/longpress"):
            return handleLongPress(body: body, query: query)
        case ("POST", "/type"):
            return handleType(body: body, query: query)
        case ("POST", "/backspace"):
            return handleBackspace(body: body, query: query)
        case ("POST", "/swipe"):
            return handleSwipe(body: body, query: query)
        case ("POST", "/pinch"):
            return handlePinch(body: body, query: query)
        case ("POST", "/gesture"):
            return handleGesture(body: body)
        case ("GET", "/screenshot"):
            return handleScreenshot()
        case ("POST", "/hide-keyboard"):
            return handleHideKeyboard(query: query)
        case ("POST", "/launch"):
            return handleLaunch(body: body, query: query)
        case ("POST", "/stop"):
            return handleStop(body: body, query: query)
        default:
            return .error("Not found: \(method) \(route)", status: 404)
        }
    }

    // MARK: - Query parsing

    private func parseQuery(_ query: String) -> [String: String] {
        var result: [String: String] = [:]
        for pair in query.split(separator: "&") {
            let kv = pair.split(separator: "=", maxSplits: 1)
            if kv.count == 2 {
                let key = String(kv[0]).removingPercentEncoding ?? String(kv[0])
                let value = String(kv[1]).removingPercentEncoding ?? String(kv[1])
                result[key] = value
            }
        }
        return result
    }

    /// Parse JSON body into a dictionary.
    private func parseBody(_ body: Data?) -> [String: Any]? {
        guard let data = body else { return nil }
        return try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    }

    /// Get an XCUIApplication, optionally targeting a specific bundle ID.
    private func app(query: [String: String]) -> XCUIApplication {
        if let bundleId = query["bundle_id"], !bundleId.isEmpty {
            return XCUIApplication(bundleIdentifier: bundleId)
        }
        return XCUIApplication()
    }

    // MARK: - Route handlers

    private func handleHealth() -> HTTPResponse {
        let device = UIDevice.current
        return .json([
            "status": "ok",
            "platform": "ios",
            "version": "0.5.0",
            "device_name": device.name,
            "device_model": device.model,
            "os_version": device.systemVersion,
            "device_id": device.identifierForVendor?.uuidString ?? "unknown",
        ])
    }

    private func handleHierarchy(query: [String: String]) -> HTTPResponse {
        let application = app(query: query)
        let (hierarchy, keyboardHeight, safeAreaTop, safeAreaBottom): ([[String: Any]], Int, Int, Int) = DispatchQueue.main.sync {
            application.activate()
            let tree = HierarchySerializer.serialize(app: application)
            // Detect keyboard area: from the top of the toolbar (above keys)
            // to the bottom of the screen. Includes toolbar, predictions, and keys.
            let kbHeight: Int
            let keyboards = application.keyboards
            if keyboards.count > 0 {
                let screenHeight = application.frame.height
                // Find the topmost keyboard-related element (toolbar sits above keys)
                var topY = screenHeight
                // Check keyboard keys
                for i in 0..<keyboards.count {
                    topY = min(topY, keyboards.element(boundBy: i).frame.minY)
                }
                // Check toolbars (the input accessory toolbar sits above the keyboard)
                let toolbars = application.toolbars
                for i in 0..<toolbars.count {
                    let tb = toolbars.element(boundBy: i)
                    // Only count toolbars in the lower half of the screen
                    if tb.frame.minY > screenHeight / 2 {
                        topY = min(topY, tb.frame.minY)
                    }
                }
                kbHeight = Int(screenHeight - topY)
            } else {
                kbHeight = 0
            }
            // Detect safe area from SpringBoard's status bar (not the app's —
            // the status bar belongs to SpringBoard, same approach as Appium/WDA)
            let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
            let statusBar = springboard.statusBars.firstMatch
            let safeTop = statusBar.exists ? Int(statusBar.frame.height) : 0
            // Bottom safe area: 34pt on home indicator devices (status bar > 20pt), 0 otherwise
            let safeBottom = safeTop > 20 ? 34 : 0

            return (tree, kbHeight, safeTop, safeBottom)
        }
        // Wrap hierarchy with metadata
        let response: [String: Any] = [
            "tree": hierarchy,
            "keyboard_height": keyboardHeight,
            "safe_area_top": safeAreaTop,
            "safe_area_bottom": safeAreaBottom,
            "device_model": Self.deviceModel
        ]
        return .json(response)
    }

    private func handleTap(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let x = params["x"] as? Double,
              let y = params["y"] as? Double else {
            return .error("Missing x/y coordinates", status: 400)
        }
        let application = app(query: query)
        DispatchQueue.main.sync {
            application.activate()
            let normalized = application.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
            let target = normalized.withOffset(CGVector(dx: x, dy: y))
            target.tap()
        }
        return .json(["status": "ok"])
    }

    private func handleLongPress(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let x = params["x"] as? Double,
              let y = params["y"] as? Double else {
            return .error("Missing x/y coordinates", status: 400)
        }
        let duration = (params["duration_ms"] as? Double ?? 1000.0) / 1000.0
        let application = app(query: query)
        DispatchQueue.main.sync {
            application.activate()
            let normalized = application.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
            let target = normalized.withOffset(CGVector(dx: x, dy: y))
            target.press(forDuration: duration)
        }
        return .json(["status": "ok"])
    }

    private func handleType(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let text = params["text"] as? String else {
            return .error("Missing 'text' field", status: 400)
        }
        let application = app(query: query)
        DispatchQueue.main.sync {
            application.activate()
            application.typeText(text)
        }
        return .json(["status": "ok"])
    }

    private func handleBackspace(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let count = params["count"] as? Int else {
            return .error("Missing 'count' field", status: 400)
        }
        let application = app(query: query)
        let deleteString = String(repeating: XCUIKeyboardKey.delete.rawValue, count: count)
        DispatchQueue.main.sync {
            application.activate()
            application.typeText(deleteString)
        }
        return .json(["status": "ok"])
    }

    private func handleSwipe(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let startX = params["from_x"] as? Double,
              let startY = params["from_y"] as? Double,
              let endX = params["to_x"] as? Double,
              let endY = params["to_y"] as? Double else {
            return .error("Missing from_x/from_y/to_x/to_y coordinates", status: 400)
        }
        let duration = (params["duration_ms"] as? Double ?? 300.0) / 1000.0
        let application = app(query: query)
        DispatchQueue.main.sync {
            application.activate()
            let normalized = application.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
            let start = normalized.withOffset(CGVector(dx: startX, dy: startY))
            let end = normalized.withOffset(CGVector(dx: endX, dy: endY))
            start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .default, thenHoldForDuration: duration)
        }
        return .json(["status": "ok"])
    }

    /// Pinch gesture at specific coordinates via GestureSynthesizer.
    /// Request: { "x": N, "y": N, "scale": 2.0, "velocity": 5.0 }
    private func handlePinch(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let cx = params["x"] as? Double,
              let cy = params["y"] as? Double,
              let scale = params["scale"] as? Double else {
            return .error("Missing x/y/scale", status: 400)
        }
        let velocity = params["velocity"] as? Double ?? 5.0
        let duration = max(0.1, abs(scale - 1.0) / velocity)
        // Fingers start close together and spread apart for zoom-in (scale > 1),
        // or start apart and come together for zoom-out (scale < 1).
        let startDist = 50.0
        let endDist = startDist * scale

        let fingers: [[String: Any]] = [
            ["points": [[cx, cy - startDist], [cx, cy - endDist]] as [[Double]], "duration": NSNumber(value: duration)],
            ["points": [[cx, cy + startDist], [cx, cy + endDist]] as [[Double]], "duration": NSNumber(value: duration)],
        ]

        do {
            try GestureSynthesizer.synthesizeFingers(fingers)
            return .json(["status": "ok"])
        } catch {
            return .error("Pinch failed: \(error.localizedDescription)", status: 500)
        }
    }

    /// Execute a multi-touch gesture via GestureSynthesizer.
    /// Request: { "fingers": [{ "points": [[x,y], ...], "duration_ms": N }, ...] }
    private func handleGesture(body: Data?) -> HTTPResponse {
        guard let params = parseBody(body),
              let rawFingers = params["fingers"] as? [[String: Any]] else {
            return .error("Missing 'fingers' array", status: 400)
        }

        var fingers: [[String: Any]] = []
        for raw in rawFingers {
            guard let points = raw["points"] as? [[Any]],
                  points.count >= 2 else {
                return .error("Each finger needs at least 2 points", status: 400)
            }
            let durationMs = raw["duration_ms"] as? Double ?? 300.0
            let duration = durationMs / 1000.0
            let pts: [[Double]] = points.map { pt in
                [(pt[0] as? Double) ?? 0.0, (pt[1] as? Double) ?? 0.0]
            }
            fingers.append(["points": pts, "duration": NSNumber(value: duration)])
        }

        do {
            try GestureSynthesizer.synthesizeFingers(fingers)
            return .json(["status": "ok"])
        } catch {
            return .error("Gesture failed: \(error.localizedDescription)", status: 500)
        }
    }

    private func handleScreenshot() -> HTTPResponse {
        let pngData: Data? = DispatchQueue.main.sync {
            let screenshot = XCUIScreen.main.screenshot()
            return screenshot.pngRepresentation
        }
        guard let data = pngData else {
            return .error("Failed to capture screenshot")
        }
        return .png(data)
    }

    private func handleHideKeyboard(query: [String: String]) -> HTTPResponse {
        let application = app(query: query)
        DispatchQueue.main.sync {
            application.activate()
            let normalized = application.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
            let target = normalized.withOffset(CGVector(dx: 10, dy: 10))
            target.tap()
        }
        return .json(["status": "ok"])
    }

    private func handleLaunch(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let bundleId = params["bundle_id"] as? String, !bundleId.isEmpty else {
            return .error("Missing bundle_id", status: 400)
        }
        let application = XCUIApplication(bundleIdentifier: bundleId)
        DispatchQueue.main.sync {
            application.launch()
        }
        return .json(["status": "ok"])
    }

    private func handleStop(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let bundleId = params["bundle_id"] as? String, !bundleId.isEmpty else {
            return .error("Missing bundle_id", status: 400)
        }
        let application = XCUIApplication(bundleIdentifier: bundleId)
        DispatchQueue.main.sync {
            application.terminate()
        }
        return .json(["status": "ok"])
    }
}
