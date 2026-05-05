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
    ///
    /// Wraps dispatch in an Obj-C `@try/@catch`. XCUITest internals
    /// raise NSException on a few rare paths (XCTWaiter timeouts inside
    /// `XCUIApplication.init`, missing bundle id, snapshot races). Swift
    /// `try` can't catch those; without this guard, one bad request
    /// terminates the harness via `_XCTTerminateHandler`.
    func handle(method: String, path: String, body: Data?) -> HTTPResponse {
        var response: HTTPResponse?
        var caught: NSException?
        let ok = SnapshotHelper.catchNSException({
            response = self.dispatch(method: method, path: path, body: body)
        }, exception: &caught)
        if !ok {
            let name = caught?.name.rawValue ?? "unknown"
            let reason = caught?.reason ?? ""
            return .error("handler raised NSException \(name): \(reason)", status: 500)
        }
        return response ?? .error("handler returned no response", status: 500)
    }

    private func dispatch(method: String, path: String, body: Data?) -> HTTPResponse {
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
        case ("POST", "/press"):
            return handlePress(body: body)
        case ("POST", "/launch"):
            return handleLaunch(body: body, query: query)
        case ("POST", "/stop"):
            return handleStop(body: body, query: query)
        default:
            return .error("Not found: \(method) \(route)", status: 404)
        }
    }

    // MARK: - Main-thread watchdog
    //
    // Every XCUITest call must run on the main thread. Today's HTTP server
    // serves requests from background threads, so handlers `DispatchQueue.main
    // .sync` to hop. The hazard: when one main-thread call wedges (a
    // `typeText` racing the soft keyboard, a `windows.firstMatch` waiting on
    // a snapshot that never comes, an unexpected modal stealing focus), the
    // bare `.sync` form blocks the calling thread for the lifetime of the
    // wedge. The HTTP server's own thread is fine, but the runner sees the
    // request hang past its own deadline — and if the runner reissues, the
    // new connection's handler also hops to main and queues behind the
    // wedge.
    //
    // `runOnMain(timeout:)` switches every hop to `.async` plus a deadline-
    // semaphore wait. When the deadline fires, the handler returns 504 and
    // the connection closes. The work itself stays queued on main (we have
    // no way to cancel it from outside), so when main eventually frees the
    // late-completing call still runs — but the runner has already moved on.
    // That's a deliberately degraded but recovering state: better than the
    // current behaviour where every later request also pays the wedge time.

    private func runOnMain<T>(timeout: TimeInterval, _ work: @escaping () -> T) -> T? {
        let semaphore = DispatchSemaphore(value: 0)
        // Box the result so the @escaping closure can write through after
        // the outer function may have already returned (timeout path).
        let resultBox = ResultBox<T>()
        DispatchQueue.main.async {
            let value = work()
            resultBox.value = value
            semaphore.signal()
        }
        if semaphore.wait(timeout: .now() + timeout) == .timedOut {
            return nil
        }
        return resultBox.value
    }

    /// Void-returning overload — Swift infers `T = Void.Type` for void
    /// closures via the generic version, which doesn't compile. Returns
    /// `true` on completion within the deadline, `false` on timeout.
    @discardableResult
    private func runOnMainVoid(timeout: TimeInterval, _ work: @escaping () -> Void) -> Bool {
        let semaphore = DispatchSemaphore(value: 0)
        DispatchQueue.main.async {
            work()
            semaphore.signal()
        }
        return semaphore.wait(timeout: .now() + timeout) != .timedOut
    }

    /// Reference holder for the result of an async main-thread block, so
    /// the value survives across the @escaping closure / outer function
    /// boundary (the closure may write after the function has timed out
    /// and returned).
    private final class ResultBox<T> {
        var value: T?
    }

    // Per-handler timeouts. Generous enough to not cut off legitimate
    // slow paths (long type strings, post-launch settle), tight enough
    // that a true wedge fails-fast at one handler instead of stacking.
    private static let kTimeoutFast: TimeInterval = 5.0
    private static let kTimeoutLaunch: TimeInterval = 20.0
    private static let kTimeoutType: TimeInterval = 30.0

    private func gatewayTimeout(_ what: String) -> HTTPResponse {
        .error("\(what) timed out on main thread", status: 504)
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

    /// Last launched bundle ID — used as default for hierarchy/tap/etc.
    private var lastLaunchedBundle: String?

    /// Get an XCUIApplication, targeting a specific bundle ID if provided,
    /// otherwise falling back to the last launched bundle.
    ///
    /// Construction stays off-main: callers reference an already-running
    /// app, so the constructor's XCTWaiter wait resolves immediately
    /// without raising. The only constructor that can raise NSException
    /// is in `handleLaunch`, which targets a possibly-not-running bundle
    /// — that one is wrapped in `runOnMain` so the XCTest runloop's
    /// handler catches the exception instead of SIGABRT.
    private func app(query: [String: String]) -> XCUIApplication {
        if let bundleId = query["bundle_id"], !bundleId.isEmpty {
            return XCUIApplication(bundleIdentifier: bundleId)
        }
        if let bundleId = lastLaunchedBundle {
            return XCUIApplication(bundleIdentifier: bundleId)
        }
        return XCUIApplication()
    }

    // MARK: - Route handlers

    private func handleHealth() -> HTTPResponse {
        // /health intentionally does NOT touch the main thread — it should
        // stay responsive even when XCUITest is wedged so the runner can
        // distinguish "harness alive but stuck" from "harness dead".
        let device = UIDevice.current
        // Prefer the simulator's UDID (set by CoreSimulator in
        // `SIMULATOR_UDID`) over `identifierForVendor`. The latter is
        // a per-app identifier that has no relationship to the device
        // the runner booted, so the runner can't use it to verify
        // it's talking to the right simulator.
        let simulatorUdid = ProcessInfo.processInfo.environment["SIMULATOR_UDID"]
        let deviceId = simulatorUdid
            ?? device.identifierForVendor?.uuidString
            ?? "unknown"
        return .json([
            "status": "ok",
            "platform": "ios",
            "version": "0.5.9",
            "device_name": device.name,
            "device_model": device.model,
            "os_version": device.systemVersion,
            "device_id": deviceId,
        ])
    }

    private func handleHierarchy(query: [String: String]) -> HTTPResponse {
        let application = app(query: query)
        guard let result = runOnMain(timeout: Self.kTimeoutFast, { () -> ([[String: Any]], Int, Int, Int) in
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
        }) else {
            return gatewayTimeout("hierarchy")
        }
        let (hierarchy, keyboardHeight, safeAreaTop, safeAreaBottom) = result
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
        guard runOnMainVoid(timeout: Self.kTimeoutFast, {
            application.activate()
            // Force XCUITest to materialise an accessibility snapshot of
            // the topmost window before synthesising the tap. Rooting
            // the coordinate on the window (vs the application) also
            // ensures the resulting HID event is dispatched on the
            // same OS path the WKWebView gesture recognizer is wired
            // to receive.
            let win = application.windows.firstMatch
            _ = win.waitForExistence(timeout: 2.0)
            let normalized = win.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
            let target = normalized.withOffset(CGVector(dx: x, dy: y))
            // `press(forDuration:)` gives explicit touch-down + hold +
            // touch-up timing; bare `tap()` synthesises an instant
            // up-after-down that the WebView can race-drop the up of,
            // leaving the click event unfired.
            target.press(forDuration: 0.05)
        }) else {
            return gatewayTimeout("tap")
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
        // Long-press budget: gesture itself + 5s slack for window snapshot.
        let timeout = Self.kTimeoutFast + duration
        guard runOnMainVoid(timeout: timeout, {
            application.activate()
            let win = application.windows.firstMatch
            _ = win.waitForExistence(timeout: 2.0)
            let normalized = win.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
            let target = normalized.withOffset(CGVector(dx: x, dy: y))
            target.press(forDuration: duration)
        }) else {
            return gatewayTimeout("longpress")
        }
        return .json(["status": "ok"])
    }

    private func handleType(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let text = params["text"] as? String else {
            return .error("Missing 'text' field", status: 400)
        }
        let application = app(query: query)
        guard runOnMainVoid(timeout: Self.kTimeoutType, {
            application.activate()
            application.typeText(text)
        }) else {
            return gatewayTimeout("type")
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
        guard runOnMainVoid(timeout: Self.kTimeoutType, {
            application.activate()
            application.typeText(deleteString)
        }) else {
            return gatewayTimeout("backspace")
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
        let timeout = Self.kTimeoutFast + duration
        guard runOnMainVoid(timeout: timeout, {
            application.activate()
            let normalized = application.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
            let start = normalized.withOffset(CGVector(dx: startX, dy: startY))
            let end = normalized.withOffset(CGVector(dx: endX, dy: endY))
            start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .default, thenHoldForDuration: duration)
        }) else {
            return gatewayTimeout("swipe")
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
        let pngData: Data?? = runOnMain(timeout: Self.kTimeoutFast) {
            XCUIScreen.main.screenshot().pngRepresentation
        }
        guard let data = pngData?.flatMap({ $0 }) else {
            // Distinguish between a timed-out main and a screenshot that
            // ran but produced no PNG bytes.
            if pngData == nil {
                return gatewayTimeout("screenshot")
            }
            return .error("Failed to capture screenshot")
        }
        return .png(data)
    }

    private func handleHideKeyboard(query: [String: String]) -> HTTPResponse {
        let application = app(query: query)
        guard runOnMainVoid(timeout: Self.kTimeoutFast, {
            application.activate()
            let win = application.windows.firstMatch
            _ = win.waitForExistence(timeout: 2.0)
            let normalized = win.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
            let target = normalized.withOffset(CGVector(dx: 10, dy: 10))
            target.tap()
        }) else {
            return gatewayTimeout("hide-keyboard")
        }
        return .json(["status": "ok"])
    }

    /// Press a hardware/system button via `XCUIDevice`. XCUITest's
    /// `XCUIDevice.shared.press(.home)` is the version-stable path —
    /// `simctl ui <udid> home` was dropped in Xcode 26.
    private func handlePress(body: Data?) -> HTTPResponse {
        guard let params = parseBody(body),
              let button = params["button"] as? String else {
            return .error("Missing 'button' field", status: 400)
        }
        let mapped: XCUIDevice.Button
        switch button {
        case "home": mapped = .home
        default:
            return .error("Unsupported button on iOS: \(button)", status: 400)
        }
        guard runOnMainVoid(timeout: Self.kTimeoutFast, {
            XCUIDevice.shared.press(mapped)
        }) else {
            return gatewayTimeout("press")
        }
        return .json(["status": "ok"])
    }

    private func handleLaunch(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let bundleId = params["bundle_id"] as? String, !bundleId.isEmpty else {
            return .error("Missing bundle_id", status: 400)
        }
        let application = XCUIApplication(bundleIdentifier: bundleId)
        guard runOnMainVoid(timeout: Self.kTimeoutLaunch, {
            // activate() brings to foreground without restarting.
            // If not running, it launches it fresh.
            application.activate()
            // Block until XCUITest considers the app fully foregrounded
            // AND the WKWebView has rendered enough DOM that we can
            // observe at least one accessibility element produced by
            // the page itself. Without this gate the HTTP response
            // returns before the WebView's gesture recognizer is
            // wired — the next /tap then synthesises HID events the
            // WebView silently drops, the long-standing "first tap
            // after launch is dead" flake.
            //
            // `staticTexts.firstMatch` is the cheapest probe that
            // requires a real DOM render: empty/loading WebViews have
            // no static text in their tree, but an `<h1>` or button
            // label produces one as soon as it's painted.
            _ = application.wait(for: .runningForeground, timeout: 5.0)
            _ = application.windows.firstMatch.waitForExistence(timeout: 5.0)
            _ = application.staticTexts.firstMatch.waitForExistence(timeout: 5.0)
        }) else {
            return gatewayTimeout("launch")
        }
        lastLaunchedBundle = bundleId
        return .json(["status": "ok"])
    }

    private func handleStop(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let bundleId = params["bundle_id"] as? String, !bundleId.isEmpty else {
            return .error("Missing bundle_id", status: 400)
        }
        let application = XCUIApplication(bundleIdentifier: bundleId)
        guard runOnMainVoid(timeout: Self.kTimeoutFast, {
            application.terminate()
        }) else {
            return gatewayTimeout("stop")
        }
        return .json(["status": "ok"])
    }
}
