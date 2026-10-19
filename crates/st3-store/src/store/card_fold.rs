//! The agents roster reduces every agent's status at one snapshot. One subject at a time, that
//! costs a dozen statements per agent; here each chunk of subjects reads what the same
//! reductions need in a few statements, and the shared folds reduce what it read.


#[path = "harness_health_capture.rs"]
#[cfg(any(test, feature = "test-support"))]
pub mod health_capture;

#[cfg(any(test, feature = "test-support"))]
pub mod health_capture_tests_shared_attention_tests;
