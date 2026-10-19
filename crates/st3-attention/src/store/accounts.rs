//! The part of `store::accounts` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;

pub trait AccountsAttentionStore {
    /// The account a pooled seat on this node was last placed on.
    fn seat_account_choice(&self, seat: &str) -> Result<Option<String>>;
}

impl AccountsAttentionStore for Store {

    fn seat_account_choice(&self, seat: &str) -> Result<Option<String>> {
        self.readers
            .get()
            .query_row(
                "SELECT account FROM local_seat_accounts WHERE seat=?1",
                [seat],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }
}

