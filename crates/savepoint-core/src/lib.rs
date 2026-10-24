//! Savepoint engine: finds game saves on disk, backs them up and restores them.
//!
//! Nothing in here runs in the background on its own; `savepointd` decides
//! when to call into it.

pub mod backup;
pub mod config;
pub mod fsutil;
pub mod game;
pub mod icon;
pub mod installs;
pub mod manifest;
pub mod paths;
pub mod portable;
pub mod scan;
pub mod win;

pub use config::{Config, CustomGame};
pub use game::Game;

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
