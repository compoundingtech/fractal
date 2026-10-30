use super::*;
impl Store {
    /// The current reviewer of this exact immutable gate request, including one-shot repairs.
    pub fn human_review_reviewer(&self, request: &ClaimRecord) -> Result<String> {
        human_review_reassignment::reviewer_tx(&self.readers.get(), request)
    }
}
