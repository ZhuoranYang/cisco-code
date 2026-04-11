//! Multi-channel input system.
//!
//! Channels receive messages from external sources (CLI, Webex, Slack, HTTP)
//! and convert them to a unified message format for the agent to process.
//!
//! Architecture (inspired by IronClaw):
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                       ChannelManager                        │
//! │                                                             │
//! │   ┌──────────┐   ┌────────────┐   ┌────────────┐           │
//! │   │   Repl   │   │   Webex    │   │   Slack    │   ...     │
//! │   └────┬─────┘   └─────┬──────┘   └─────┬──────┘           │
//! │        │               │               │                    │
//! │        └───────────────┴───────────────┘                    │
//! │                        │                                    │
//! │               select_all (futures)                          │
//! │                        │                                    │
//! │                        ▼                                    │
//! │                  IncomingMessage                             │
//! │                        │                                    │
//! │                        ▼                                    │
//! │              ConversationRuntime                             │
//! │                        │                                    │
//! │                        ▼                                    │
//! │              OutgoingResponse → channel.respond()            │
//! └─────────────────────────────────────────────────────────────┘
//! ```

pub mod channel;
pub mod manager;
pub mod repl;
pub mod webex;
pub mod slack;

pub use channel::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
pub use manager::ChannelManager;
pub use repl::ReplChannel;
pub use slack::{SlackChannel, SlackChannelConfig};
pub use webex::{WebexChannel, WebexChannelConfig};
