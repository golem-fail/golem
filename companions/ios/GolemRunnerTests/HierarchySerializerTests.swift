import Foundation
import Testing
import XCTest

/// Unit tests for `HierarchySerializer`'s pure serialization logic, driven by
/// `MockSnapshot` so no running app or simulator UI automation is needed.
struct HierarchySerializerTests {
    @Test func elementTypeNameMapsKnownTypes() {
        #expect(HierarchySerializer.elementTypeName(.button) == "button")
        #expect(HierarchySerializer.elementTypeName(.staticText) == "text")
        #expect(HierarchySerializer.elementTypeName(.checkBox) == "checkbox")
        #expect(HierarchySerializer.elementTypeName(.secureTextField) == "secure_text_field")
        #expect(HierarchySerializer.elementTypeName(.any) == "any")
    }

    @Test func serializeNodeEmitsCoreFields() {
        let snap = MockSnapshot(
            elementType: .button,
            identifier: "submit",
            label: "Increment",
            title: "+",
            frame: CGRect(x: 10, y: 20, width: 30, height: 40)
        )
        let node = HierarchySerializer.serializeNodePublic(snap)
        #expect(node["element_type"] as? String == "button")
        #expect(node["id"] as? String == "submit")
        #expect(node["label"] as? String == "Increment")
        #expect(node["title"] as? String == "+")
        // `text` prefers a non-empty title over the label.
        #expect(node["text"] as? String == "+")
        #expect(node["enabled"] as? Bool == true)
        let bounds = node["bounds"] as? [String: Int]
        #expect(bounds == ["x": 10, "y": 20, "width": 30, "height": 40])
        // Mock has no `visibleFrame` → serializer falls back to `frame`.
        #expect((node["visible_bounds"] as? [String: Int]) == bounds)
    }

    @Test func serializeNodeFallsBackToLabelForText() {
        let snap = MockSnapshot(elementType: .staticText, label: "Hello", title: "")
        let node = HierarchySerializer.serializeNodePublic(snap)
        #expect(node["text"] as? String == "Hello")
    }

    @Test func serializeNodeRecursesChildren() {
        let child = MockSnapshot(elementType: .staticText, label: "child")
        let parent = MockSnapshot(elementType: .group, label: "parent", children: [child])
        let node = HierarchySerializer.serializeNodePublic(parent)
        let kids = node["children"] as? [[String: Any]]
        #expect(kids?.count == 1)
        #expect(kids?.first?["label"] as? String == "child")
    }

    @Test func serializeNodeOmitsEmptyChildren() {
        let node = HierarchySerializer.serializeNodePublic(MockSnapshot(elementType: .other))
        #expect(node["children"] == nil)
    }
}
