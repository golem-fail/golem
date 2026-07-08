mod alerts;
mod hierarchy;
mod http;
mod webview;

#[cfg(test)]
use golem_element::Element;

pub use alerts::{detect_anr, find_alert, find_alert_buttons};
pub use hierarchy::{parse_hierarchy, CornerPosition, CutoutRect, HierarchyMeta, RoundedCorner};
pub use http::{CompanionClient, CompanionHealth};

pub(crate) use hierarchy::{
    build_backspace_body, build_gesture_body, build_long_press_body, build_swipe_body,
    build_tap_body, build_type_body, parse_text_unchanged,
};
pub(crate) use webview::{find_webview_bounds, replace_webview_children};

// Only reached from the inline test module below (`use super::*`) — the
// non-test crate never names these directly, so the re-export is gated to
// avoid an unused-import warning on the plain lib target.
#[cfg(test)]
pub(crate) use hierarchy::{
    normalize_android_rect, normalize_json, parse_corners_json, parse_cutouts_json,
    promote_label_to_id,
};
#[cfg(test)]
pub(crate) use http::coded_status_err;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- companion error classification ----

    // A 504 from the companion is its main-thread watchdog firing — code it as
    // wedged (D503) so it stops rendering as an opaque EX000.
    #[test]
    fn status_504_codes_as_companion_wedged() {
        let e = coded_status_err(
            reqwest::StatusCode::GATEWAY_TIMEOUT,
            "POST /tap returned 504: timed out on main thread".to_string(),
        );
        assert_eq!(
            golem_events::extract_code(&e),
            Some(golem_events::FailureCode::DeviceCompanionWedged),
        );
    }

    // Other non-success statuses (e.g. a companion 500) aren't cleanly
    // attributable — leave them uncoded rather than guess.
    #[test]
    fn status_500_stays_uncoded() {
        let e = coded_status_err(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "POST /tap returned 500: handler raised".to_string(),
        );
        assert_eq!(golem_events::extract_code(&e), None);
    }

    // ---- parse_hierarchy ----

    // 1. A single root object parses into an Element with node_count 1.
    #[test]
    fn parse_hierarchy_single_root_object() {
        let json = r#"{
            "element_type": "other", "text": null, "accessibility_label": null,
            "placeholder": null, "enabled": true, "checked": false,
            "clickable": false, "focused": false,
            "bounds": { "x": 0, "y": 0, "width": 100, "height": 200 },
            "children": []
        }"#;
        let (el, meta) = parse_hierarchy(json).expect("single root SHALL parse");
        assert_eq!(el.element_type, "other", "element_type SHALL round-trip");
        assert_eq!(meta.node_count, 1, "single node SHALL count as 1");
        assert_eq!(
            meta.keyboard_height, 0,
            "absent keyboard_height SHALL default 0"
        );
    }

    // 2. A single-element array unwraps to that element (no synthetic container).
    #[test]
    fn parse_hierarchy_single_element_array_unwraps() {
        let json = r#"[{
            "element_type": "Button", "text": "OK", "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 10, "height": 10 },
            "children": []
        }]"#;
        let (el, meta) = parse_hierarchy(json).expect("array of one SHALL parse");
        assert_eq!(
            el.element_type, "Button",
            "lone array element SHALL be unwrapped"
        );
        assert_eq!(
            meta.node_count, 1,
            "unwrapped lone element SHALL count as 1"
        );
    }

    // 3. A multi-root array is wrapped in a synthetic `other` container whose
    //    bounds span the union of children right/bottom edges.
    #[test]
    fn parse_hierarchy_multi_root_array_wraps_with_bounding_box() {
        let json = r#"[
            { "element_type": "A", "text": null, "accessibility_label": null,
              "placeholder": null,
              "bounds": { "x": 0, "y": 0, "width": 50, "height": 60 }, "children": [] },
            { "element_type": "B", "text": null, "accessibility_label": null,
              "placeholder": null,
              "bounds": { "x": 100, "y": 200, "width": 30, "height": 40 }, "children": [] }
        ]"#;
        let (el, meta) = parse_hierarchy(json).expect("multi-root SHALL parse");
        assert_eq!(
            el.element_type, "other",
            "wrapper SHALL be synthetic `other`"
        );
        assert_eq!(el.children.len(), 2, "wrapper SHALL hold both roots");
        assert_eq!(
            el.bounds.width, 130,
            "wrapper width SHALL be max child right edge"
        );
        assert_eq!(
            el.bounds.height, 240,
            "wrapper height SHALL be max child bottom edge"
        );
        assert_eq!(
            meta.node_count, 3,
            "two roots plus wrapper SHALL count as 3"
        );
    }

    // 4. The `{ "tree": ..., ...meta }` wrapper extracts metadata and uses `tree`.
    #[test]
    fn parse_hierarchy_tree_wrapper_extracts_meta() {
        let json = r#"{
            "tree": { "element_type": "other", "text": null,
                      "accessibility_label": null, "placeholder": null,
                      "bounds": { "x": 0, "y": 0, "width": 4, "height": 8 },
                      "children": [] },
            "keyboard_height": 300,
            "safe_area_top": 47,
            "safe_area_bottom": 34,
            "safe_area_left": 5,
            "safe_area_right": 6
        }"#;
        let (_el, meta) = parse_hierarchy(json).expect("tree wrapper SHALL parse");
        assert_eq!(
            meta.keyboard_height, 300,
            "keyboard_height SHALL be read from wrapper"
        );
        assert_eq!(meta.safe_area_top, 47, "safe_area_top SHALL be read");
        assert_eq!(meta.safe_area_bottom, 34, "safe_area_bottom SHALL be read");
        assert_eq!(meta.safe_area_left, 5, "safe_area_left SHALL be read");
        assert_eq!(meta.safe_area_right, 6, "safe_area_right SHALL be read");
    }

    // 5. Invalid JSON surfaces a contextual parse error.
    #[test]
    fn parse_hierarchy_invalid_json_errors() {
        let err = parse_hierarchy("not json").expect_err("garbage SHALL error");
        assert!(
            err.to_string().contains("failed to parse hierarchy JSON"),
            "error SHALL carry parse context, got: {err}"
        );
    }

    // 6. Valid JSON that is not a valid Element (missing bounds) errors at deserialize.
    #[test]
    fn parse_hierarchy_wrong_shape_errors() {
        let err = parse_hierarchy(r#"{"element_type": "x"}"#)
            .expect_err("missing bounds SHALL fail deserialize");
        assert!(
            err.to_string()
                .contains("failed to deserialize hierarchy into Element"),
            "error SHALL carry deserialize context, got: {err}"
        );
    }

    // 7. Android `class` + bounds get normalized through parse_hierarchy end to end.
    #[test]
    fn parse_hierarchy_normalizes_android_node() {
        let json = r#"{
            "class": "android.widget.Button",
            "text": "Tap",
            "bounds": { "left": 10, "top": 20, "right": 60, "bottom": 120 },
            "children": []
        }"#;
        let (el, _meta) = parse_hierarchy(json).expect("android node SHALL parse");
        assert_eq!(
            el.element_type, "Button",
            "class SHALL simplify to last segment"
        );
        assert_eq!(el.bounds.x, 10, "left SHALL map to x");
        assert_eq!(el.bounds.width, 50, "right-left SHALL be width");
        assert_eq!(el.bounds.height, 100, "bottom-top SHALL be height");
    }

    // ---- normalize_json ----

    // 8. `id` is renamed to accessibility_label when none present.
    #[test]
    fn normalize_renames_id_to_accessibility_label() {
        let mut v = json!({ "id": "save_btn" });
        normalize_json(&mut v);
        assert_eq!(
            v["accessibility_label"], "save_btn",
            "id SHALL become accessibility_label"
        );
        assert!(v.get("id").is_none(), "raw id SHALL be removed");
    }

    // 9. `id` rename is skipped when accessibility_label already present.
    #[test]
    fn normalize_keeps_existing_accessibility_label_over_id() {
        let mut v = json!({ "id": "a", "accessibility_label": "b" });
        normalize_json(&mut v);
        assert_eq!(
            v["accessibility_label"], "b",
            "existing label SHALL win over id"
        );
    }

    // 10. Android `class` simplifies to the final dotted segment.
    #[test]
    fn normalize_simplifies_android_class() {
        let mut v = json!({ "class": "android.widget.EditText" });
        normalize_json(&mut v);
        assert_eq!(
            v["element_type"], "EditText",
            "class SHALL simplify to last segment"
        );
    }

    // 11. Non-empty contentDescription fills an absent accessibility_label.
    #[test]
    fn normalize_uses_content_description_for_label() {
        let mut v = json!({ "contentDescription": "Close" });
        normalize_json(&mut v);
        assert_eq!(
            v["accessibility_label"], "Close",
            "contentDescription SHALL fill label"
        );
    }

    // 12. Empty contentDescription does not set a label.
    #[test]
    fn normalize_ignores_empty_content_description() {
        let mut v = json!({ "contentDescription": "" });
        normalize_json(&mut v);
        assert!(
            v.get("accessibility_label").is_none(),
            "empty contentDescription SHALL NOT set a label"
        );
    }

    // 13. Switch with value "1" is normalized to checked = true.
    #[test]
    fn normalize_switch_value_one_sets_checked() {
        let mut v = json!({ "element_type": "Switch", "value": "1" });
        normalize_json(&mut v);
        assert_eq!(
            v["checked"], true,
            "switch value \"1\" SHALL set checked true"
        );
    }

    // 14. Switch with value "true" (case-insensitive) sets checked.
    #[test]
    fn normalize_switch_value_true_sets_checked() {
        let mut v = json!({ "element_type": "checkbox", "value": "TRUE" });
        normalize_json(&mut v);
        assert_eq!(v["checked"], true, "value \"TRUE\" SHALL set checked true");
    }

    // 15. Switch with value "0" leaves checked unset (does not force false).
    #[test]
    fn normalize_switch_value_zero_does_not_set_checked() {
        let mut v = json!({ "element_type": "toggle", "value": "0" });
        normalize_json(&mut v);
        assert!(
            v.get("checked").is_none(),
            "value \"0\" SHALL NOT insert checked"
        );
    }

    // 16. Input element prefers `value` over placeholder/label for text.
    #[test]
    fn normalize_input_prefers_value_for_text() {
        let mut v = json!({
            "element_type": "text_field", "value": "typed",
            "placeholder": "hint", "label": "field"
        });
        normalize_json(&mut v);
        assert_eq!(
            v["text"], "typed",
            "input SHALL surface typed value as text"
        );
    }

    // 17. Empty-value input falls back to placeholder.
    #[test]
    fn normalize_empty_input_falls_back_to_placeholder() {
        let mut v = json!({
            "element_type": "text_field", "value": "",
            "placeholder": "Enter name", "label": "field"
        });
        normalize_json(&mut v);
        assert_eq!(
            v["text"], "Enter name",
            "empty input SHALL fall back to placeholder"
        );
    }

    // 18. Non-input prefers placeholder → label over the value field.
    #[test]
    fn normalize_non_input_ignores_value_for_text() {
        let mut v = json!({ "element_type": "other", "value": "internal", "label": "Visible" });
        normalize_json(&mut v);
        assert_eq!(
            v["text"], "Visible",
            "non-input SHALL prefer label over value"
        );
    }

    // 19. With nothing else, existing `text` is preserved.
    #[test]
    fn normalize_preserves_text_when_no_overrides() {
        let mut v = json!({ "element_type": "other", "text": "keep" });
        normalize_json(&mut v);
        assert_eq!(v["text"], "keep", "existing text SHALL be preserved");
    }

    // 20. normalize_json recurses into children.
    #[test]
    fn normalize_recurses_into_children() {
        let mut v = json!({
            "element_type": "other",
            "children": [ { "class": "android.widget.TextView", "id": "child" } ]
        });
        normalize_json(&mut v);
        assert_eq!(
            v["children"][0]["element_type"], "TextView",
            "child class SHALL normalize"
        );
        assert_eq!(
            v["children"][0]["accessibility_label"], "child",
            "child id SHALL normalize"
        );
    }

    // ---- normalize_android_rect ----

    // 21. left/top/right/bottom convert to x/y/width/height.
    #[test]
    fn android_rect_converts_edges_to_xywh() {
        let mut rect = serde_json::Map::new();
        rect.insert("left".into(), json!(10));
        rect.insert("top".into(), json!(20));
        rect.insert("right".into(), json!(40));
        rect.insert("bottom".into(), json!(60));
        normalize_android_rect(&mut rect);
        assert_eq!(rect["x"], 10, "x SHALL equal left");
        assert_eq!(rect["y"], 20, "y SHALL equal top");
        assert_eq!(rect["width"], 30, "width SHALL equal right-left");
        assert_eq!(rect["height"], 40, "height SHALL equal bottom-top");
    }

    // 22. Inverted edges clamp width/height to 0 (off-screen WebView clip).
    #[test]
    fn android_rect_clamps_negative_dims_to_zero() {
        let mut rect = serde_json::Map::new();
        rect.insert("left".into(), json!(100));
        rect.insert("top".into(), json!(100));
        rect.insert("right".into(), json!(50));
        rect.insert("bottom".into(), json!(50));
        normalize_android_rect(&mut rect);
        assert_eq!(rect["width"], 0, "negative width SHALL clamp to 0");
        assert_eq!(rect["height"], 0, "negative height SHALL clamp to 0");
    }

    // 23. Rect already in x/y form is left untouched.
    #[test]
    fn android_rect_skips_when_x_already_present() {
        let mut rect = serde_json::Map::new();
        rect.insert("x".into(), json!(7));
        rect.insert("left".into(), json!(99));
        normalize_android_rect(&mut rect);
        assert_eq!(rect["x"], 7, "existing x SHALL be untouched");
        assert!(
            rect.get("width").is_none(),
            "no conversion SHALL run when x present"
        );
    }

    // ---- promote_label_to_id ----

    // 24. label promotes to accessibility_label when label slot is null.
    #[test]
    fn promote_label_fills_null_label() {
        let mut map = serde_json::Map::new();
        map.insert("label".into(), json!("Submit"));
        map.insert("accessibility_label".into(), serde_json::Value::Null);
        promote_label_to_id(&mut map);
        assert_eq!(
            map["accessibility_label"], "Submit",
            "null label SHALL be filled from label"
        );
    }

    // 25. label does not overwrite a non-empty accessibility_label.
    #[test]
    fn promote_label_keeps_existing_nonempty() {
        let mut map = serde_json::Map::new();
        map.insert("label".into(), json!("ignored"));
        map.insert("accessibility_label".into(), json!("kept"));
        promote_label_to_id(&mut map);
        assert_eq!(
            map["accessibility_label"], "kept",
            "non-empty label SHALL NOT be overwritten"
        );
    }

    // ---- build_* request bodies ----

    // 26. Tap body serializes x/y.
    #[test]
    fn build_tap_body_serializes_coords() {
        let body = build_tap_body(3, 7).expect("tap body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["x"], 3, "x SHALL serialize");
        assert_eq!(v["y"], 7, "y SHALL serialize");
    }

    // 27. Type body serializes text.
    #[test]
    fn build_type_body_serializes_text() {
        let body = build_type_body("héllo").expect("type body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["text"], "héllo", "text SHALL serialize verbatim");
    }

    // 28. Backspace body serializes count.
    #[test]
    fn build_backspace_body_serializes_count() {
        let body = build_backspace_body(5).expect("backspace body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["count"], 5, "count SHALL serialize");
    }

    // 28b. parse_text_unchanged reads the companion's post-mutation flag:
    //      Some(true) only when the flag is explicitly true; Some(false)
    //      for a plain ok, an explicit false, or malformed/absent JSON
    //      (never None — a companion that ran the check answered).
    #[test]
    fn parse_text_unchanged_reads_flag() {
        assert_eq!(
            parse_text_unchanged(r#"{"status":"ok","text_unchanged":true}"#),
            Some(true),
            "explicit true flag SHALL be Some(true) (extend settle)"
        );
        assert_eq!(
            parse_text_unchanged(r#"{"status":"ok"}"#),
            Some(false),
            "absent flag (change observed) SHALL be Some(false)"
        );
        assert_eq!(
            parse_text_unchanged(r#"{"status":"ok","text_unchanged":false}"#),
            Some(false),
            "explicit false flag SHALL be Some(false)"
        );
        assert_eq!(
            parse_text_unchanged("not json"),
            Some(false),
            "malformed response SHALL degrade to Some(false), not extend"
        );
    }

    // 29. Long-press body serializes coords and duration.
    #[test]
    fn build_long_press_body_serializes_fields() {
        let body = build_long_press_body(1, 2, 800).expect("long press body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["x"], 1, "x SHALL serialize");
        assert_eq!(v["y"], 2, "y SHALL serialize");
        assert_eq!(v["duration_ms"], 800, "duration_ms SHALL serialize");
    }

    // 30. Swipe body serializes all five fields under their own names, so a
    //     from_y/to_x (or any) field swap in the wire contract is caught.
    //     Distinct argument values per field make a transposition observable.
    #[test]
    fn build_swipe_body_serializes_fields() {
        let body = build_swipe_body(1, 2, 3, 4, 250).expect("swipe body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["from_x"], 1, "from_x SHALL serialize");
        assert_eq!(v["from_y"], 2, "from_y SHALL serialize");
        assert_eq!(v["to_x"], 3, "to_x SHALL serialize");
        assert_eq!(v["to_y"], 4, "to_y SHALL serialize");
        assert_eq!(v["duration_ms"], 250, "duration_ms SHALL serialize");
    }

    // 31. Gesture body serializes finger points as [x,y] pairs plus duration.
    #[test]
    fn build_gesture_body_serializes_fingers() {
        let fingers = vec![
            crate::GestureFinger {
                points: vec![(0, 0), (5, 5)],
                duration_ms: 300,
            },
            crate::GestureFinger {
                points: vec![(9, 9)],
                duration_ms: 100,
            },
        ];
        let body = build_gesture_body(&fingers).expect("gesture body SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(
            v["fingers"][0]["points"][1],
            json!([5, 5]),
            "point SHALL be [x,y]"
        );
        assert_eq!(
            v["fingers"][0]["duration_ms"], 300,
            "duration SHALL serialize"
        );
        assert_eq!(
            v["fingers"][1]["points"][0],
            json!([9, 9]),
            "second finger SHALL serialize"
        );
    }

    // 32. Empty gesture serializes an empty fingers array.
    #[test]
    fn build_gesture_body_empty_fingers() {
        let body = build_gesture_body(&[]).expect("empty gesture SHALL serialize");
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(
            v["fingers"],
            json!([]),
            "empty input SHALL yield empty array"
        );
    }

    // ---- parse_cutouts_json ----

    // 33. None input yields an empty cutout vec.
    #[test]
    fn parse_cutouts_none_is_empty() {
        assert!(
            parse_cutouts_json(None).is_empty(),
            "None SHALL yield empty cutouts"
        );
    }

    // 34. Valid cutouts parse; zero/negative-area entries are filtered out.
    #[test]
    fn parse_cutouts_filters_zero_area() {
        let v = json!([
            { "x": 10, "y": 0, "width": 100, "height": 30 },
            { "x": 0, "y": 0, "width": 0, "height": 30 }
        ]);
        let cutouts = parse_cutouts_json(Some(&v));
        assert_eq!(cutouts.len(), 1, "zero-width cutout SHALL be filtered");
        assert_eq!(cutouts[0].width, 100, "valid cutout SHALL retain width");
    }

    // 35. Entries missing a required field are skipped.
    #[test]
    fn parse_cutouts_skips_missing_fields() {
        let v = json!([ { "x": 1, "y": 2, "width": 3 } ]);
        assert!(
            parse_cutouts_json(Some(&v)).is_empty(),
            "missing height SHALL skip entry"
        );
    }

    // ---- parse_corners_json ----

    // 36. Each corner position string maps to its enum variant.
    #[test]
    fn parse_corners_maps_positions() {
        let v = json!([
            { "position": "top_left", "radius": 5, "center_x": 5, "center_y": 5 },
            { "position": "top_right", "radius": 5, "center_x": 1, "center_y": 5 },
            { "position": "bottom_right", "radius": 5, "center_x": 1, "center_y": 1 },
            { "position": "bottom_left", "radius": 5, "center_x": 5, "center_y": 1 }
        ]);
        let corners = parse_corners_json(Some(&v));
        assert_eq!(corners.len(), 4, "all four corners SHALL parse");
        assert_eq!(
            corners[0].position,
            CornerPosition::TopLeft,
            "first SHALL be TopLeft"
        );
        assert_eq!(
            corners[3].position,
            CornerPosition::BottomLeft,
            "last SHALL be BottomLeft"
        );
    }

    // 37. Unknown position string skips the entry.
    #[test]
    fn parse_corners_skips_unknown_position() {
        let v = json!([ { "position": "middle", "radius": 5, "center_x": 1, "center_y": 1 } ]);
        assert!(
            parse_corners_json(Some(&v)).is_empty(),
            "unknown position SHALL be skipped"
        );
    }

    // 38. None input yields empty corners.
    #[test]
    fn parse_corners_none_is_empty() {
        assert!(
            parse_corners_json(None).is_empty(),
            "None SHALL yield empty corners"
        );
    }

    // ---- element-tree helpers (detect_anr / find_alert / buttons) ----
    // These build Elements via JSON since Element has no Default.

    fn el(json: serde_json::Value) -> Element {
        serde_json::from_value(json).expect("test element SHALL deserialize")
    }

    fn leaf(element_type: &str, text: Option<&str>) -> serde_json::Value {
        json!({
            "element_type": element_type,
            "text": text,
            "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 1, "height": 1 },
            "children": []
        })
    }

    // 39. detect_anr matches the straight-apostrophe title.
    #[test]
    fn detect_anr_matches_straight_apostrophe() {
        let tree = el(json!({
            "element_type": "FrameLayout", "text": null,
            "accessibility_label": null, "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 1, "height": 1 },
            "children": [ leaf("TextView", Some("App isn't responding")) ]
        }));
        assert!(
            detect_anr(&tree),
            "straight-apostrophe ANR title SHALL match"
        );
    }

    // 40. detect_anr matches the curly-apostrophe (Unicode) title.
    #[test]
    fn detect_anr_matches_curly_apostrophe() {
        let tree = el(leaf("TextView", Some("App isn\u{2019}t responding")));
        assert!(detect_anr(&tree), "curly-apostrophe ANR title SHALL match");
    }

    // 41. detect_anr is false for unrelated text.
    #[test]
    fn detect_anr_false_for_normal_ui() {
        let tree = el(leaf("TextView", Some("Welcome")));
        assert!(!detect_anr(&tree), "non-ANR text SHALL NOT match");
    }

    // 41b. detect_anr does NOT match iOS system prompts. These are
    //      dismissable interrupts (Touch ID / location / notifications),
    //      not device wedges — matching them would trigger a spurious
    //      reboot. iOS recovery rides the wedge paths instead.
    #[test]
    fn detect_anr_false_for_ios_system_prompts() {
        for prompt in [
            "Allow \u{201c}App\u{201d} to use your location?",
            "Do You Want to Allow \u{201c}App\u{201d} to Send You Notifications?",
            "Touch ID for \u{201c}App\u{201d}",
            "Face ID",
            "Sign in with your Apple Account",
        ] {
            let tree = el(json!({
                "element_type": "alert", "text": prompt,
                "accessibility_label": null, "placeholder": null,
                "bounds": { "x": 0, "y": 100, "width": 300, "height": 200 },
                "children": [
                    leaf("StaticText", Some(prompt)),
                    leaf("Button", Some("Allow")),
                    leaf("Button", Some("Don\u{2019}t Allow")),
                ]
            }));
            assert!(
                !detect_anr(&tree),
                "iOS system prompt {prompt:?} SHALL NOT be treated as an ANR"
            );
        }
    }

    // 42. find_alert returns an iOS native alert and extracts message as text.
    #[test]
    fn find_alert_ios_extracts_message() {
        let tree = el(json!({
            "element_type": "alert", "text": "Title", "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 10, "height": 10 },
            "children": [
                leaf("StaticText", Some("Alert Title")),
                leaf("StaticText", Some("This is the body")),
                leaf("Button", Some("OK"))
            ]
        }));
        let alert = find_alert(&tree).expect("alert SHALL be found");
        assert_eq!(
            alert.element_type, "alert",
            "found element SHALL be the alert"
        );
        assert_eq!(
            alert.text.as_deref(),
            Some("This is the body"),
            "alert text SHALL be the message (second non-button text)"
        );
    }

    // 43. find_alert detects the Android FrameLayout-with-Button dialog pattern.
    #[test]
    fn find_alert_android_dialog_pattern() {
        let tree = el(json!({
            "element_type": "FrameLayout", "text": null, "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 100, "width": 200, "height": 200 },
            "children": [
                leaf("TextView", Some("Permission")),
                leaf("Button", Some("Allow"))
            ]
        }));
        let alert = find_alert(&tree).expect("android dialog SHALL be found");
        assert_eq!(
            alert.element_type, "FrameLayout",
            "android alert SHALL be the frame"
        );
    }

    // 44. Android FrameLayout at y == 0 is NOT treated as an alert.
    #[test]
    fn find_alert_skips_top_anchored_frame() {
        let tree = el(json!({
            "element_type": "FrameLayout", "text": null, "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 200, "height": 200 },
            "children": [ leaf("Button", Some("X")) ]
        }));
        assert!(
            find_alert(&tree).is_none(),
            "y==0 frame SHALL NOT be an alert"
        );
    }

    // 45. find_alert returns None when no alert is present.
    #[test]
    fn find_alert_none_when_absent() {
        let tree = el(leaf("other", Some("content")));
        assert!(
            find_alert(&tree).is_none(),
            "tree without alert SHALL yield None"
        );
    }

    // 46. find_alert_buttons collects all buttons recursively, case-insensitively.
    #[test]
    fn find_alert_buttons_collects_all() {
        let alert = el(json!({
            "element_type": "alert", "text": "t", "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 10, "height": 10 },
            "children": [
                leaf("Button", Some("Cancel")),
                json!({
                    "element_type": "other", "text": null, "accessibility_label": null,
                    "placeholder": null,
                    "bounds": { "x": 0, "y": 0, "width": 1, "height": 1 },
                    "children": [ leaf("button", Some("OK")) ]
                })
            ]
        }));
        let buttons = find_alert_buttons(&alert);
        assert_eq!(
            buttons.len(),
            2,
            "both buttons SHALL be collected across depths"
        );
    }

    // 47. extract_alert_message returns the single text when only one non-button text exists.
    #[test]
    fn find_alert_single_text_becomes_message() {
        let tree = el(json!({
            "element_type": "alert", "text": "ignored-root", "accessibility_label": null,
            "placeholder": null,
            "bounds": { "x": 0, "y": 0, "width": 10, "height": 10 },
            "children": [ leaf("StaticText", Some("Only line")) ]
        }));
        let alert = find_alert(&tree).expect("alert SHALL be found");
        assert_eq!(
            alert.text.as_deref(),
            Some("Only line"),
            "single non-button text SHALL be used as message"
        );
    }

    // ---- find_webview_bounds ----

    // 48. Android WebView bounds read from {left,top}.
    #[test]
    fn find_webview_bounds_android() {
        let v = json!({
            "class": "android.webkit.WebView",
            "bounds": { "left": 5, "top": 15, "right": 100, "bottom": 200 }
        });
        assert_eq!(
            find_webview_bounds(&v),
            Some((5, 15)),
            "android webview SHALL read left/top"
        );
    }

    // 49. iOS web_view bounds read from {x,y}; array root is traversed.
    #[test]
    fn find_webview_bounds_ios_array_root() {
        let v = json!([
            { "element_type": "window", "children": [] },
            { "element_type": "web_view", "bounds": { "x": 7, "y": 8 } }
        ]);
        assert_eq!(
            find_webview_bounds(&v),
            Some((7, 8)),
            "ios webview SHALL read x/y from array"
        );
    }

    // 50. find_webview_bounds recurses into children.
    #[test]
    fn find_webview_bounds_nested_child() {
        let v = json!({
            "element_type": "root",
            "children": [
                { "element_type": "web_view", "bounds": { "x": 1, "y": 2 } }
            ]
        });
        assert_eq!(
            find_webview_bounds(&v),
            Some((1, 2)),
            "nested webview SHALL be found"
        );
    }

    // 51. No webview yields None.
    #[test]
    fn find_webview_bounds_none() {
        let v = json!({ "element_type": "other", "children": [] });
        assert_eq!(find_webview_bounds(&v), None, "no webview SHALL yield None");
    }

    // ---- replace_webview_children ----

    // 52. Replacing webview children swaps them for the DOM payload and returns true.
    #[test]
    fn replace_webview_children_swaps_dom() {
        let mut v = json!({
            "element_type": "web_view",
            "children": [ { "element_type": "stale" } ]
        });
        let dom = json!({ "element_type": "dom_root" });
        let replaced = replace_webview_children(&mut v, dom);
        assert!(replaced, "replacement SHALL report success");
        assert_eq!(
            v["children"].as_array().map(|a| a.len()),
            Some(1),
            "children SHALL be exactly the DOM"
        );
        assert_eq!(
            v["children"][0]["element_type"], "dom_root",
            "children SHALL be the DOM node"
        );
    }

    // 53. No webview present returns false and leaves the tree unchanged.
    #[test]
    fn replace_webview_children_no_webview() {
        let mut v = json!({ "element_type": "other", "children": [] });
        let replaced = replace_webview_children(&mut v, json!({}));
        assert!(!replaced, "no webview SHALL report failure");
    }

    // 54. Array root is traversed for replacement.
    #[test]
    fn replace_webview_children_array_root() {
        let mut v = json!([
            { "element_type": "window", "children": [] },
            { "class": "android.webkit.WebView", "children": [ {} ] }
        ]);
        let replaced = replace_webview_children(&mut v, json!({ "element_type": "dom" }));
        assert!(replaced, "array-root webview SHALL be replaced");
        assert_eq!(
            v[1]["children"][0]["element_type"], "dom",
            "DOM SHALL replace android webview kids"
        );
    }

    // ---- CompanionClient URL / timeout (non-network) ----

    // 55. url() with no default query produces base_url + path.
    #[test]
    fn companion_url_without_query() {
        let c = CompanionClient::new(1234);
        assert_eq!(
            c.url("/tap"),
            "http://localhost:1234/tap",
            "bare path SHALL append to base"
        );
    }

    // 56. url() appends default query with `?` when path has none.
    #[test]
    fn companion_url_appends_query_with_question_mark() {
        let c = CompanionClient::new(1234);
        c.set_default_query("bundle_id=fail.golem.test");
        assert_eq!(
            c.url("/hierarchy"),
            "http://localhost:1234/hierarchy?bundle_id=fail.golem.test",
            "query SHALL be joined with ?"
        );
    }

    // 57. url() uses `&` when the path already contains a query string.
    #[test]
    fn companion_url_appends_query_with_ampersand() {
        let c = CompanionClient::new(1234);
        c.set_default_query("a=b");
        assert_eq!(
            c.url("/x?foo=1"),
            "http://localhost:1234/x?foo=1&a=b",
            "existing query SHALL be extended with &"
        );
    }

    // 58. Request timeout round-trips; ZERO clears it.
    #[test]
    fn companion_request_timeout_roundtrip() {
        let c = CompanionClient::new(1);
        assert!(
            c.current_request_timeout().is_none(),
            "default SHALL be no timeout"
        );
        c.set_request_timeout(std::time::Duration::from_millis(250));
        assert_eq!(
            c.current_request_timeout(),
            Some(std::time::Duration::from_millis(250)),
            "set timeout SHALL round-trip"
        );
        c.set_request_timeout(std::time::Duration::ZERO);
        assert!(
            c.current_request_timeout().is_none(),
            "ZERO SHALL clear the timeout"
        );
    }

    // 59. Sub-millisecond Durations truncate to 0ms (whole-ms precision),
    //     which clears the timeout rather than clamping up to 1ms.
    #[test]
    fn companion_request_timeout_sub_millisecond_truncates_to_zero() {
        let c = CompanionClient::new(1);
        c.set_request_timeout(std::time::Duration::from_millis(250));
        c.set_request_timeout(std::time::Duration::from_micros(500));
        assert!(
            c.current_request_timeout().is_none(),
            "sub-millisecond timeout SHALL truncate to 0ms and clear the timeout"
        );
    }
}
