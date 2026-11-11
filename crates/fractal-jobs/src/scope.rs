//! Inactive Linux scope contract; all policy values come from configuration.
//! `io.weight` is not an IO limit and has no effect with NVMe scheduler `none`.
pub mod observation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JobScopePlan {
    memory_high: u64,
    memory_max: u64,
    tasks_max: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    NonPositive,
    HighAboveMax,
    ExceedsHeavyCaps,
    MemoryNotPageAligned,
}
impl JobScopePlan {
    /// The host page size is supplied by the caller; this crate performs no I/O.
    pub fn new(
        memory_high: u64,
        memory_max: u64,
        tasks_max: u64,
        heavy_high: u64,
        heavy_max: u64,
        page_size: u64,
    ) -> Result<Self, PlanError> {
        if [
            memory_high,
            memory_max,
            tasks_max,
            heavy_high,
            heavy_max,
            page_size,
        ]
        .contains(&0)
        {
            return Err(PlanError::NonPositive);
        }
        if memory_high > memory_max || heavy_high > heavy_max {
            return Err(PlanError::HighAboveMax);
        }
        if !memory_high.is_multiple_of(page_size) || !memory_max.is_multiple_of(page_size) {
            return Err(PlanError::MemoryNotPageAligned);
        }
        if memory_high > heavy_high || memory_max > heavy_max {
            return Err(PlanError::ExceedsHeavyCaps);
        }
        Ok(Self {
            memory_high,
            memory_max,
            tasks_max,
        })
    }
    pub fn files(&self) -> [(&'static str, u64); 4] {
        [
            ("memory.high", self.memory_high),
            ("memory.max", self.memory_max),
            ("memory.swap.max", 0),
            ("pids.max", self.tasks_max),
        ]
    }
    /// Properties only: this does not create a unit or launch any process.
    pub fn properties(&self) -> Vec<String> {
        vec![
            "Delegate=yes".into(),
            format!("MemoryHigh={}", self.memory_high),
            format!("MemoryMax={}", self.memory_max),
            "MemorySwapMax=0".into(),
            format!("TasksMax={}", self.tasks_max),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_caps_and_properties() {
        let (high, max) = (14 << 30, 16 << 30);
        let plan = JobScopePlan::new(high, max, 128, high, max, 4096).unwrap();
        assert_eq!(
            plan.properties(),
            [
                "Delegate=yes",
                "MemoryHigh=15032385536",
                "MemoryMax=17179869184",
                "MemorySwapMax=0",
                "TasksMax=128"
            ]
        );
        assert_eq!(
            JobScopePlan::new(high + 4096, max, 128, high, max, 4096),
            Err(PlanError::ExceedsHeavyCaps)
        );
        assert_eq!(
            JobScopePlan::new(high, max + 4096, 128, high, max, 4096),
            Err(PlanError::ExceedsHeavyCaps)
        );
        assert_eq!(
            JobScopePlan::new(max, high, 128, high, max, 4096),
            Err(PlanError::HighAboveMax)
        );
        assert_eq!(
            JobScopePlan::new(high, max, 0, high, max, 4096),
            Err(PlanError::NonPositive)
        );
        assert!(JobScopePlan::new(high / 2, max / 2, 64, high, max, 4096).is_ok());
    }
    #[test]
    fn memory_limits_require_host_page_alignment() {
        let (high, max) = (14 << 30, 16 << 30);
        for page_size in [4096, 65536] {
            assert!(JobScopePlan::new(high, max, 128, high, max, page_size).is_ok());
            for (high, max) in [(high + 1, max), (high, max + 1)] {
                assert_eq!(
                    JobScopePlan::new(high, max, 128, high - 1, max - 1, page_size),
                    Err(PlanError::MemoryNotPageAligned)
                );
            }
        }
        assert_eq!(
            JobScopePlan::new(high, max, 128, high, max, 0),
            Err(PlanError::NonPositive)
        );
    }
}
