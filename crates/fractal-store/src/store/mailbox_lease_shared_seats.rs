#![allow(unused_imports)]
use crate::store::*;
use crate::mailbox::{Authority, Fence};
use crate::model::{LaunchSpec, MemberKind, MemberSpec};
use crate::store::mailbox_lease::*;
use crate::store::mailbox_lease::{ChannelLiveness, admit, admit_with_probe, channel_liveness, check_argv_declaration, check_lease_fence, lease, record};
#[cfg(any(test, feature = "test-support"))]
use crate::store::mailbox_lease::FaultReadHook;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static BEFORE_FAULT_READER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    pub static DURING_FAULT_READ: std::cell::RefCell<Option<FaultReadHook>> = const { std::cell::RefCell::new(None) };
    pub static AFTER_FAULT_ACQUISITION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    pub static CARD_FAULT_DEADLINES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Runs once, after a promotion's reader pre-check found something that may promote and
    /// before the writer is borrowed, so a test can change the lease in that gap.
    pub static AFTER_PROMOTION_PRECHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
