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

use clap::{Parser, Subcommand};
use anyhow::Result;

#[derive(Parser)]
#[command(
    name = "cisco-code",
    about = "cisco-code: An enterprise AI coding agent by Cisco",
    version,
    long_about = "An AI coding agent combining the best of Claude Code, Codex, and Astro-Assistant.\nBuilt in Rust + Python for performance, safety, and extensibility."
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
                .add_directive("cisco_code=info".parse()?)
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Doctor) => {
            run_doctor().await?;
        }
        Some(Commands::Init) => {
            run_init().await?;
        }
        Some(Commands::Login { with_api_key }) => {
            run_login(with_api_key).await?;
        }
        Some(Commands::Logout) => {
            println!("Credentials cleared.");
        }
        Some(Commands::Prompt { text }) => {
            run_prompt(&text, &cli).await?;
        }
        Some(Commands::Resume { session }) => {
            println!("Resuming session: {}", session.unwrap_or("last".into()));
            // TODO: Phase 1 — implement session resume
        }
        Some(Commands::Server { listen }) => {
            println!("Starting app server on {listen}...");
            // TODO: Phase 9 — implement app server
        }
        None => {
            // Default: interactive REPL or one-shot if prompt provided
            if let Some(prompt) = cli.prompt {
                run_prompt(&prompt, &cli).await?;
            } else {
                run_repl(&cli).await?;
            }
        }
    }

    Ok(())
}

async fn run_doctor() -> Result<()> {
    println!("cisco-code doctor");
    println!("=================");
    println!();

    // Check Rust runtime
    println!("[ok] Rust runtime: v{}", env!("CARGO_PKG_VERSION"));

    // Check Python availability
    let python_check = tokio::process::Command::new("python3")
        .arg("--version")
        .output()
        .await;
    match python_check {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            println!("[ok] Python: {}", version.trim());
        }
        _ => {
            println!("[!!] Python 3 not found — provider adapters will not work");
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
    let config_path = dirs_next::home_dir()
        .map(|h| h.join(".cisco-code").join("config.toml"));
    match config_path {
        Some(p) if p.exists() => println!("[ok] User config: {}", p.display()),
        Some(p) => println!("[--] No user config at {}", p.display()),
        None => println!("[!!] Cannot determine home directory"),
    }

    // Check project config
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
        // TODO: Phase 0 — implement secure key storage
        println!("(API key login not yet implemented)");
    } else {
        println!("Opening browser for OAuth login...");
        // TODO: Phase 0 — implement OAuth PKCE flow
        println!("(OAuth login not yet implemented)");
    }
    Ok(())
}

async fn run_prompt(text: &str, _cli: &Cli) -> Result<()> {
    println!("cisco-code v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("> {text}");
    println!();

    // TODO: Phase 1 — wire up ConversationRuntime
    let config = cisco_code_runtime::RuntimeConfig::default();
    let tools = cisco_code_tools::ToolRegistry::with_builtins()?;

    println!("Model: {}", config.model);
    println!("Tools: {} registered", tools.definitions().len());
    println!();
    println!("(Agent loop not yet implemented — Phase 1)");
    Ok(())
}

async fn run_repl(_cli: &Cli) -> Result<()> {
    println!("cisco-code v{} — Interactive Mode", env!("CARGO_PKG_VERSION"));
    println!("Type your prompt, or /help for commands.");
    println!();

    // TODO: Phase 1 — implement interactive REPL with Ratatui
    println!("(Interactive REPL not yet implemented — Phase 1)");
    println!("Use `cisco-code prompt \"text\"` for one-shot mode.");
    Ok(())
}
