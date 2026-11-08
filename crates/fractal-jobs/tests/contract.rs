use fractal_jobs::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) {
    assert_eq!(&serde_json::from_str::<T>(&serde_json::to_string(v).unwrap()).unwrap(), v);
}
#[test]
fn request_fixtures_validate_units_and_round_trip() {
    let fixture: Value = serde_json::from_str(include_str!("requests.json")).unwrap();
    for value in fixture["valid"].as_array().unwrap() {
        let request: JobRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(request.validate(), Ok(()));
        round_trip(&request);
    }
    let units = [Unit::CpuMillicores, Unit::MemoryBytes, Unit::IoWeight, Unit::TasksMax,
        Unit::ExpectedDurationMs, Unit::RunTimeoutMs, Unit::StorageBytes];
    for (field, unit) in fixture["invalid_units"].as_array().unwrap().iter().zip(units) {
        let field = field.as_str().unwrap();
        let mut value = fixture["valid"][0].clone();
        if field == "storage_request" { value[field]["bytes"] = json!(0); }
        else { value[field] = json!(0); }
        assert_eq!(serde_json::from_value::<JobRequest>(value).unwrap().validate(), Err(RequestError::NonPositive(unit)));
        for invalid in fixture["invalid_wire"].as_array().unwrap() {
            let mut value = fixture["valid"][0].clone();
            if field == "storage_request" { value[field]["bytes"] = invalid.clone(); }
            else { value[field] = invalid.clone(); }
            // Null removes an optional timeout, rather than declaring zero duration.
            if field == "run_timeout_ms" && invalid.is_null() { continue; }
            assert!(serde_json::from_value::<JobRequest>(value).is_err(), "{field}: {invalid}");
        }
    }
}
#[derive(Deserialize)]
struct Table { states: Vec<State>, events: Vec<Event>, edges: Vec<[usize; 3]> }
#[test]
fn transition_fixture_refuses_every_invalid_edge() {
    let table: Table = serde_json::from_str(include_str!("transitions.json")).unwrap();
    let waiting = transition(State::Launching, Event::CancelRequest).unwrap();
    assert_eq!(transition(waiting, Event::Withdraw), Ok(State::Withdrawn));
    assert_eq!(transition(State::Launching, Event::Refuse { refusal: RefusalReason::HolderPrecondition }), Ok(State::Refused { refusal: RefusalReason::HolderPrecondition }));
    for (s, state) in table.states.iter().enumerate() {
        round_trip(state);
        for (e, event) in table.events.iter().enumerate() {
            round_trip(event);
            let expected = table.edges.iter().find(|edge| edge[0] == s && edge[1] == e);
            assert_eq!(transition(*state, *event), expected.map_or(
                Err(TransitionError { state: *state, event: *event }), |edge| Ok(table.states[edge[2]])));
        }
    }
}
#[test]
fn outcomes_reasons_and_per_field_coverage_round_trip() {
    for refusal in [RefusalReason::InvalidRequest, RefusalReason::Unfulfillable, RefusalReason::PolicyDenied,
        RefusalReason::HolderPrecondition, RefusalReason::ExceedsClassCap] {
        round_trip(&State::Refused { refusal });
        assert_eq!(transition(State::Queued, Event::Refuse { refusal }), Ok(State::Refused { refusal }));
    }
    for outcome in [ExitOutcome::Code(0), ExitOutcome::Code(1), ExitOutcome::Signal(9.try_into().unwrap())] {
        round_trip(&State::Completed { outcome });
        assert_eq!(transition(State::Running, Event::Exited { outcome }), Ok(State::Completed { outcome }));
    }
    for coverage in [Coverage::Measured, Coverage::Partial { reason: "advisory".into() }, Coverage::Unknown { reason: "host-loss".into() }] {
        let field = Covered { value: None, coverage };
        round_trip(&Usage { cpu_usec: field.clone(), memory_peak_bytes: field.clone(), io_read_bytes: field.clone(), io_write_bytes: field.clone() });
        let field = Covered { value: Some(false), coverage: field.coverage };
        round_trip(&Containment { cpu: field.clone(), memory: field.clone(), io: field.clone(), tasks: field });
    }
}
