//! Black-box integration tests: use the library's public API without exposing internals.
//! A single Cargo test target avoids linking a separate binary for each module.
mod backup;
mod database_worker;
mod local_storage;
mod performance;
mod persistence;
mod transfer;
