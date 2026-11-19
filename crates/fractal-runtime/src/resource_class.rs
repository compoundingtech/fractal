//! Relative CPU/IO service weights for finite work, independent of declared CPU quotas.
//! These are scheduling weights, not extra admission capacity or learned resource estimates.
use crate::lane::RequestClass;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Weights {
    pub cpu: u16,
    pub io: u16,
}

/// Keep a tenfold separation between the existing classes. Linux weights are relative among
/// runnable siblings, so unused service remains work-conserving rather than a fixed partition.
pub fn weights(class: &RequestClass) -> anyhow::Result<Weights> {
    Ok(match class {
        RequestClass::Recovery | RequestClass::Landing => Weights { cpu: 1000, io: 1000 },
        RequestClass::Check => Weights { cpu: 100, io: 100 },
        RequestClass::Agent => Weights { cpu: 10, io: 10 },
        RequestClass::Unknown(value) => anyhow::bail!("unsupported resource class {value}; upgrade the execution host"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_weights_are_strictly_ordered_without_changing_core_quota() {
        let landing = weights(&RequestClass::Landing).unwrap();
        assert_eq!(weights(&RequestClass::Recovery).unwrap(), landing);
        let check = weights(&RequestClass::Check).unwrap();
        let agent = weights(&RequestClass::Agent).unwrap();
        assert!(landing.cpu > check.cpu && check.cpu > agent.cpu);
        assert!(landing.io > check.io && check.io > agent.io);
        for class in [RequestClass::Recovery, RequestClass::Landing, RequestClass::Check, RequestClass::Agent] {
            let value = weights(&class).unwrap();
            assert!((1..=10000).contains(&value.cpu));
            assert!((1..=10000).contains(&value.io));
        }
        assert!(weights(&RequestClass::Unknown("future-class".into())).is_err());
    }
}
