//! Auto-generated plugin config loader.
//!
//! `build.rs` writes the actual plugin data to `$OUT_DIR/embedded_plugins.rs`
//! during compilation. This placeholder file is tracked so that `cargo fmt`
//! can resolve the `embedded` module before `cargo build` has run.

include!(concat!(env!("OUT_DIR"), "/embedded_plugins.rs"));
