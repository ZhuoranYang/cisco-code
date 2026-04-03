//! cisco-code CLI entry point.
//!
//! Subcommands:
//! - `cisco-code` — Interactive REPL (default)
//! - `cisco-code prompt "text"` — One-shot execution
//! - `cisco-code login` — API key / OAuth setup
//! - `cisco-code doctor` — Environment health check
//! - `cisco-code init` — Create project config
//!
//! Design insight from Codex: Clean subcommand routing with clap,
//! plus a default interactive TUI mode.

use anyhow::{Context, Result};
use cisco_code_api::AnthropicClient;
use cisco_code_protocol::{StopReason, StreamEvent};
use cisco_code_runtime::{ConversationRuntime, RuntimeConfig};
use cisco_code_tools::ToolRegistry;
use clap::{Parser, Subcommand};
use std::io::{self, Write};

#[derive(Parser)]
#[command(
    name = "cisco-code",
    about = "cisco-code: An enterprise AI coding agent by Cisco",
    version,
    long_about = "An AI coding agent combining the best of Claude Code, Codex, and Astro-Assistant.\nBuilt in pure Rust for performance, safety, and a single-binary deployment."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Model to use (e.g., claude-sonnet-4-6, gpt-5, gemini-2.5-pro)
    #[arg(short, long, global = true)]
    model: Option<String>,

    /// Permission mode (default, accept-reads, bypass, deny-all)
    #[arg(long, global = true)]
    permission_mode: Option<String>,

    /// Sandbox mode (none, os-native, container)
    #[arg(long, global = true)]
    sandbox: Option<String>,

    /// Config profile to use
    #[arg(short, long, global = true)]
    profile: Option<String>,

    /// One-shot prompt (alternative to `cisco-code prompt "text"`)
    prompt: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Execute a one-shot prompt
    Prompt {
        /// The prompt text
        text: String,
    },

    /// Set up authentication (API keys or OAuth)
    Login {
        /// Use API key instead of OAuth
        #[arg(long)]
        with_api_key: bool,
    },

    /// Clear stored credentials
    Logout,

    /// Check environment and configuration
    Doctor,

    /// Initialize project configuration (.cisco-code/)
    Init,

    /// Resume a previous session
    Resume {
        /// Session ID (or "last" for most recent)
        session: Option<String>,
    },

    /// Start the app server (for IDE integration)
    Server {
        /// Listen address
        #[arg(long, default_value = "127.0.0.1:3000")]
        listen: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("cisco_code=info".parse()?),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Doctor) => run_doctor().await?,
        Some(Commands::Init) => run_init().await?,
        Some(Commands::Login { with_api_key }) => run_login(with_api_key).await?,
        Some(Commands::Logout) => println!("Credentials cleared."),
        Some(Commands::Prompt { text }) => run_prompt(&text, &cli).await?,
        Some(Commands::Resume { session }) => {
            println!(
                "Resuming session: {}",
                session.unwrap_or("last".into())
            );
            // TODO: implement session resume
        }
        Some(Commands::Server { listen }) => {
            println!("Starting app server on {listen}...");
            // TODO: implement app server
        }
        None => {
            if let Some(prompt) = cli.prompt {
                run_prompt(&prompt, &cli).await?;
            } else {
                run_repl(&cli).await?;
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Build runtime from CLI args + environment
// ---------------------------------------------------------------------------

fn build_config(cli: &Cli) -> RuntimeConfig {
    let mut config = RuntimeConfig::default();

    if let Some(ref model) = cli.model {
        config.model = model.clone();
    }

    if let Some(ref mode) = cli.permission_mode {
        config.permission_mode = match mode.as_str() {
            "accept-reads" => cisco_code_runtime::PermissionMode::AcceptReads,
            "bypass" => cisco_code_runtime::PermissionMode::BypassPermissions,
            "deny-all" => cisco_code_runtime::PermissionMode::DenyAll,
            _ => cisco_code_runtime::PermissionMode::Default,
        };
    }

    config
}

fn build_provider() -> Result<AnthropicClient> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .context("ANTHROPIC_API_KEY not set. Run `cisco-code login --with-api-key` or set the environment variable.")?;

    let mut client = AnthropicClient::new(api_key);

    // Allow custom base URL (for proxies, Bedrock, etc.)
    if let Ok(base_url) = std::env::var("ANTHROPIC_BASE_URL") {
        client = client.with_base_url(base_url);
    }

    Ok(client)
}

fn build_runtime(cli: &Cli) -> Result<ConversationRuntime<AnthropicClient>> {
    let config = build_config(cli);
    let provider = build_provider()?;
    let tools = ToolRegistry::with_builtins()?;
    Ok(ConversationRuntime::new(provider, tools, config))
}

// ---------------------------------------------------------------------------
// Render stream events to terminal
// ---------------------------------------------------------------------------

fn render_events(events: &[StreamEvent]) {
    for event in events {
        match event {
            StreamEvent::TurnStart { model, turn_number } => {
                if *turn_number > 1 {
                    eprintln!("\n--- turn {turn_number} ({model}) ---\n");
                }
            }
            StreamEvent::TextDelta { text } => {
                print!("{text}");
                let _ = io::stdout().flush();
            }
            StreamEvent::ToolUseStart {
                tool_name,
                tool_use_id: _,
            } => {
                eprintln!("\n[tool: {tool_name}]");
            }
            StreamEvent::ToolExecutionStart {
                tool_name,
                description,
                tool_use_id: _,
            } => {
                eprintln!("  > {description}");
            }
            StreamEvent::ToolResult {
                result,
                is_error,
                tool_use_id: _,
            } => {
                if *is_error {
                    eprintln!("  [error] {}", truncate(result, 500));
                } else {
                    eprintln!("  [ok] {}", truncate(result, 500));
                }
            }
            StreamEvent::TurnEnd { usage, stop_reason } => {
                let tokens = usage.total();
                if tokens > 0 {
                    eprintln!(
                        "\n({} tokens: {} in / {} out)",
                        tokens, usage.input_tokens, usage.output_tokens
                    );
                }
                if *stop_reason == StopReason::MaxTokens {
                    eprintln!("[warning] Response truncated (max tokens reached)");
                }
            }
            StreamEvent::Error {
                message,
                recoverable,
            } => {
                if *recoverable {
                    eprintln!("[warning] {message}");
                } else {
                    eprintln!("[error] {message}");
                }
            }
            _ => {}
        }
    }
    println!(); // final newline after all output
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        // Find a safe UTF-8 boundary
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

// ---------------------------------------------------------------------------
// Subcommand implementations
// ---------------------------------------------------------------------------

async fn run_prompt(text: &str, cli: &Cli) -> Result<()> {
    eprintln!("cisco-code v{}", env!("CARGO_PKG_VERSION"));
    eprintln!();

    let mut runtime = build_runtime(cli)?;
    eprintln!(
        "Model: {} | Tools: {}",
        runtime.config.model,
        runtime.tools.definitions().len()
    );
    eprintln!();

    let events = runtime.submit_message(text).await?;
    render_events(&events);

    let usage = runtime.total_usage();
    eprintln!(
        "Session total: {} tokens ({} in / {} out)",
        usage.total(),
        usage.input_tokens,
        usage.output_tokens
    );

    Ok(())
}

async fn run_repl(cli: &Cli) -> Result<()> {
    eprintln!(
        "cisco-code v{} — Interactive Mode",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("Type your prompt, or /help for commands. Ctrl-D to exit.");
    eprintln!();

    let mut runtime = build_runtime(cli)?;
    eprintln!(
        "Model: {} | Tools: {}",
        runtime.config.model,
        runtime.tools.definitions().len()
    );
    eprintln!();

    let stdin = io::stdin();

    loop {
        eprint!("> ");
        let _ = io::stderr().flush();

        let mut line = String::new();
        let bytes = stdin.read_line(&mut line)?;
        if bytes == 0 {
            // EOF (Ctrl-D)
            eprintln!("\nGoodbye!");
            break;
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        // Handle REPL commands
        match input {
            "/help" => {
                eprintln!("Commands:");
                eprintln!("  /help     — Show this help");
                eprintln!("  /model    — Show current model");
                eprintln!("  /usage    — Show token usage");
                eprintln!("  /clear    — Clear conversation history");
                eprintln!("  /quit     — Exit");
                continue;
            }
            "/model" => {
                eprintln!("Model: {}", runtime.config.model);
                continue;
            }
            "/usage" => {
                let usage = runtime.total_usage();
                eprintln!(
                    "Session: {} turns, {} tokens ({} in / {} out)",
                    runtime.turn_count(),
                    usage.total(),
                    usage.input_tokens,
                    usage.output_tokens
                );
                continue;
            }
            "/clear" => {
                runtime.session = cisco_code_runtime::Session::new();
                eprintln!("Conversation cleared.");
                continue;
            }
            "/quit" | "/exit" | "/q" => {
                eprintln!("Goodbye!");
                break;
            }
            _ if input.starts_with('/') => {
                eprintln!("Unknown command: {input}. Type /help for available commands.");
                continue;
            }
            _ => {}
        }

        // Submit to agent
        match runtime.submit_message(input).await {
            Ok(events) => render_events(&events),
            Err(e) => {
                eprintln!("[error] {e}");
                eprintln!("(The conversation continues — try again or /help)");
            }
        }
    }

    Ok(())
}

async fn run_doctor() -> Result<()> {
    println!("cisco-code doctor");
    println!("=================");
    println!();

    println!("[ok] Rust runtime: v{}", env!("CARGO_PKG_VERSION"));

    // Check ripgrep (needed by Grep/Glob tools)
    let rg_check = tokio::process::Command::new("rg")
        .arg("--version")
        .output()
        .await;
    match rg_check {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            let first_line = version.lines().next().unwrap_or("unknown");
            println!("[ok] ripgrep: {first_line}");
        }
        _ => {
            println!("[!!] ripgrep (rg) not found — Grep/Glob tools will use fallbacks");
        }
    }

    // Check API keys
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        println!("[ok] ANTHROPIC_API_KEY is set");
    } else {
        println!("[--] ANTHROPIC_API_KEY not set");
    }

    if std::env::var("OPENAI_API_KEY").is_ok() {
        println!("[ok] OPENAI_API_KEY is set");
    } else {
        println!("[--] OPENAI_API_KEY not set");
    }

    // Check config
    let config_path = dirs_next::home_dir().map(|h| h.join(".cisco-code").join("config.toml"));
    match config_path {
        Some(p) if p.exists() => println!("[ok] User config: {}", p.display()),
        Some(p) => println!("[--] No user config at {}", p.display()),
        None => println!("[!!] Cannot determine home directory"),
    }

    if std::path::Path::new(".cisco-code").exists() {
        println!("[ok] Project config: .cisco-code/");
    } else {
        println!("[--] No project config (run `cisco-code init` to create)");
    }

    println!();
    println!("cisco-code is ready.");
    Ok(())
}

async fn run_init() -> Result<()> {
    let config_dir = std::path::Path::new(".cisco-code");
    if config_dir.exists() {
        println!(".cisco-code/ already exists");
        return Ok(());
    }

    std::fs::create_dir_all(config_dir)?;
    std::fs::write(
        config_dir.join("config.toml"),
        r#"# cisco-code project configuration
# See: https://cisco.github.io/cisco-code/config

[general]
# default_model = "claude-sonnet-4-6"
# plan_mode = "auto"

[permissions]
# mode = "default"

[sandbox]
# mode = "os-native"
"#,
    )?;

    println!("Created .cisco-code/config.toml");
    println!("Edit this file to configure cisco-code for your project.");
    Ok(())
}

async fn run_login(with_api_key: bool) -> Result<()> {
    if with_api_key {
        println!("Enter your API key:");
        // TODO: implement secure key storage
        println!("(API key login not yet implemented)");
    } else {
        println!("Opening browser for OAuth login...");
        // TODO: implement OAuth PKCE flow
        println!("(OAuth login not yet implemented)");
    }
    Ok(())
}
