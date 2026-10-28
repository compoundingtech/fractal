//! The journal line for a slow request names who sent it.
use super::*;

#[test]
fn a_slow_request_line_names_the_caller_its_pid_and_the_status() {
    assert_eq!(
        slow_request_line(
            "/v1/replication/status",
            Duration::from_millis(1_517),
            "agent/invented/seat · st3 replication status",
            Some(4_242),
            StatusCode::OK,
        ),
        "st3: slow request /v1/replication/status took 1517 ms status=200 \
         caller=\"agent/invented/seat · st3 replication status\" pid=4242"
    );
    assert_eq!(
        slow_request_line(
            "/v1/replication/status",
            Duration::from_secs(5),
            "(tcp)",
            None,
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        "st3: slow request /v1/replication/status took 5000 ms status=503 caller=\"(tcp)\" pid=-"
    );
}
