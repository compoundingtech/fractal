use std::sync::Arc;
use anyhow::Result;
use fractal_light_node_core::{journal::{RunRequest, RunResult, Selection, RunRefusal}, session::RunEffects, authority::local::AuthorityChange};

pub struct StoreRunEffects { store: Arc<crate::Store> }
impl StoreRunEffects {
    pub fn new(store: Arc<crate::Store>) -> Self { Self { store } }
}
impl RunEffects for StoreRunEffects {
    fn execute(&self, request: &RunRequest) -> Result<RunResult> {
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis().try_into()?;
        let result = self.store.execute_asserted_claim(&request.assertion,&request.claim,now_ms)
            .map_err(|error| RunRefusal { code:error.code.to_string(),message:error.message });
        Ok(RunResult { request_id:request.assertion.request.request_id.clone(),result })
    }
    fn install_selection(&self, selection: &Selection) -> Result<()> {
        self.store.install_run_epoch(&selection.host,selection.epoch)?;
        Ok(())
    }
    fn close_selection(&self, selection: &Selection) -> Result<()> {
        self.store.close_run_epoch(&selection.host,selection.epoch)?;
        Ok(())
    }
    fn apply_authority(&self, change: &AuthorityChange, host_key: &str) -> Result<()> {
        // A non-program seat has native custody but deliberately has no graph run authority.
        if change.binding.run.is_some() {
            if change.revoked { self.store.revoke_run_execution(&change.binding)?; }
            else { self.store.admit_run_execution(&change.binding,host_key)?; }
        }
        Ok(())
    }
}
