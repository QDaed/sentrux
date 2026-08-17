//! Sentrux binary library — CLI/GUI entry point reused by optional Pro crates.
//!
//! Architecture: `sentrux_bin::run()` initializes license state and starts the
//! selected mode. With the default `pro` feature, built-in Pro capabilities
//! are registered at startup so the public binary ships the full feature set.

mod main_impl;
pub use main_impl::run;
