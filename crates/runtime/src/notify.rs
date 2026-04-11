//! Notification system for alerting users about agent events.
//!
//! Supports multiple channels:
//! - Webhook: POST JSON to a URL (generic, works with anything)
//! - Slack: Send messages via Slack incoming webhook or API
//! - Webex: Send messages via Webex incoming webhook
//! - Console: Print to stderr (for local CLI use)
//! - TerminalBell: Emit BEL character to stderr (audible bell)
//! - OsNotification: Native OS notification (macOS/Linux)
//!
//! Notifications are fire-and-forget — failures are logged but don't
//! block the agent.

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::hooks::{HookEvent, HookInput, HookResult, HookRunner};

/// A notification to send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// Notification title/subject.
    pub title: String,
    /// Notification body (markdown supported for Slack/Webex).
    pub body: String,
    /// Severity level.
    pub level: NotificationLevel,
    /// Source session ID.
    pub session_id: Option<String>,
    /// Additional metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Severity levels for notifications.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// Configuration for a notification channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum NotificationChannel {
    /// Generic webhook — POST JSON to a URL.
    #[serde(rename = "webhook")]
    Webhook {
        url: String,
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
    },
    /// Slack incoming webhook.
    #[serde(rename = "slack")]
    Slack {
        webhook_url: String,
        #[serde(default)]
        channel: Option<String>,
    },
    /// Webex incoming webhook.
    #[serde(rename = "webex")]
    Webex {
        webhook_url: String,
        #[serde(default)]
        room_id: Option<String>,
    },
    /// Console output (stderr).
    #[serde(rename = "console")]
    Console,
    /// Terminal bell — emits BEL character (\x07) to stderr.
    #[serde(rename = "terminal_bell")]
    TerminalBell,
    /// Native OS notification (macOS `osascript`, Linux `notify-send`).
    #[serde(rename = "os_notification")]
    OsNotification {
        /// Optional sound name (macOS only).
        #[serde(default)]
        sound: Option<String>,
    },
}

/// The notification dispatcher — sends notifications to configured channels.
pub struct Notifier {
    channels: Vec<NotificationChannel>,
    http: reqwest::Client,
    /// Filter: minimum level to send. Notifications below this level are dropped.
    min_level: NotificationLevel,
    /// Optional hook runner for firing Notification hooks before dispatch.
    hooks: Option<Arc<HookRunner>>,
}

impl Notifier {
    pub fn new() -> Self {
        Self {
            channels: Vec::new(),
            http: reqwest::Client::new(),
            min_level: NotificationLevel::Info,
            hooks: None,
        }
    }

    /// Attach a hook runner so Notification hooks fire before dispatch.
    pub fn with_hooks(mut self, hooks: Arc<HookRunner>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Add a notification channel.
    pub fn add_channel(&mut self, channel: NotificationChannel) {
        self.channels.push(channel);
    }

    /// Set minimum notification level.
    pub fn set_min_level(&mut self, level: NotificationLevel) {
        self.min_level = level;
    }

    /// Send a notification to all configured channels.
    ///
    /// Failures are logged but don't propagate — notifications are best-effort.
    /// If a Notification hook is configured and returns Suppress, the notification
    /// is silently dropped.
    pub async fn send(&self, notification: &Notification) {
        if !self.should_send(&notification.level) {
            return;
        }

        // Fire the Notification hook before dispatching to channels.
        if let Some(ref hooks) = self.hooks {
            let hook_input = HookInput {
                event: HookEvent::Notification,
                session_id: notification.session_id.clone().unwrap_or_default(),
                tool_name: None,
                tool_input: None,
                tool_result: None,
                is_error: None,
                subagent_id: None,
                stop_reason: None,
                notification: Some(serde_json::json!({
                    "title": notification.title,
                    "body": notification.body,
                    "level": notification.level,
                })),
                file_path: None,
                file_operation: None,
                prompt: None,
                summary_tokens: None,
            };
            match hooks.run(&hook_input).await {
                HookResult::Suppress { message } => {
                    tracing::debug!("Notification suppressed by hook: {message}");
                    return;
                }
                _ => {} // Continue with sending
            }
        }

        for channel in &self.channels {
            if let Err(e) = self.send_to_channel(channel, notification).await {
                tracing::warn!("Notification failed: {e}");
            }
        }
    }

    /// Check if a notification level meets the minimum threshold.
    fn should_send(&self, level: &NotificationLevel) -> bool {
        level_priority(level) >= level_priority(&self.min_level)
    }

    /// Send to a specific channel.
    async fn send_to_channel(
        &self,
        channel: &NotificationChannel,
        notification: &Notification,
    ) -> Result<()> {
        match channel {
            NotificationChannel::Webhook { url, headers } => {
                let mut req = self
                    .http
                    .post(url)
                    .header("content-type", "application/json")
                    .json(notification);

                for (k, v) in headers {
                    req = req.header(k.as_str(), v.as_str());
                }

                let resp = req.send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    tracing::warn!("Webhook notification returned {status}");
                }
            }

            NotificationChannel::Slack { webhook_url, channel } => {
                let emoji = match notification.level {
                    NotificationLevel::Info => ":information_source:",
                    NotificationLevel::Success => ":white_check_mark:",
                    NotificationLevel::Warning => ":warning:",
                    NotificationLevel::Error => ":x:",
                };

                let mut payload = serde_json::json!({
                    "text": format!("{emoji} *{}*\n{}", notification.title, notification.body),
                });

                if let Some(ch) = channel {
                    payload["channel"] = serde_json::json!(ch);
                }

                let resp = self
                    .http
                    .post(webhook_url)
                    .header("content-type", "application/json")
                    .json(&payload)
                    .send()
                    .await?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    anyhow::bail!("Slack webhook error {status}: {text}");
                }
            }

            NotificationChannel::Webex { webhook_url, room_id } => {
                let markdown = format!("**{}**\n\n{}", notification.title, notification.body);

                let mut payload = serde_json::json!({
                    "markdown": markdown,
                });

                if let Some(rid) = room_id {
                    payload["roomId"] = serde_json::json!(rid);
                }

                let resp = self
                    .http
                    .post(webhook_url)
                    .header("content-type", "application/json")
                    .json(&payload)
                    .send()
                    .await?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    anyhow::bail!("Webex webhook error {status}: {text}");
                }
            }

            NotificationChannel::Console => {
                let prefix = match notification.level {
                    NotificationLevel::Info => "[info]",
                    NotificationLevel::Success => "[done]",
                    NotificationLevel::Warning => "[warn]",
                    NotificationLevel::Error => "[error]",
                };
                eprintln!("{prefix} {} — {}", notification.title, notification.body);
            }

            NotificationChannel::TerminalBell => {
                eprint!("\x07");
            }

            NotificationChannel::OsNotification { sound } => {
                self.send_os_notification(&notification.title, &notification.body, sound.as_deref())
                    .await?;
            }
        }

        Ok(())
    }

    /// Send a native OS notification (platform-specific).
    async fn send_os_notification(
        &self,
        title: &str,
        body: &str,
        sound: Option<&str>,
    ) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let sound_clause = match sound {
                Some(s) => format!(" sound name \"{}\"", s.replace('"', "\\\"")),
                None => String::new(),
            };
            let script = format!(
                "display notification \"{}\" with title \"{}\"{}",
                body.replace('"', "\\\""),
                title.replace('"', "\\\""),
                sound_clause,
            );
            let status = tokio::process::Command::new("osascript")
                .args(["-e", &script])
                .status()
                .await?;
            if !status.success() {
                tracing::warn!("osascript exited with {status}");
            }
            return Ok(());
        }

        #[cfg(target_os = "linux")]
        {
            let _ = sound; // sound is macOS-only
            let status = tokio::process::Command::new("notify-send")
                .args([title, body])
                .status()
                .await?;
            if !status.success() {
                tracing::warn!("notify-send exited with {status}");
            }
            return Ok(());
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = (title, body, sound);
            tracing::warn!("OS notifications not supported on this platform");
            Ok(())
        }
    }

    /// Number of configured channels.
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Convenience: send a success notification.
    pub async fn notify_success(&self, title: &str, body: &str, session_id: Option<&str>) {
        self.send(&Notification {
            title: title.into(),
            body: body.into(),
            level: NotificationLevel::Success,
            session_id: session_id.map(|s| s.into()),
            metadata: serde_json::Value::Null,
        })
        .await;
    }

    /// Convenience: send an error notification.
    pub async fn notify_error(&self, title: &str, body: &str, session_id: Option<&str>) {
        self.send(&Notification {
            title: title.into(),
            body: body.into(),
            level: NotificationLevel::Error,
            session_id: session_id.map(|s| s.into()),
            metadata: serde_json::Value::Null,
        })
        .await;
    }
}

impl Default for Notifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Map level to numeric priority for filtering.
fn level_priority(level: &NotificationLevel) -> u8 {
    match level {
        NotificationLevel::Info => 0,
        NotificationLevel::Success => 1,
        NotificationLevel::Warning => 2,
        NotificationLevel::Error => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notifier_empty() {
        let notifier = Notifier::new();
        assert_eq!(notifier.channel_count(), 0);
    }

    #[test]
    fn test_add_channel() {
        let mut notifier = Notifier::new();
        notifier.add_channel(NotificationChannel::Console);
        assert_eq!(notifier.channel_count(), 1);
    }

    #[test]
    fn test_level_filtering() {
        let mut notifier = Notifier::new();
        notifier.set_min_level(NotificationLevel::Warning);

        assert!(!notifier.should_send(&NotificationLevel::Info));
        assert!(!notifier.should_send(&NotificationLevel::Success));
        assert!(notifier.should_send(&NotificationLevel::Warning));
        assert!(notifier.should_send(&NotificationLevel::Error));
    }

    #[test]
    fn test_level_priority() {
        assert!(level_priority(&NotificationLevel::Error) > level_priority(&NotificationLevel::Warning));
        assert!(level_priority(&NotificationLevel::Warning) > level_priority(&NotificationLevel::Success));
        assert!(level_priority(&NotificationLevel::Success) > level_priority(&NotificationLevel::Info));
    }

    #[test]
    fn test_notification_serialization() {
        let notif = Notification {
            title: "Job Complete".into(),
            body: "Your agent finished analyzing the codebase".into(),
            level: NotificationLevel::Success,
            session_id: Some("sess-abc".into()),
            metadata: serde_json::json!({"turns": 5}),
        };

        let json = serde_json::to_value(&notif).unwrap();
        assert_eq!(json["title"], "Job Complete");
        assert_eq!(json["level"], "success");
        assert_eq!(json["metadata"]["turns"], 5);
    }

    #[test]
    fn test_channel_config_webhook() {
        let json = r#"{"type": "webhook", "url": "https://example.com/hook", "headers": {"X-Token": "abc"}}"#;
        let channel: NotificationChannel = serde_json::from_str(json).unwrap();
        assert!(matches!(channel, NotificationChannel::Webhook { .. }));
    }

    #[test]
    fn test_channel_config_slack() {
        let json = r##"{"type": "slack", "webhook_url": "https://hooks.slack.com/services/T/B/x", "channel": "#alerts"}"##;
        let channel: NotificationChannel = serde_json::from_str(json).unwrap();
        assert!(matches!(channel, NotificationChannel::Slack { .. }));
    }

    #[test]
    fn test_channel_config_webex() {
        let json = r#"{"type": "webex", "webhook_url": "https://webexapis.com/v1/webhooks/incoming/abc"}"#;
        let channel: NotificationChannel = serde_json::from_str(json).unwrap();
        assert!(matches!(channel, NotificationChannel::Webex { .. }));
    }

    #[tokio::test]
    async fn test_console_notification() {
        let mut notifier = Notifier::new();
        notifier.add_channel(NotificationChannel::Console);

        // This just prints to stderr — no assertion needed, just verify no panic
        notifier
            .send(&Notification {
                title: "Test".into(),
                body: "test body".into(),
                level: NotificationLevel::Info,
                session_id: None,
                metadata: serde_json::Value::Null,
            })
            .await;
    }

    #[tokio::test]
    async fn test_notify_convenience_methods() {
        let mut notifier = Notifier::new();
        notifier.add_channel(NotificationChannel::Console);

        notifier.notify_success("Done", "All good", Some("sess-1")).await;
        notifier.notify_error("Failed", "Something broke", None).await;
    }

    #[test]
    fn test_channel_config_terminal_bell() {
        let json = r#"{"type": "terminal_bell"}"#;
        let channel: NotificationChannel = serde_json::from_str(json).unwrap();
        assert!(matches!(channel, NotificationChannel::TerminalBell));
    }

    #[test]
    fn test_channel_config_os_notification() {
        let json = r#"{"type": "os_notification"}"#;
        let channel: NotificationChannel = serde_json::from_str(json).unwrap();
        assert!(matches!(
            channel,
            NotificationChannel::OsNotification { sound: None }
        ));
    }

    #[test]
    fn test_channel_config_os_notification_with_sound() {
        let json = r#"{"type": "os_notification", "sound": "Glass"}"#;
        let channel: NotificationChannel = serde_json::from_str(json).unwrap();
        match channel {
            NotificationChannel::OsNotification { sound } => {
                assert_eq!(sound.as_deref(), Some("Glass"));
            }
            _ => panic!("expected OsNotification"),
        }
    }

    #[tokio::test]
    async fn test_terminal_bell_notification() {
        let mut notifier = Notifier::new();
        notifier.add_channel(NotificationChannel::TerminalBell);

        // Should complete without panic — BEL goes to stderr
        notifier
            .send(&Notification {
                title: "Ping".into(),
                body: "bell test".into(),
                level: NotificationLevel::Info,
                session_id: None,
                metadata: serde_json::Value::Null,
            })
            .await;
    }

    #[tokio::test]
    async fn test_os_notification_send() {
        let mut notifier = Notifier::new();
        notifier.add_channel(NotificationChannel::OsNotification { sound: None });

        // Best-effort: on CI / unsupported platforms this logs a warning but doesn't panic
        notifier
            .send(&Notification {
                title: "OS test".into(),
                body: "os notification body".into(),
                level: NotificationLevel::Success,
                session_id: None,
                metadata: serde_json::Value::Null,
            })
            .await;
    }

    #[test]
    fn test_hooks_none_by_default() {
        let notifier = Notifier::new();
        assert!(notifier.hooks.is_none());
    }

    #[test]
    fn test_with_hooks_sets_hook_runner() {
        use crate::hooks::HookRunner;
        let runner = Arc::new(HookRunner::new("."));
        let notifier = Notifier::new().with_hooks(runner);
        assert!(notifier.hooks.is_some());
    }
}
