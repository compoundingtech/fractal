use crate::{Client, ClientError, percent_encode_segment};
use fractal_schema::task_grants::{CreateTaskGrantRequest, RevokeTaskGrant, TaskGrantReceipt, TaskGrantRecord};

impl Client {
    /// Read Q336 grant state, including window expiry and revocation.
    pub async fn task_grant(&self, id: &str) -> Result<TaskGrantRecord, ClientError> {
        self.get(&format!("/v1/task-grants/{}", percent_encode_segment(id))).await
    }

    /// Remains dark until P3 supplies the op-proxy kernel identity binding.
    pub async fn create_task_grant(&self, input: &CreateTaskGrantRequest) -> Result<TaskGrantReceipt, ClientError> {
        self.post("/v1/task-grants", input).await
    }

    /// Idempotent; remains dark until P3 binds the caller's identity.
    pub async fn revoke_task_grant(&self, id: &str, input: &RevokeTaskGrant) -> Result<TaskGrantRecord, ClientError> {
        self.post(&format!("/v1/task-grants/{}/revoke", percent_encode_segment(id)), input).await
    }
}
