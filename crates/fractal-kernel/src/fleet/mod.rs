//! Fleet membership as Fractal uses it. The membership fold, joining from `fleet.toml`,
//! and transports live in `smallclaims::fleet`; keys and the pure join handshake live in
//! `fractal_auth`. See `docs/fleet-join.md` for the design.

pub use smallclaims::fleet::{Acceptance, FleetClaim, FleetView, Incarnation, MemberState, MemberView, Membership, Refusal, Window, accept};
pub use smallclaims::fleet::{file, join, membership, transport, view};
