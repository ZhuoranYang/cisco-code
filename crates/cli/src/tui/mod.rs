//! Terminal User Interface using ratatui + crossterm.
//!
//! Architecture: Event loop drives `App` state, renders via ratatui `Frame`.
//! StreamEvents from the agent runtime are fed into the app's message buffer
//! and rendered in real-time.

pub mod app;
pub mod input;
pub mod render;
pub mod widgets;

pub use app::{App, AppMode};
