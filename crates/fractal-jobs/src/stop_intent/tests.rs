use super::*;

fn requester() -> StopOrigin<'static> {
    StopOrigin::Requester { actor: Some("person/fixture") }
}

#[test]
fn requester_authorization_fails_closed_without_an_operator_override() {
    for actor in [None, Some("")] {
        assert_eq!(
            decide(
                State::Queued, "person/fixture", StopOrigin::Requester { actor }, StopInput::Cancel,
            ),
            Err(StopRefusal::RequesterUnresolved),
        );
    }
    assert_eq!(
        decide(State::Queued, "person/fixture",
            StopOrigin::Requester { actor: Some("person/operator") }, StopInput::Cancel),
        Err(StopRefusal::NotRequester),
    );
}

#[test]
fn cancel_records_only_valid_idempotent_state_machine_edges() {
    for (state, event) in [
        (State::Queued, Event::Withdraw),
        (State::Running, Event::CancelRequest),
    ] {
        let result = decide(state, "person/fixture", requester(), StopInput::Cancel).unwrap();
        assert_eq!(result, StopDecision::Record { event, reason: StopReason::RequesterCancelled });
        let next = crate::transition(state, event).unwrap();
        assert_eq!(decide(next, "person/fixture", requester(), StopInput::Cancel),
            Ok(StopDecision::Unchanged));
    }
    assert_eq!(decide(State::Completed { outcome: crate::ExitOutcome::Code(0) },
        "person/fixture", requester(), StopInput::Cancel), Err(StopRefusal::JobTerminal));
}

#[test]
fn expiry_withdraws_only_queued_and_disconnect_never_cancels() {
    assert_eq!(
        decide(State::Queued, "person/fixture", requester(), StopInput::RequesterLeaseExpired),
        Ok(StopDecision::Record {
            event: Event::Withdraw,
            reason: StopReason::RequesterLeaseExpired,
        }),
    );
    for state in [State::Queued, State::Running] {
        for input in [StopInput::RequesterLeaseExpired, StopInput::WaitDisconnected] {
            if state == State::Running {
                assert_eq!(decide(state, "person/fixture", requester(), input),
                    Ok(StopDecision::KeepRunningWithDeclaredMaxHoldOrDeadline));
            } else if matches!(input, StopInput::WaitDisconnected) {
                assert_eq!(decide(state, "person/fixture", requester(), input),
                    Ok(StopDecision::Unchanged));
            }
        }
    }
}

#[test]
fn orphaned_running_jobs_remain_subject_to_daemon_max_hold() {
    let orphan = decide(
        State::Running, "person/fixture", requester(), StopInput::RequesterLeaseExpired,
    );
    assert_eq!(orphan, Ok(StopDecision::KeepRunningWithDeclaredMaxHoldOrDeadline));
    let origin = StopOrigin::DaemonLimit { reason: LimitReason::MaxHold };
    assert_eq!(decide(State::Running, "person/fixture", origin, StopInput::Cancel),
        Ok(StopDecision::Record { event: Event::CancelRequest, reason: StopReason::MaxHold }));
    assert_eq!(serde_json::to_value(StopReason::MaxHold).unwrap(), "max-hold");
    assert_eq!(serde_json::to_value(StopReason::RequesterLeaseExpired).unwrap(),
        "requester-lease-expired");
}
