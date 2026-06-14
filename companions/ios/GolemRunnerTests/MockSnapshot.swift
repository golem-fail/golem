import XCTest

/// Minimal in-memory `XCUIElementSnapshot` for unit-testing `HierarchySerializer`
/// without a running app. It deliberately does not respond to `visibleFrame`, so
/// `SnapshotHelper.visibleFrame(of:)` returns `CGRectNull` and the serializer
/// falls back to `frame` for `visible_bounds`.
final class MockSnapshot: NSObject, XCUIElementSnapshot {
    var identifier: String
    var frame: CGRect
    var value: Any?
    var title: String
    var label: String
    var elementType: XCUIElement.ElementType
    var isEnabled: Bool
    var horizontalSizeClass: XCUIElement.SizeClass
    var verticalSizeClass: XCUIElement.SizeClass
    var placeholderValue: String?
    var isSelected: Bool
    var hasFocus: Bool
    var children: [XCUIElementSnapshot]
    var dictionaryRepresentation: [XCUIElement.AttributeName: Any] { [:] }

    init(
        elementType: XCUIElement.ElementType = .other,
        identifier: String = "",
        label: String = "",
        title: String = "",
        value: Any? = nil,
        placeholderValue: String? = nil,
        frame: CGRect = .zero,
        isEnabled: Bool = true,
        isSelected: Bool = false,
        hasFocus: Bool = false,
        children: [XCUIElementSnapshot] = []
    ) {
        self.elementType = elementType
        self.identifier = identifier
        self.label = label
        self.title = title
        self.value = value
        self.placeholderValue = placeholderValue
        self.frame = frame
        self.isEnabled = isEnabled
        self.isSelected = isSelected
        self.hasFocus = hasFocus
        self.horizontalSizeClass = .unspecified
        self.verticalSizeClass = .unspecified
        self.children = children
        super.init()
    }
}
