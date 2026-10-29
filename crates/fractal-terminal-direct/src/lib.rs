//! Reach a terminal's PTY session without an st daemon carrying its bytes.
//!
//! A terminal on this host is its PTY session socket, proven against the incarnation st selected.
//! A terminal another fleet host owns is the same session protocol over a Fabric tunnel to that
//! host, which `st terminals expose-fabric` has Fabric forward to the owner daemon's terminal
//! socket. The local daemon only says where a terminal lives and which incarnation is running;
//! nothing here calls it.

mod local;
mod route;

pub use local::{LocalTerminal, connect_pty_session, open_local_terminal};
pub use route::{
    FabricTarget, ROUTE_LINE_LIMIT, ROUTE_TIMEOUT, RouteAnswer, RouteError, RouteRequest,
    open_route, owner_peer, protocol,
};
