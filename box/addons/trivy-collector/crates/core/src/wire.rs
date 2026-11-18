//! The scraper's internal API contract, shared by the scraper that serves it
//! and the server that calls it.

pub use crate::authored::constant_time_eq;

/// Header carrying the shared internal token.
pub const INTERNAL_TOKEN_HEADER: &str = "x-internal-token";

/// Prefix every internal route sits under.
pub const INTERNAL_API_PREFIX: &str = "/internal/v1";
