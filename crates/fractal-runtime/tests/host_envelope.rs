use fractal_runtime::host_envelope::{Coverage, Error, HostEnvelope, json_schema};
use serde_json::{Value, json};

const CAPPED: &[u8] = include_bytes!("host-envelope/capped.json");

#[test]
fn measured_host_shapes_are_readable_and_roundtrip() {
    for bytes in [
        CAPPED,
        include_bytes!("host-envelope/nvme.json").as_ref(),
        include_bytes!("host-envelope/small.json").as_ref(),
    ] {
        let envelope = HostEnvelope::parse(bytes).unwrap();
        assert_eq!(
            HostEnvelope::parse(&serde_json::to_vec(&envelope).unwrap()).unwrap(),
            envelope
        );
        match &envelope.mem_available_floor {
            Coverage::Enforced { value } => assert_eq!(
                value.bytes(),
                if envelope.host == "fixture-nvme" {
                    12
                } else {
                    6
                } * 1024
                    * 1024
                    * 1024
            ),
            Coverage::NotEnforced { reason } => {
                assert_eq!(envelope.host, "fixture-capped");
                assert_eq!(reason, "host-budget admission off");
            }
        }
        assert!(
            envelope
                .slices
                .iter()
                .all(|slice| slice.name != "app-capped-jobs.slice")
        );
        assert!(!envelope.slice_psi.memory.enforced() && !envelope.slice_psi.cpu.enforced());
    }
}

#[test]
fn invalid_contract_fixtures_are_refused() {
    for bytes in [
        include_bytes!("host-envelope/missing-field.json").as_ref(),
        include_bytes!("host-envelope/unknown-coverage.json").as_ref(),
        include_bytes!("host-envelope/unit-error.json").as_ref(),
        include_bytes!("host-envelope/cpu-weight-out-of-range.json").as_ref(),
        include_bytes!("host-envelope/darwin-enforced.json").as_ref(),
    ] {
        assert!(HostEnvelope::parse(bytes).is_err());
    }
    assert!(matches!(
        HostEnvelope::parse(include_bytes!("host-envelope/unknown-version.json")),
        Err(Error::UnsupportedVersion(99))
    ));
}

fn parse(value: &Value) -> Result<HostEnvelope, Error> {
    HostEnvelope::parse(&serde_json::to_vec(value).unwrap())
}

#[test]
fn coverage_is_required_for_each_resource_and_unknown_fields_are_errors() {
    let base: Value = serde_json::from_slice(CAPPED).unwrap();
    for pointer in [
        "/mem_available_floor",
        "/slice_psi/memory",
        "/slice_psi/cpu",
        "/slices/0/memory/max",
        "/slices/0/memory/high",
        "/slices/0/cpu/max",
        "/slices/0/cpu/weight",
        "/slices/0/io",
        "/slices/0/swap/max",
        "/slices/0/tasks/max",
    ] {
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        let mut value = base.clone();
        value
            .pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(parse(&value).is_err(), "missing {pointer}");
        let mut value = base.clone();
        *value.pointer_mut(pointer).unwrap() = json!({"coverage":"not-enforced","reason":"  "});
        assert!(parse(&value).is_err(), "empty reason at {pointer}");
        let mut value = base.clone();
        value.pointer_mut(pointer).unwrap()["unexpected"] = json!(true);
        assert!(parse(&value).is_err(), "unknown field at {pointer}");
    }
    let mut value = base;
    value["unexpected"] = json!(true);
    assert!(parse(&value).is_err());
}

#[test]
fn advisory_platform_cpu_weight_and_all_named_psi_windows_are_supported() {
    let mut value: Value = serde_json::from_slice(CAPPED).unwrap();
    value["slices"][0]["cpu"]["weight"] = json!({"coverage":"enforced","value":100});
    value["slices"][0]["io"] = json!({"coverage":"enforced","value":{"weight":100}});
    for window in ["avg10", "avg60", "avg300"] {
        value["slice_psi"]["cpu"] = json!({"coverage":"enforced","value":{
            "window":window,"percent":20,"target_slice":"app-capped.slice"
        }});
        assert!(parse(&value).is_ok());
    }
    fn advisory(value: &mut Value) {
        if value.get("coverage").is_some() {
            *value = json!({"coverage":"not-enforced","reason":"advisory platform; no hard per-tree cap"});
        } else {
            match value {
                Value::Object(fields) => fields.values_mut().for_each(advisory),
                Value::Array(items) => items.iter_mut().for_each(advisory),
                _ => {}
            }
        }
    }
    advisory(&mut value);
    value["platform"] = json!("darwin-advisory");
    assert!(parse(&value).is_ok());
    for (field, resource) in [
        (
            "max",
            json!({"quota_micros":2000000,"period_micros":100000}),
        ),
        ("weight", json!(20)),
    ] {
        let mut enforced_cpu = value.clone();
        enforced_cpu["slices"][0]["cpu"][field] = json!({"coverage":"enforced","value":resource});
        assert!(
            parse(&enforced_cpu).is_err(),
            "Darwin cannot enforce CPU {field}"
        );
    }
    value["slices"][0]["io"] = json!({"coverage":"enforced","value":{"weight":100}});
    assert!(parse(&value).is_err());
}

#[test]
fn shared_size_overflow_and_invalid_targets_are_refused() {
    let mut value: Value = serde_json::from_slice(CAPPED).unwrap();
    value["mem_available_floor"] = json!({"coverage":"enforced","value":"18446744073709551615T"});
    assert!(parse(&value).is_err());
    value["mem_available_floor"] = json!({"coverage":"enforced","value":"16G"});
    value["slice_psi"]["memory"] = json!({"coverage":"enforced","value":{
        "window":"avg60","percent":10,"target_slice":"unknown.slice"
    }});
    assert!(parse(&value).is_err());
}

#[test]
fn enforced_memory_high_cannot_exceed_max_in_bytes() {
    let mut value: Value = serde_json::from_slice(CAPPED).unwrap();
    value["slices"][0]["memory"]["max"] = json!({"coverage":"enforced","value":"1G"});
    value["slices"][0]["memory"]["high"] = json!({"coverage":"enforced","value":"1025M"});
    assert!(matches!(parse(&value), Err(Error::Invalid(reason))
        if reason == "memory.high must not exceed memory.max"));
    value["slices"][0]["memory"]["high"]["value"] = json!("1024M");
    assert!(parse(&value).is_ok());
    value["slices"][0]["memory"]["high"] = json!({
        "coverage":"not-enforced","reason":"no configured soft limit"
    });
    assert!(parse(&value).is_ok());
}

#[test]
fn cpu_max_and_weight_apply_independently_with_strict_ranges() {
    let bytes = include_bytes!("host-envelope/both-cpu-fields.json");
    let envelope = HostEnvelope::parse(bytes).unwrap();
    assert!(envelope.slices[0].cpu.max.enforced());
    assert!(envelope.slices[0].cpu.weight.enforced());
    let base: Value = serde_json::from_slice(bytes).unwrap();
    for weight in [0, 10001] {
        let mut value = base.clone();
        value["slices"][0]["cpu"]["weight"]["value"] = json!(weight);
        assert!(parse(&value).is_err());
    }
    for weight in [1, 10000] {
        let mut value = base.clone();
        value["slices"][0]["cpu"]["weight"]["value"] = json!(weight);
        assert!(parse(&value).is_ok());
    }
    for field in ["quota_micros", "period_micros"] {
        let mut value = base.clone();
        value["slices"][0]["cpu"]["max"]["value"][field] = json!(0);
        assert!(parse(&value).is_err());
    }
}

#[test]
fn checked_in_schema_equals_generated_export() {
    let checked_in: Value =
        serde_json::from_str(include_str!("../../../fixtures/host-envelope.schema.json")).unwrap();
    assert_eq!(checked_in, json_schema());
}
