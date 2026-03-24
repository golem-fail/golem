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

    /// Recursively serialize a single element snapshot node.
    private static func serializeNode(_ snapshot: any XCUIElementSnapshot) -> [String: Any] {
        let frame = snapshot.frame
        var node: [String: Any] = [
            "element_type": elementTypeName(snapshot.elementType),
            "text": snapshot.label,
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
