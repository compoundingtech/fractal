#![allow(unused_imports)]
use crate::store::*;
use serde::Deserialize;
use smallclaims::ivm::{Contribution, Definition, View};
use crate::store::agent_view::*;
use crate::store::tests_shared::{exchange_from, receive_and_project};
use crate::store::tests_shared_seats::exchange_of;
use smallclaims::ivm::{Readiness, SourceCut, Views};

pub const SEAT: &str = "agent/grove.cedar";
