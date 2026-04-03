//! Cisco-specific and enterprise integration tools.
//!
//! These tools connect cisco-code to enterprise communication platforms:
//! - Webex: Cisco's collaboration platform (messages, rooms, people)
//! - Slack: Team messaging (messages, channels, reactions)
//!
//! Each tool uses the platform's REST API via reqwest.
//! Authentication is via environment variables:
//! - WEBEX_TOKEN for Webex
//! - SLACK_TOKEN for Slack

pub mod webex;
pub mod slack;

pub use webex::WebexTool;
pub use slack::SlackTool;
