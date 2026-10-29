//! The st daemon's part: say which seat a process is, signed with the node key, so the gateway
//! can tell a seat from its person. The daemon finds the seat from the kernel's word: the
//! process's cgroup scope is the scope the daemon started that seat's terminal in, which the
//! terminal registry records as the `st3.scope-unit` tag beside the seat's `st3.subject`.













#[cfg(any(test, feature = "test-support"))]
pub mod shared_tests_seats;
