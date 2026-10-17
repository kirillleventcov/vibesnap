//! vibesnap: an edit ledger for AI agents.
//!
//! Every change to the working tree becomes an event: a small git commit on
//! `refs/vibesnap/ledger` inside `.vibe/` whose tree is the state after the
//! change, indexed in SQLite by time, agent, turn and path.

pub mod cli;
pub mod commit;
pub mod config;
pub mod error;
pub mod fsutil;
pub mod hook;
pub mod paths;
pub mod refs;
pub mod store;
pub mod text;
pub mod wc;

pub use error::{AppError, Result};
