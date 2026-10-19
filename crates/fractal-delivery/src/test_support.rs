#![allow(unused_imports)]
pub use st3_store::test_support::*;
pub use st3_secrets::test_support::*;
use crate::DeliveryStore;
use std::sync::OnceLock;


/// Exercise the real Store admission in isolated integration fixtures, without an API bypass
/// in the installed CLI. This module exists only for test/test-support builds.
pub fn bind_fixture_mailbox(store: &crate::store::Store, fence: &crate::mailbox::Fence)
    -> Result<crate::mailbox::Fence, crate::St3Error>
{
    store.bind_mailbox(fence)
}

pub fn check_fixture_mailbox(store: &crate::store::Store, fence: &crate::mailbox::Fence)
    -> Result<(), crate::St3Error>
{
    store.check_mailbox(fence)
}
