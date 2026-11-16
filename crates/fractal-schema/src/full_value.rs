//! Claim families whose every claim carries the complete value for its logical key.
//!
//! In a full-value family, the newest claim of a key in canonical order is the key's whole
//! value: every reader answers from it alone, so an older claim of the same key contributes
//! nothing. That is what later lets a member retire the older claims locally without changing
//! any answer (migration step 4 of the trim design; removal itself is a later, separate step).
//!
//! A declaration here has no runtime effect. It removes nothing and adds no checkpoint rule:
//! `retention.toml` stays the checkpoint policy, and its rules digest is unchanged. It is the
//! contract that each family's readers are tested against, and the list a local trim reads
//! once removal for a family is approved.

use crate::Cardinality;

/// One part of a full-value family's logical key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPart {
    /// The claim's subject.
    Subject,
    /// The writer that observed the value. Readers that do not distinguish writers still read
    /// the canonical newest of all writers' newest values, so keeping each writer's newest keeps
    /// theirs too.
    Origin,
}

#[derive(Debug)]
pub struct FullValue {
    pub kind: &'static str,
    pub key: &'static [KeyPart],
    /// Why every claim is complete: for a state transition, a field a claim leaves out is
    /// cleared, so the claim never inherits an older claim's field.
    pub cardinality: Cardinality,
}

/// The declared full-value families.
pub const FAMILIES: &[FullValue] = &[
    // Written by the member that observes a subscription, on every status change (missions
    // store). Readers: attention's target state, the actual-state fold, and the observer and
    // subscription list and detail, each the canonical newest claim at the read cut.
    FullValue {
        kind: "subscription.state",
        key: &[KeyPart::Subject, KeyPart::Origin],
        cardinality: Cardinality::StateTransition,
    },
];

/// The declaration for `kind`, if its family is full-value.
pub fn family(kind: &str) -> Option<&'static FullValue> {
    FAMILIES.iter().find(|family| family.kind == kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_is_a_registered_state_transition_keyed_by_subject() {
        for family in FAMILIES {
            let registry = crate::registry();
            let spec = registry
                .claims
                .get(family.kind)
                .unwrap_or_else(|| panic!("{} is not a registered kind", family.kind));
            // Only a state transition clears the fields a newer claim leaves out. An appending
            // kind would let an older claim's field survive, so its claims are not complete.
            assert_eq!(family.cardinality, Cardinality::StateTransition, "{}", family.kind);
            assert_eq!(spec.cardinality, family.cardinality, "{}", family.kind);
            assert_eq!(family.key.first(), Some(&KeyPart::Subject), "{}", family.kind);
            assert_eq!(
                FAMILIES.iter().filter(|other| other.kind == family.kind).count(),
                1,
                "{} is declared once",
                family.kind
            );
        }
    }

    #[test]
    fn a_full_value_family_has_no_checkpoint_rule_yet() {
        // Declaring a family is not approval to remove its claims: removal is its own step.
        for family in FAMILIES {
            assert_eq!(
                crate::retention::policy().rules_for(family.kind).count(),
                0,
                "{} has a checkpoint rule",
                family.kind
            );
        }
    }
}
