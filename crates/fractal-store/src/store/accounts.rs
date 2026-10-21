//! Declared model accounts, and the account each pooled seat on this node runs on.
//!
//! The declarations are shared graph state. Which of its owner's accounts a pooled seat runs on is
//! this node's own decision, because only the node that starts a seat has the logins, so it lives in
//! a local table that no shared projection reads.

use super::*;


impl Store {
    /// The account a pooled seat on this node was last placed on.
    pub fn seat_account_choice(&self, seat: &str) -> Result<Option<String>> {
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

