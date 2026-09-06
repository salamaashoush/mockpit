//! Ferrimock CLI library: mock management and fake data command implementations.

// See the note in `ferrimock`'s lib.rs: proving the mock loader's future
// `Send` needs more solver depth than the default allows since
// nightly-2026-08-24, and the limit is per crate rather than inherited.
#![recursion_limit = "256"]

pub mod commands;
pub mod config;
pub mod ops;

// Re-export the command entry points and types for convenience
pub use commands::fake;
pub use commands::{FakeCommand, MockCommand, execute};
