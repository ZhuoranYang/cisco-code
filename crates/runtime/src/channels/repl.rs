//! Interactive REPL channel.
//!
//! The CLI is just another channel — it implements the same Channel trait
//! as Webex, Slack, etc. This is the default channel for interactive use.

use std::io::{self, Write};

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};

/// REPL channel for CLI interaction.
pub struct ReplChannel {
    /// Optional single message (for one-shot `-m` mode).
    single_message: Option<String>,
}

impl ReplChannel {
    pub fn new() -> Self {
        Self {
            single_message: None,
        }
    }

    /// Create a REPL channel that sends a single message then signals exit.
    pub fn with_message(message: String) -> Self {
        Self {
            single_message: Some(message),
        }
    }
}

impl Default for ReplChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl Channel for ReplChannel {
    fn name(&self) -> &str {
        "repl"
    }

    async fn start(&self) -> anyhow::Result<MessageStream> {
        let (tx, rx) = mpsc::channel(32);
        let single_message = self.single_message.clone();

        // Spawn a blocking thread for stdin reading
        std::thread::spawn(move || {
            // Single message mode
            if let Some(msg) = single_message {
                let incoming = IncomingMessage::new("repl", "default", &msg);
                let _ = tx.blocking_send(incoming);
                // Signal exit
                let _ = tx.blocking_send(IncomingMessage::new("repl", "default", "/quit"));
                return;
            }

            // Interactive mode
            let stdin = io::stdin();
            loop {
                eprint!("\x1b[1;36m> \x1b[0m");
                let _ = io::stderr().flush();

                let mut line = String::new();
                match stdin.read_line(&mut line) {
                    Ok(0) => {
                        // EOF (Ctrl-D)
                        let _ = tx.blocking_send(IncomingMessage::new("repl", "default", "/quit"));
                        break;
                    }
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let msg = IncomingMessage::new("repl", "default", trimmed);
                        if tx.blocking_send(msg).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Input error: {e}");
                        break;
                    }
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> anyhow::Result<()> {
        println!("{}", response.content);
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> anyhow::Result<()> {
        match status {
            StatusUpdate::Thinking(msg) => eprintln!("  \x1b[90m{msg}\x1b[0m"),
            StatusUpdate::ToolStarted { name } => eprintln!("  \x1b[33m● {name}\x1b[0m"),
            StatusUpdate::ToolCompleted { name, success } => {
                if success {
                    eprintln!("  \x1b[32m✓ {name}\x1b[0m");
                } else {
                    eprintln!("  \x1b[31m✗ {name}\x1b[0m");
                }
            }
            StatusUpdate::StreamChunk(chunk) => {
                print!("{chunk}");
                let _ = io::stdout().flush();
            }
            StatusUpdate::Status(msg) => eprintln!("  \x1b[90m{msg}\x1b[0m"),
        }
        Ok(())
    }

    async fn health_check(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_single_message_mode() {
        let repl = ReplChannel::with_message("hello".into());
        let mut stream = repl.start().await.unwrap();

        let first = stream.next().await.unwrap();
        assert_eq!(first.content, "hello");
        assert_eq!(first.channel, "repl");

        let second = stream.next().await.unwrap();
        assert_eq!(second.content, "/quit");

        // Stream should end after quit
        assert!(stream.next().await.is_none());
    }
}
