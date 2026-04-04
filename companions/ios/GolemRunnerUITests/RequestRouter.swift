import XCTest

/// Routes HTTP requests to XCUITest actions.
final class RequestRouter {

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
        case ("GET", "/screenshot"):
            return handleScreenshot()
        case ("POST", "/hide-keyboard"):
            return handleHideKeyboard(query: query)
        case ("GET", "/alert"):
            return handleGetAlert(query: query)
        case ("POST", "/alert"):
            return handlePostAlert(body: body, query: query)
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
            "version": "0.3.0",
            "device_name": device.name,
            "device_model": device.model,
            "os_version": device.systemVersion,
            "device_id": device.identifierForVendor?.uuidString ?? "unknown",
        ])
    }

    private func handleHierarchy(query: [String: String]) -> HTTPResponse {
        let application = app(query: query)
        let hierarchy: [[String: Any]] = DispatchQueue.main.sync {
            application.activate()
            return HierarchySerializer.serialize(app: application)
        }
        return .json(hierarchy)
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

    private func handleGetAlert(query: [String: String]) -> HTTPResponse {
        let application = app(query: query)
        let result: [String: Any] = DispatchQueue.main.sync {
            application.activate()
            let alert = application.alerts.firstMatch
            guard alert.exists else {
                return ["exists": false]
            }
            let label = alert.label
            let buttons = alert.buttons.allElementsBoundByIndex.map { $0.label }
            return [
                "exists": true,
                "text": label,
                "buttons": buttons
            ]
        }
        return .json(result)
    }

    private func handlePostAlert(body: Data?, query: [String: String]) -> HTTPResponse {
        guard let params = parseBody(body),
              let action = params["action"] as? String else {
            return .error("Missing 'action' field (accept/dismiss)", status: 400)
        }
        let application = app(query: query)
        let success: Bool = DispatchQueue.main.sync {
            application.activate()
            let alert = application.alerts.firstMatch
            guard alert.exists else { return false }

            switch action {
            case "accept":
                // Tap the last button (typically the accept/OK button).
                let buttons = alert.buttons.allElementsBoundByIndex
                guard let acceptButton = buttons.last else { return false }
                acceptButton.tap()
                return true
            case "dismiss":
                // Tap the first button (typically the cancel/dismiss button).
                let buttons = alert.buttons.allElementsBoundByIndex
                guard let dismissButton = buttons.first else { return false }
                dismissButton.tap()
                return true
            default:
                // Try to find a button matching the action text.
                let button = alert.buttons[action]
                guard button.exists else { return false }
                button.tap()
                return true
            }
        }
        if success {
            return .json(["status": "ok"])
        } else {
            return .error("Alert not found or button not found", status: 400)
        }
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
