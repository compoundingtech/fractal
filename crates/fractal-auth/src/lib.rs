//! Fleet authentication primitives shared by graph-backed and graph-free runtimes.

#![forbid(unsafe_code)]

pub mod code;
pub mod handshake;
mod http;
mod keys;
mod route;

pub use http::FleetAuth;
pub use keys::{MemberKey, claim_signature_message, envelope_signature_message, envelope_signature_message_in, verify_signature};
pub use route::{is_permitted_route_address, is_tailnet_address};

/// What a node knows about the sender of one signed message.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Sender {
    pub name: String,
    /// The member key the message presented, if any.
    pub member_key: Option<String>,
    /// Whether the member signature over the message verified against that key.
    pub member_signature_valid: bool,
}
