#![allow(unused_imports)]
use crate::store::*;
use crate::graph::parse_test_intent;

pub const FLEET: &str = "5b4b1a8e-7c3d-4e2f-9a1b-0c6d5e4f3a2b";

pub const PERSON: &str = "person/avery";

/// SplitMix64: small, seeded, and the same on every platform.
pub struct Rng(pub u64);

impl Rng {
    #[allow(clippy::should_implement_trait)] // was private before the cut moved its callers up
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    pub fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// How one exchange reaches its receiver.
#[derive(Clone, Copy, Debug)]
pub enum Delivery {
    /// As one exchange.
    Whole,
    /// The newest envelope first, then the rest, as out-of-order history arrives.
    NewestFirst,
    /// In random pieces, in random order.
    Scattered,
    /// The receiver stores the envelopes and stops before it admits them.
    CrashAfterReceipt,
    /// The receiver admits the claims and stops before it projects them.
    CrashAfterAdmission,
}

pub fn mission_source(revision: usize) -> String {
    format!(
        r#"version 2

mission "sim" state="ready" {{
  goal "Converge revision {revision}."
  step "prepare" {{ assigned-to "agent/sim.worker" }}
  step "check" {{ assigned-to "agent/sim.worker"; depends-on {{ step "prepare" completed }} }}
  step "probe" {{ goal "Probe revision {revision}."; assigned-to "agent/sim.worker"; depends-on {{ step "prepare" completed }} }}
  step "announce" {{ assigned-to "agent/sim.worker"; depends-on {{ step "check" completed }} }}
  finally {{ step "report" {{ agentless }} }}
}}
"#
    )
}

pub fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
