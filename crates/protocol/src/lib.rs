//! cisco-code-protocol: Shared types and event models
//!
//! This crate defines the core data structures used across all cisco-code crates.
//! Design insight: Following Claw-Code-Parity's approach of a shared protocol crate
//! that all other crates depend on, ensuring type consistency.

pub mod messages;
pub mod events;
pub mod tools;
pub mod errors;

pub use messages::*;
pub use events::*;
pub use tools::*;
