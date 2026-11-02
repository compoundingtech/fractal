//! Declared model accounts, and the account each pooled seat on this node runs on.
//!
//! The declarations are shared graph state. Which of its owner's accounts a pooled seat runs on is
//! this node's own decision, because only the node that starts a seat has the logins, so it lives in
//! a local table that no shared projection reads.

use super::*;
use crate::accounts::{AccountDecl, parse_account};

pub fn declared_accounts_at(connection: &Connection) -> Result<Vec<AccountDecl>> {
    let mut statement = connection
        .prepare_cached("SELECT subject, body FROM desired WHERE kind='account' ORDER BY subject")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut accounts = Vec::new();
    for row in rows {
        let (subject, body) = row?;
        if let Some(account) = serde_json::from_str(&body)
            .ok()
            .and_then(|desired: Value| parse_account(&subject, &desired))
        {
            accounts.push(account);
        }
    }
    Ok(accounts)
}

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

