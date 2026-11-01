//! ngx: manage nginx sites and Let's Encrypt certificates.
//!
//! The binary (`src/main.rs`) is only the command line; everything else lives
//! here so it can be tested and fuzzed.

pub mod cmd;
pub mod env;
pub mod logs;
pub mod nginx;
pub mod ops;
pub mod probe;
pub mod render;
pub mod ssl;
pub mod state;
pub mod status;
pub mod tui;
pub mod validate;
