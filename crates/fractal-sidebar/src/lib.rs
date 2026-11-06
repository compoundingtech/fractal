//! Transport-free sidebar projection and planning over shared schema inputs.
//! Hosts supply authoritative resolved maps; this crate performs no I/O.

pub mod model;
pub mod paging;
pub mod plan;
pub mod projection;
#[cfg(feature = "ratatui")]
pub mod widget;
