use super::*;

impl Store {
    #[cfg(any(test, feature = "test-support"))]
    pub fn count_refolded_cards_for_test(&self, cards: usize) {
        self.smalltalk.agent_resources_refolded_cards.fetch_add(cards, std::sync::atomic::Ordering::Relaxed);
        self.smalltalk.agent_resources_largest_fold.fetch_max(cards, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn agent_resources_refolded_cards_for_test(&self) -> usize {
        self.smalltalk.agent_resources_refolded_cards.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn usage_summary_from_rows<I>(
        rows: I,
        incarnation: Option<&str>,
    ) -> Result<Option<UsageSummary>>
    where
        I: IntoIterator<Item = rusqlite::Result<(u64, String, String)>>,
    {
        let mut fold = UsageFold::default();
        for row in rows {
            let (store_index, body, accepted_at) = row?;
            fold.apply(store_index, &body, &accepted_at, incarnation)?;
        }
        Ok(fold.summary())
    }

}

