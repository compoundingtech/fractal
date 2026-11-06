//! Version 1 of the external quota-source contract. Percentages are not fractions.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct QuotaSnapshot {
    pub schema_version: u32,
    pub generated_at: String,
    pub freshness_horizon_seconds: u64,
    pub accounts: Vec<QuotaAccount>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct QuotaAccount {
    pub id: String,
    pub provider: String,
    pub label: String,
    pub observed_at: String,
    pub governing_window_id: Option<String>,
    pub windows: Vec<QuotaWindow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<QuotaHistory>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct QuotaWindow {
    pub id: String,
    pub label: String,
    pub used_percent: Option<f64>,
    pub resets_at: Option<String>,
}
/// At most four series per account, each with 1..=128 chronological samples.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct QuotaHistory {
    pub id: String,
    pub label: String,
    pub unit: String,
    pub points: Vec<QuotaHistoryPoint>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct QuotaHistoryPoint {
    pub at: String,
    /// A gap is unknown, not zero. The source defines each sample's bucket.
    pub value: Option<f64>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum QuotaSource {
    Disabled,
    Available { snapshot: QuotaSnapshot, refresh_seconds: u64 },
    Unavailable { refresh_seconds: u64, reason: QuotaFailure },
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaFailure { Transport, InvalidSnapshot }

impl crate::Client {
    pub async fn quota(&self) -> Result<crate::Envelope<QuotaSource>, crate::ClientError> {
        self.get("/v1/client/quota").await
    }
}
