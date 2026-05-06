import XCTest

/// Converts an XCUIApplication's element snapshot tree into a JSON-compatible dictionary
/// matching GOLEM's Element struct.
enum HierarchySerializer {

    /// Serialize the full element hierarchy for the given application.
    static func serialize(app: XCUIApplication) -> [[String: Any]] {
        let snapshot: XCUIElementSnapshot
        do {
            snapshot = try app.snapshot()
        } catch {
            return []
        }
        return snapshot.children.map { serializeNode($0) }
    }

    /// Serialize any system alerts presented by SpringBoard (deep-link
    /// "Open in <App>?" confirms, photo-library prompts, etc.). These
    /// are owned by `com.apple.springboard`, not the app under test, so
    /// they don't appear in `serialize(app:)`. Returning them as
    /// additional top-level entries lets `find_alert` on the runner side
    /// pick them up without needing to know which process owns the
    /// alert — closer to what a user sees on screen.
    static func serializeSpringBoardAlerts() -> [[String: Any]] {
        // System alert ownership varies on iOS — SpringBoard for most,
        // but iOS 26+ openURL confirmations show up under
        // `com.apple.springboard.SystemApp` or sometimes the calling
        // simulator's launchd path. Probe each candidate.
        let candidateBundles = [
            "com.apple.springboard",
            "com.apple.springboard.SystemApp",
        ]
        var out: [[String: Any]] = []
        for bid in candidateBundles {
            let app = XCUIApplication(bundleIdentifier: bid)
            // .alerts (UIAlertController .alert)
            let alerts = app.alerts
            for i in 0..<alerts.count {
                let el = alerts.element(boundBy: i)
                guard el.exists else { continue }
                if let snap = try? el.snapshot() {
                    out.append(serializeNode(snap))
                }
            }
            // .sheets (UIAlertController .actionSheet)
            let sheets = app.sheets
            for i in 0..<sheets.count {
                let el = sheets.element(boundBy: i)
                guard el.exists else { continue }
                if let snap = try? el.snapshot() {
                    var node = serializeNode(snap)
                    node["element_type"] = "alert"
                    out.append(node)
                }
            }
            // NOTE: a dialog-shape fallback via `try? app.snapshot()` was
            // removed here. SpringBoard's full snapshot is expensive
            // and, when polled repeatedly by accept_alert, has been
            // observed to wedge or terminate the XCTest harness. Stick
            // to the cheap `.alerts` / `.sheets` queries. If a system
            // confirmation doesn't classify as either, callers will get
            // an empty array and accept_alert can fall through to its
            // own timeout instead of taking the harness down with it.
            if !out.isEmpty { break }
        }
        return out
    }

    /// Heuristic: a window-or-other that owns >=2 visible buttons and
    /// at least one non-button text child, rendered in the middle of
    /// the screen. Catches deep-link / "share with" confirms that
    /// aren't formal UIAlertController instances.
    private static func findDialogLikeWindow(_ snap: any XCUIElementSnapshot) -> (any XCUIElementSnapshot)? {
        if countButtons(snap) >= 2 && hasNonButtonText(snap) {
            // Reject root windows that contain the whole UI; require
            // a moderately small bounding box.
            let f = snap.frame
            if f.width > 0 && f.height > 0 && f.width < 600 && f.height < 600 {
                return snap
            }
        }
        for child in snap.children {
            if let hit = findDialogLikeWindow(child) {
                return hit
            }
        }
        return nil
    }

    private static func countButtons(_ snap: any XCUIElementSnapshot) -> Int {
        var n = snap.elementType == .button ? 1 : 0
        for c in snap.children { n += countButtons(c) }
        return n
    }

    private static func hasNonButtonText(_ snap: any XCUIElementSnapshot) -> Bool {
        if snap.elementType != .button && !snap.label.isEmpty {
            return true
        }
        for c in snap.children {
            if hasNonButtonText(c) { return true }
        }
        return false
    }

    /// Public wrapper around `serializeNode` for the debug probe.
    static func serializeNodePublic(_ snap: any XCUIElementSnapshot) -> [String: Any] {
        return serializeNode(snap)
    }

    /// Recursively serialize a single element snapshot node.
    private static func serializeNode(_ snapshot: any XCUIElementSnapshot) -> [String: Any] {
        let frame = snapshot.frame
        // For HTML elements rendered inside a WKWebView, `snapshot.label`
        // is the computed accessibility name (i.e. `aria-label` when set,
        // otherwise the visible text). `snapshot.title` is XCUITest's view
        // of the element's title attribute, which for HTML buttons with
        // an explicit `aria-label` resolves to the visible textContent.
        // So `<button aria-label="Increment">+</button>` exposes
        // `label = "Increment"` and `title = "+"`.
        //
        // Using `title || label` for `text` matches what the user sees on
        // screen — `tap on_text="+"` hits the rendered "+" without
        // having to wait for WebKit Inspector DOM enrichment to overlay
        // a separate text. `accessibility_label` still gets the aria-label
        // via the `id` / `label` fields downstream so identity-based
        // selectors keep working.
        let visibleText: String
        if !snapshot.title.isEmpty {
            visibleText = snapshot.title
        } else {
            visibleText = snapshot.label
        }
        var node: [String: Any] = [
            "element_type": elementTypeName(snapshot.elementType),
            "text": visibleText,
            "label": snapshot.label,
            "title": snapshot.title,
            "value": (snapshot.value as? String) ?? "",
            "id": snapshot.identifier,
            "placeholder": snapshot.placeholderValue ?? "",
            "enabled": snapshot.isEnabled,
            "checked": snapshot.isSelected,
            "clickable": snapshot.isEnabled,
            "focused": snapshot.hasFocus,
            "bounds": [
                "x": Int(frame.origin.x),
                "y": Int(frame.origin.y),
                "width": Int(frame.size.width),
                "height": Int(frame.size.height)
            ]
        ]

        // Access private visibleFrame via Objective-C helper (avoids Swift
        // protocol existential issues with KVC on struct-returning properties).
        let visibleFrame: CGRect
        let vf = SnapshotHelper.visibleFrame(of: snapshot)
        if !vf.isNull {
            visibleFrame = vf
        } else {
            visibleFrame = frame
        }
        node["visible_bounds"] = [
            "x": Int(visibleFrame.origin.x),
            "y": Int(visibleFrame.origin.y),
            "width": Int(visibleFrame.size.width),
            "height": Int(visibleFrame.size.height)
        ]

        let children = snapshot.children.map { serializeNode($0) }
        if !children.isEmpty {
            node["children"] = children
        }

        return node
    }

    /// Map XCUIElement.ElementType to a human-readable string name.
    static func elementTypeName(_ type: XCUIElement.ElementType) -> String {
        switch type {
        case .any:                  return "any"
        case .other:                return "other"
        case .application:          return "application"
        case .group:                return "group"
        case .window:               return "window"
        case .sheet:                return "sheet"
        case .drawer:               return "drawer"
        case .alert:                return "alert"
        case .dialog:               return "dialog"
        case .button:               return "button"
        case .radioButton:          return "radio_button"
        case .radioGroup:           return "radio_group"
        case .checkBox:             return "checkbox"
        case .disclosureTriangle:   return "disclosure_triangle"
        case .popUpButton:          return "popup_button"
        case .comboBox:             return "combo_box"
        case .menuButton:           return "menu_button"
        case .toolbarButton:        return "toolbar_button"
        case .popover:              return "popover"
        case .keyboard:             return "keyboard"
        case .key:                  return "key"
        case .navigationBar:        return "navigation_bar"
        case .tabBar:               return "tab_bar"
        case .tabGroup:             return "tab_group"
        case .toolbar:              return "toolbar"
        case .statusBar:            return "status_bar"
        case .table:                return "table"
        case .tableRow:             return "table_row"
        case .tableColumn:          return "table_column"
        case .outline:              return "outline"
        case .outlineRow:           return "outline_row"
        case .browser:              return "browser"
        case .collectionView:       return "collection_view"
        case .slider:               return "slider"
        case .pageIndicator:        return "page_indicator"
        case .progressIndicator:    return "progress_indicator"
        case .activityIndicator:    return "activity_indicator"
        case .segmentedControl:     return "segmented_control"
        case .picker:               return "picker"
        case .pickerWheel:          return "picker_wheel"
        case .switch:               return "switch"
        case .toggle:               return "toggle"
        case .link:                 return "link"
        case .image:                return "image"
        case .icon:                 return "icon"
        case .searchField:          return "search_field"
        case .scrollView:           return "scroll_view"
        case .scrollBar:            return "scroll_bar"
        case .staticText:           return "text"
        case .textField:            return "text_field"
        case .secureTextField:      return "secure_text_field"
        case .datePicker:           return "date_picker"
        case .textView:             return "text_view"
        case .menu:                 return "menu"
        case .menuItem:             return "menu_item"
        case .menuBar:              return "menu_bar"
        case .menuBarItem:          return "menu_bar_item"
        case .map:                  return "map"
        case .webView:              return "web_view"
        case .incrementArrow:       return "increment_arrow"
        case .decrementArrow:       return "decrement_arrow"
        case .timeline:             return "timeline"
        case .ratingIndicator:      return "rating_indicator"
        case .valueIndicator:       return "value_indicator"
        case .splitGroup:           return "split_group"
        case .splitter:             return "splitter"
        case .relevanceIndicator:   return "relevance_indicator"
        case .colorWell:            return "color_well"
        case .helpTag:              return "help_tag"
        case .matte:                return "matte"
        case .dockItem:             return "dock_item"
        case .ruler:                return "ruler"
        case .rulerMarker:          return "ruler_marker"
        case .grid:                 return "grid"
        case .levelIndicator:       return "level_indicator"
        case .cell:                 return "cell"
        case .layoutArea:           return "layout_area"
        case .layoutItem:           return "layout_item"
        case .handle:               return "handle"
        case .stepper:              return "stepper"
        case .tab:                  return "tab"
        case .touchBar:             return "touch_bar"
        case .statusItem:           return "status_item"
        @unknown default:           return "unknown"
        }
    }
}
