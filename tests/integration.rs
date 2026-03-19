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
