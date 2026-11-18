//! The scraper's SQLite report store. Only the scraper links it: the file
//! lives on the scraper's `emptyDir`, and the server reads it remotely.

mod dashboard;
mod database;
mod extractors;
mod operations;
mod report_store;
mod schema;

pub use database::Database;
