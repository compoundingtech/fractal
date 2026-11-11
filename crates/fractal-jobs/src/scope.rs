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
pub enum PlanError { NonPositive, HighAboveMax, ExceedsHeavyCaps }
impl JobScopePlan {
    pub fn new(memory_high: u64, memory_max: u64, tasks_max: u64,
        heavy_high: u64, heavy_max: u64) -> Result<Self, PlanError> {
        if [memory_high, memory_max, tasks_max, heavy_high, heavy_max].contains(&0) {
            return Err(PlanError::NonPositive);
        }
        if memory_high > memory_max || heavy_high > heavy_max {
            return Err(PlanError::HighAboveMax);
        }
        if memory_high > heavy_high || memory_max > heavy_max {
            return Err(PlanError::ExceedsHeavyCaps);
        }
        Ok(Self { memory_high, memory_max, tasks_max })
    }
    pub fn files(&self) -> [(&'static str, u64); 4] {
        [("memory.high", self.memory_high), ("memory.max", self.memory_max),
            ("memory.swap.max", 0), ("pids.max", self.tasks_max)]
    }
    /// Properties only: this does not create a unit or launch any process.
    pub fn properties(&self) -> Vec<String> {
        vec!["Delegate=yes".into(), format!("MemoryHigh={}", self.memory_high),
            format!("MemoryMax={}", self.memory_max), "MemorySwapMax=0".into(),
            format!("TasksMax={}", self.tasks_max)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_caps_and_properties() {
        let (high, max) = (14 << 30, 16 << 30);
        let plan = JobScopePlan::new(high, max, 128, high, max).unwrap();
        assert_eq!(plan.properties(), ["Delegate=yes", "MemoryHigh=15032385536",
            "MemoryMax=17179869184", "MemorySwapMax=0", "TasksMax=128"]);
        assert_eq!(JobScopePlan::new(high + 1, max, 128, high, max), Err(PlanError::ExceedsHeavyCaps));
        assert_eq!(JobScopePlan::new(high, max + 1, 128, high, max), Err(PlanError::ExceedsHeavyCaps));
        assert_eq!(JobScopePlan::new(max, high, 128, high, max), Err(PlanError::HighAboveMax));
        assert_eq!(JobScopePlan::new(high, max, 0, high, max), Err(PlanError::NonPositive));
        assert!(JobScopePlan::new(high / 2, max / 2, 64, high, max).is_ok());
    }
}
