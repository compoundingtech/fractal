//! The work-queue fields of an agent card: shared by the API's roster and the store's cached
//! agent rows, which compare them to decide whether a card changed.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::Store;
use crate::client_values::{client_timestamp, client_work_state};

