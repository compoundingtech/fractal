//! Diagnostic provider header samples, separate from authoritative request admission.
use super::*;

const RETENTION_MS: u128 = 48 * HOUR;
const MAX_SAMPLES: usize = 8_192;

/// `used`/`remaining` retain the provider's resource units: GraphQL points are not requests.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetSample {
    pub at_unix_ms: u128,
    pub resource: String,
    pub limit: u64,
    pub used: u64,
    pub remaining: u64,
    pub reset_at_unix_ms: u128,
}

#[derive(Default)]
pub(super) struct History {
    path: Option<std::path::PathBuf>,
    samples: Vec<BudgetSample>,
    pub(super) gap: Option<String>,
}

impl History {
    pub(super) fn record(&mut self, admission_path: &Path, sample: BudgetSample) -> Result<()> {
        let path = admission_path.with_extension("json.budget-history.json");
        if self.path.as_ref() != Some(&path) {
            self.samples.clear();
            self.gap = None;
            self.path = Some(path.clone());
            let loaded = (|| -> Result<Vec<BudgetSample>> {
                let Some(bytes) = read_bytes(&path)? else { return Ok(Vec::new()) };
                let samples: Vec<BudgetSample> = serde_json::from_slice(&bytes)?;
                ensure!(samples.len() <= MAX_SAMPLES, "GitHub budget sample history exceeds row bound");
                ensure!(samples.iter().all(valid_sample), "invalid GitHub budget history sample");
                ensure!(samples.windows(2).all(|pair| pair[0].at_unix_ms <= pair[1].at_unix_ms),
                    "GitHub budget history is not time-ordered");
                Ok(samples)
            })();
            match loaded {
                Ok(samples) => self.samples = samples,
                Err(error) => self.gap = Some(error.to_string()),
            }
        }
        if let Some(gap) = &self.gap {
            anyhow::bail!("GitHub budget history unavailable: {gap}");
        }
        ensure!(valid_sample(&sample), "invalid GitHub budget sample");
        // Persist the first actual header sample per resource/reset/minute. Avoid a
        // second fsync and rewriting the history for every HTTP response.
        if self.samples.iter().rev().any(|known| {
            known.resource == sample.resource
                && known.reset_at_unix_ms == sample.reset_at_unix_ms
                && known.at_unix_ms / MINUTE == sample.at_unix_ms / MINUTE
        }) {
            return Ok(());
        }
        self.samples.retain(|known| sample.at_unix_ms.saturating_sub(known.at_unix_ms) < RETENTION_MS);
        let index = self.samples.partition_point(|known| known.at_unix_ms <= sample.at_unix_ms);
        self.samples.insert(index, sample);
        if self.samples.len() > MAX_SAMPLES {
            self.samples.drain(..self.samples.len() - MAX_SAMPLES);
        }
        if let Err(error) = publish(&path, &self.samples) {
            self.gap = Some(error.to_string());
            return Err(error);
        }
        Ok(())
    }
}

fn valid_sample(sample: &BudgetSample) -> bool {
    !sample.resource.is_empty() && sample.resource.len() <= 32
        && sample.resource.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(at: u128, used: u64) -> BudgetSample {
        BudgetSample { at_unix_ms: at, resource: "graphql".into(), limit: 5_000,
            used, remaining: 5_000 - used, reset_at_unix_ms: 100 * HOUR }
    }

    #[test]
    fn budget_history_persists_minute_samples_and_retains_resource_units() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("github-admission.json");
        let mut history = History::default();
        history.record(&path, sample(HOUR, 10)).unwrap();
        history.record(&path, sample(HOUR + 1, 37)).unwrap();
        assert_eq!(history.samples.len(), 1);
        let mut restarted = History::default();
        restarted.record(&path, sample(HOUR + MINUTE, 39)).unwrap();
        assert_eq!(restarted.samples.len(), 2);
        assert_eq!(restarted.samples[0].used, 10);
        assert_eq!(restarted.samples[1].used, 39);
        assert_eq!(restarted.samples[1].resource, "graphql");
        restarted.record(&path, sample(50 * HOUR, 40)).unwrap();
        assert_eq!(restarted.samples.len(), 1);
        assert!(std::fs::read(path).is_err(), "history never creates or changes admission");
    }

    #[test]
    fn corrupt_budget_history_is_not_silently_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("github-admission.json");
        let history_path = path.with_extension("json.budget-history.json");
        std::fs::write(&history_path, b"invalid").unwrap();
        let mut history = History::default();
        assert!(history.record(&path, sample(HOUR, 10)).is_err());
        assert!(history.gap.is_some());
        assert_eq!(std::fs::read(history_path).unwrap(), b"invalid");
    }
}
