//! Prepared canonical work-summary relation, independent of effective work/authorization.
//! Installation and complete admitted-claim capture belong to the shared source owner.
//! Readers never replay history or advance the captured clock.
#![allow(dead_code)]

#[cfg(test)]
#[path = "work_summaries_ivm_tests.rs"]
mod tests;

use super::*;
use smallclaims::ivm::{Readiness, Views, source_cut};

pub const VIEW: &str = "st3.work-summaries.v1";
pub fn key(subject: &str, attempt: u32) -> Result<String> {
    Ok(serde_json::to_string(&(subject, attempt))?)
}

pub fn ready(connection: &Connection, views: &Views) -> Result<smallclaims::ivm::SourceCut> {
    let cut = source_cut(connection)?.context("work summary source unavailable")?;
    anyhow::ensure!(
        matches!(
            views.readiness(connection, VIEW, cut.epoch)?,
            Readiness::Ready(_)
        ),
        "work summaries unavailable"
    );
    // This equality is necessary, but only the complete admission/source owner can certify
    // the frontier. Computing a largest applied index does not establish hook coverage.
    anyhow::ensure!(
        cut.admitted == cut.projected && cut.projected == current_index(connection)?,
        "work summary prefix incomplete"
    );
    Ok(cut)
}
#[cfg(any(test, feature = "test-support"))]
pub mod tests_shared_seats;

#[cfg(any(test, feature = "test-support"))]
pub mod tests_shared_attention_tests;
