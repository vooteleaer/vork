//! vØrk storage networking over the machine's vØrk node (prototype).
//!
//! - [`protocol`]: the `/vork/get` wire format
//! - [`node`]: identities, storage's destinations, the connection to the vØrk node
//! - [`transport`]: the transport seam (ADR-0003) and its vØrk implementation
//! - [`endpoints`]: serving requests to storage's destinations
//! - [`fetch`]: local-first, batched, multi-link/multi-source fetching by read-cap
//! - [`seek`]: one `have` + `get` round for continuous seeking (ADR-0010)
//! - [`serve`]: answering `have` and `get`, apart from any transport

pub mod endpoints;
pub mod fetch;
pub mod node;
pub mod protocol;
pub mod seek;
pub mod serve;
pub mod transport;
