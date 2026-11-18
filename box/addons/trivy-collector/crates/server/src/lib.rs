//! The server: web UI, REST API, MCP server, and authentication. Stateless,
//! it reads reports from the scraper over the internal API.

pub mod auth;
pub mod config;
pub mod mcp;
pub mod metrics;
pub mod notes;
pub mod remote;
pub mod token_store;
pub mod web;
