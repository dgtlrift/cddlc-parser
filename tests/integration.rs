/// Integration tests for cddlc-parser.
///
/// These tests exercise complete CDDL snippets representative of real-world
/// usage patterns: IoT device descriptions, CoAP payloads, sensor networks.

use std::path::PathBuf;
use cddlc_parser::{
    parse_cddl, PragmaValue, RuleKind, Type2,
};

fn path() -> PathBuf { PathBuf::from("test.cddl") }

fn parse(src: &str) -> cddlc_parser::CddlModule {
    parse_cddl(src, path()).expect("parse failed").value
}

// ── Basic rule forms ──────────────────────────────────────────────────────────

#[test]
fn test_primitive_alias() {
    let m = parse("device-id = uint");
    assert_eq!(m.rules.len(), 1);
    let rule = &m.rules[0].node;
    assert_eq!(rule.name.node.as_str(), "device-id");
    assert!(matches!(rule.kind, RuleKind::Type { extend: false, .. }));
}

#[test]
fn test_struct_with_fields() {
    let m = parse(r#"
        sensor = {
            id:    uint,
            label: tstr,
            value: float64,
        }
    "#);
    assert_eq!(m.rules.len(), 1);
    let rule = &m.rules[0].node;
    assert!(matches!(rule.kind, RuleKind::Type { .. }));
}

#[test]
fn test_array_rule() {
    let m = parse("readings = [* float32]");
    assert_eq!(m.rules.len(), 1);
}

#[test]
fn test_type_choice() {
    let m = parse("status = \"ok\" / \"warn\" / \"error\"");
    let rule = &m.rules[0].node;
    if let RuleKind::Type { ty, .. } = &rule.kind {
        assert_eq!(ty.node.choices.len(), 3);
    } else {
        panic!("expected type rule");
    }
}

#[test]
fn test_extend_type_rule() {
    let m = parse("status /= \"unknown\"");
    if let RuleKind::Type { extend, .. } = &m.rules[0].node.kind {
        assert!(*extend);
    } else {
        panic!("expected type rule");
    }
}

#[test]
fn test_extend_group_rule() {
    let m = parse("$$my-group //= ( extra: bool )");
    let extend = match &m.rules[0].node.kind {
        RuleKind::Type  { extend, .. } => *extend,
        RuleKind::Group { extend, .. } => *extend,
    };
    assert!(extend);
}

// ── Occurrences ───────────────────────────────────────────────────────────────

#[test]
fn test_optional_field() {
    let m = parse("msg = { id: uint, ? label: tstr }");
    // Parser should produce map with two entries, second optional
    assert_eq!(m.rules.len(), 1);
}

#[test]
fn test_bounded_occurrence() {
    let m = parse("packet = [2*8 uint]");
    assert_eq!(m.rules.len(), 1);
}

// ── Generics ──────────────────────────────────────────────────────────────────

#[test]
fn test_generic_definition() {
    let m = parse("tagged<T> = { tag: uint, value: T }");
    let rule = &m.rules[0].node;
    assert_eq!(rule.generic_params.len(), 1);
    assert_eq!(rule.generic_params[0].node.as_str(), "T");
}

#[test]
fn test_generic_two_params() {
    let m = parse("pair<A, B> = [A, B]");
    let rule = &m.rules[0].node;
    assert_eq!(rule.generic_params.len(), 2);
}

#[test]
fn test_generic_usage() {
    let m = parse(r#"
        labeled<T> = { label: tstr, value: T }
        temp-reading = labeled<float32>
    "#);
    assert_eq!(m.rules.len(), 2);
}

// ── Tags ──────────────────────────────────────────────────────────────────────

#[test]
fn test_cbor_tag() {
    let m = parse("timestamp = #6.1(uint)");
    let rule = &m.rules[0].node;
    if let RuleKind::Type { ty, .. } = &rule.kind {
        let t2 = &ty.node.choices[0].node.base.node;
        assert!(matches!(t2, Type2::Tagged { tag: 1, .. }));
    } else {
        panic!("expected type rule");
    }
}

#[test]
fn test_large_tag_number() {
    let m = parse("my-tagged = #6.65535(bstr)");
    let rule = &m.rules[0].node;
    if let RuleKind::Type { ty, .. } = &rule.kind {
        let t2 = &ty.node.choices[0].node.base.node;
        assert!(matches!(t2, Type2::Tagged { tag: 65535, .. }));
    }
}

// ── Control operators ─────────────────────────────────────────────────────────

#[test]
fn test_size_control() {
    let m = parse("label = tstr .size 64");
    assert_eq!(m.rules.len(), 1);
}

#[test]
fn test_range_control() {
    let m = parse("temp = int .ge -40 .le 85");
    assert_eq!(m.rules.len(), 1);
}

#[test]
fn test_regexp_control() {
    let m = parse(r#"serial = tstr .regexp "[A-Z]{3}-[0-9]{4}""#);
    assert_eq!(m.rules.len(), 1);
}

#[test]
fn test_cbor_control() {
    let m = parse("embedded = bstr .cbor uint");
    assert_eq!(m.rules.len(), 1);
}

#[test]
fn test_default_control() {
    let m = parse("priority = uint .default 0");
    assert_eq!(m.rules.len(), 1);
}

// ── Pragmas ───────────────────────────────────────────────────────────────────

#[test]
fn test_import_pragma() {
    let m = parse(r#"
; @import "common/types.cddl"
device-id = uint
"#);
    assert_eq!(m.file_pragmas.len(), 1);
    assert!(matches!(
        &m.file_pragmas[0].value,
        PragmaValue::Import(p) if p == "common/types.cddl"
    ));
}

#[test]
fn test_capacity_pragma() {
    let m = parse(r#"
; @capacity 32
readings = [* float32]
"#);
    let rule = &m.rules[0].node;
    assert_eq!(rule.pragmas.len(), 1);
    assert!(matches!(rule.pragmas[0].value, PragmaValue::Capacity(32)));
}

#[test]
fn test_multiple_pragmas_on_rule() {
    let m = parse(r#"
; @capacity 16
; @str-capacity 128
; @doc "Log entry list"
log-entries = [* log-entry]
"#);
    let rule = &m.rules[0].node;
    assert_eq!(rule.pragmas.len(), 3);
}

#[test]
fn test_deprecated_pragma() {
    let m = parse(r#"
; @deprecated
old-format = { x: uint }
"#);
    assert!(matches!(m.rules[0].node.pragmas[0].value, PragmaValue::Deprecated));
}

#[test]
fn test_regex_hook_pragma() {
    let m = parse(r#"
; @regex-hook validate_device_serial
serial = tstr .regexp "[A-Z]{3}-[0-9]+"
"#);
    assert!(matches!(
        &m.rules[0].node.pragmas[0].value,
        PragmaValue::RegexHook(name) if name == "validate_device_serial"
    ));
}

#[test]
fn test_unknown_pragma_preserved() {
    let src = r#"
; @my-backend-custom-pragma foobar
device-id = uint
"#;
    let result = parse_cddl(src, path()).unwrap();
    // Unknown pragmas produce warnings
    assert!(!result.warnings.is_empty());
    // But the pragma is still preserved
    let rule = &result.value.rules[0].node;
    assert_eq!(rule.pragmas.len(), 1);
    assert!(matches!(rule.pragmas[0].value, PragmaValue::Unknown(_)));
}

// ── Complex real-world examples ───────────────────────────────────────────────

#[test]
fn test_iot_sensor_schema() {
    let src = r#"
; @import "primitives.cddl"

; @doc "A complete sensor network message"
sensor-message = {
    version:   uint,
    device-id: device-id,
    timestamp: #6.1(uint),
    payload:   sensor-payload,
}

device-id = tstr .size 16

; @capacity 32
sensor-payload = [* sensor-reading]

; @doc "Individual sensor reading"
sensor-reading = {
    sensor-type: sensor-type,
    value:        float32,
    ? unit:       tstr,
}

sensor-type = "temperature" / "humidity" / "pressure" / "co2"
"#;
    let result = parse_cddl(src, path()).unwrap();
    let m = result.value;
    assert_eq!(m.file_pragmas.len(), 1);  // @import
    assert_eq!(m.rules.len(), 5);
}

#[test]
fn test_coap_message_schema() {
    let src = r#"
coap-message = {
    ver:    1,
    t:      uint .size 2,
    tkl:    uint .size 4,
    code:   uint .size 8,
    mid:    uint .size 16,
    token:  bstr,
    ? options: [* coap-option],
    ? payload: bstr,
}

coap-option = {
    delta:  uint,
    length: uint,
    value:  bstr,
}
"#;
    let m = parse(src);
    assert_eq!(m.rules.len(), 2);
}

#[test]
fn test_multiple_rules_ordering() {
    let src = r#"
a = uint
b = tstr
c = bool
d = a / b / c
"#;
    let m = parse(src);
    assert_eq!(m.rules.len(), 4);
    assert_eq!(m.rules[0].node.name.node.as_str(), "a");
    assert_eq!(m.rules[3].node.name.node.as_str(), "d");
}

// ── Inline comment handling ───────────────────────────────────────────────────

#[test]
fn test_inline_comment_after_comma() {
    // The original failing case: `;` comment after a value+comma on same line
    let src = r#"TestFormat = (
    feild_test: uint .size 4, ; This comment broke the parser
)"#;
    let m = parse(src);
    assert_eq!(m.rules.len(), 1);
    let rule = &m.rules[0].node;
    if let RuleKind::Group { group, .. } = &rule.kind {
        let entries = &group.node.choices[0].entries;
        assert_eq!(entries.len(), 1, "expected 1 field, got {}", entries.len());
    } else {
        panic!("expected group rule");
    }
}

#[test]
fn test_inline_comment_multiple_fields() {
    let src = r#"sensor = {
    id:    uint,           ; unique sensor id
    label: tstr,           ; human-readable name
    value: float32,        ; last reading
}"#;
    let m = parse(src);
    assert_eq!(m.rules.len(), 1);
    let rule = &m.rules[0].node;
    if let RuleKind::Type { ty, .. } = &rule.kind {
        if let Type2::Map(group) = &ty.node.choices[0].node.base.node {
            let entries = &group.node.choices[0].entries;
            assert_eq!(entries.len(), 3, "expected 3 fields, got {}", entries.len());
        } else {
            panic!("expected map");
        }
    } else {
        panic!("expected type rule");
    }
}

#[test]
fn test_inline_comment_on_last_field_no_trailing_comma() {
    // Inline comment on the last field which has no trailing comma
    let src = r#"msg = {
    id:   uint,   ; required
    text: tstr    ; optional in practice but required here
}"#;
    let m = parse(src);
    assert_eq!(m.rules.len(), 1);
}

#[test]
fn test_inline_comment_in_group_rule() {
    // Group rule (parentheses) with inline comments — the exact failing schema
    let src = r#"; This is a CDDL comment

TestFormat = (
; This field below has an inline comment
    feild_test: uint .size 4, ; This comment breaks the parser
)
"#;
    let m = parse(src);
    assert_eq!(m.rules.len(), 1);
    assert_eq!(m.rules[0].node.name.node.as_str(), "TestFormat");
}

#[test]
fn test_inline_comment_between_type_choices() {
    // Inline comment between `/` type alternatives
    let src = "status = \"ok\" ; ok\n    / \"warn\" ; warn\n    / \"error\" ; error";
    let m = parse(src);
    assert_eq!(m.rules.len(), 1);
    if let RuleKind::Type { ty, .. } = &m.rules[0].node.kind {
        assert_eq!(ty.node.choices.len(), 3);
    } else {
        panic!("expected type rule");
    }
}

#[test]
fn test_inline_comment_with_pragma_on_preceding_line() {
    // Mix of pragma comments and inline comments
    let src = r#"; @capacity 4
readings = [
    * float32, ; each reading is a 32-bit float
]
"#;
    let m = parse(src);
    assert_eq!(m.rules.len(), 1);
    assert_eq!(m.rules[0].node.pragmas.len(), 1);
}

#[test]
fn test_inline_comment_only_comment_no_fields_after() {
    // Inline comment after the last field before closing brace
    let src = r#"device = {
    id: uint, ; the device id
    name: tstr ; the device name — no comma, inline comment
}"#;
    let m = parse(src);
    assert_eq!(m.rules.len(), 1);
}

// ── Enumeration (&) expressions — RFC 8610 §3.9 ──────────────────────────────

#[test]
fn test_enum_ref_named() {
    // &typename as a standalone type alias (RFC 8610 §3.9)
    let m = parse(r#"
        terminal-type = (
            garden-controller: 1,
            battery-powered:   2,
            solar-powered:     3,
        )
        device-kind = &terminal-type
    "#);
    assert_eq!(m.rules.len(), 2);
    let rule = &m.rules[1].node;
    assert_eq!(rule.name.node.as_str(), "device-kind");
    if let RuleKind::Type { ty, .. } = &rule.kind {
        assert!(
            matches!(&ty.node.choices[0].node.base.node, Type2::EnumTypename { name, .. }
                if name.node.as_str() == "terminal-type"),
            "expected EnumTypename(terminal-type)"
        );
    } else {
        panic!("expected type rule");
    }
}

#[test]
fn test_enum_ref_inline_group() {
    // &(group) inline enumeration as a type (RFC 8610 §3.9)
    let m = parse(r#"
        priority = &(
            low:    1,
            medium: 2,
            high:   3,
        )
    "#);
    assert_eq!(m.rules.len(), 1);
    let rule = &m.rules[0].node;
    assert_eq!(rule.name.node.as_str(), "priority");
    if let RuleKind::Type { ty, .. } = &rule.kind {
        assert!(
            matches!(&ty.node.choices[0].node.base.node, Type2::EnumGroup(_)),
            "expected EnumGroup"
        );
    } else {
        panic!("expected type rule");
    }
}

#[test]
fn test_enum_ref_in_struct_field() {
    // &typename used as the value type of a struct field (RFC 8610 §3.9)
    let m = parse(r#"
        terminal-type = (
            garden-controller: 1,
            battery-powered:   2,
            solar-powered:     3,
        )
        device = {
            id:   uint,
            kind: &terminal-type,
        }
    "#);
    assert_eq!(m.rules.len(), 2);
}

#[test]
fn test_enum_ref_inline_in_struct_field() {
    // &(group) inline enumeration used directly as a struct field type
    let m = parse(r#"
        message = {
            priority: &(low: 1, medium: 2, high: 3),
            payload:  bstr,
        }
    "#);
    assert_eq!(m.rules.len(), 1);
}

// ── Comments as documentation and source lines (dgtlrift/cddlc#8, #9) ────────

mod comments_and_sources {
    use cddlc_parser::{parse_cddl, CddlModule, MemberEntry, MemberKind, RuleKind, Type2};
    use std::path::PathBuf;

    const SRC: &str = "; File comment, cut off by the blank line.

; A reading from one sensor.
; Taken every second.
reading = {
  id: uint,          ; the sensor's id
  ; The value, in degrees Celsius.
  celsius: float64,

  ; Separated by a blank line: documents nothing.

  ; @doc \"Its unit.\"
  unit: unit-kind,
}

unit-kind = \"c\" / \"f\"   ; Celsius or Fahrenheit
; Directly above: documents `n`.
n = uint
";

    fn module() -> CddlModule {
        parse_cddl(SRC, PathBuf::from("schema/r.cddl")).unwrap().value
    }

    fn members(m: &CddlModule) -> Vec<MemberEntry> {
        let RuleKind::Type { ty, .. } = &m.rules[0].node.kind else { panic!("type rule") };
        let Type2::Map(g) = &ty.node.choices[0].node.base.node else { panic!("map") };
        g.node.choices[0].entries.iter().map(|e| e.node.clone()).collect()
    }

    #[test]
    fn rules_are_documented_by_the_block_above_and_a_same_line_comment() {
        let m = module();
        let docs: Vec<Option<&str>> = m.rules.iter().map(|r| r.node.doc.as_deref()).collect();
        assert_eq!(
            docs,
            [
                Some("A reading from one sensor.\nTaken every second."),
                Some("Celsius or Fahrenheit"),
                Some("Directly above: documents `n`."),
            ]
        );
    }

    #[test]
    fn members_are_documented_above_on_their_line_and_by_doc() {
        let docs: Vec<Option<String>> = members(&module()).into_iter().map(|e| e.doc).collect();
        assert_eq!(
            docs,
            [
                Some("the sensor's id".to_owned()),
                Some("The value, in degrees Celsius.".to_owned()),
                Some("Its unit.".to_owned()),
            ]
        );
    }

    #[test]
    fn rules_and_members_know_their_source_lines() {
        let m = module();
        let rules: Vec<(usize, String)> = m.rules.iter()
            .map(|r| { let s = r.node.source.as_ref().unwrap(); (s.line, s.text.clone()) })
            .collect();
        assert_eq!(rules[0], (5, "reading = {".to_owned()));
        assert_eq!(rules[1], (16, "unit-kind = \"c\" / \"f\"   ; Celsius or Fahrenheit".to_owned()));
        assert_eq!(rules[2].0, 18);
        let lines: Vec<(usize, String)> = members(&m).iter()
            .map(|e| { let s = e.source.as_ref().unwrap(); (s.line, s.text.clone()) })
            .collect();
        assert_eq!(
            lines,
            [
                (6, "id: uint,          ; the sensor's id".to_owned()),
                (8, "celsius: float64,".to_owned()),
                (13, "unit: unit-kind,".to_owned()),
            ]
        );
        assert_eq!(m.rules[0].node.source.as_ref().unwrap().path, PathBuf::from("schema/r.cddl"));
        let keys: Vec<bool> = members(&m).iter().map(|e| matches!(e.kind, MemberKind::KeyValue { .. })).collect();
        assert_eq!(keys, [true, true, true]);
    }
}
