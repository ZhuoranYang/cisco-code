//! cisco-code CLI entry point.
//!
//! Subcommands:
//! - `cisco-code` — Interactive REPL (default)
//! - `cisco-code prompt "text"` — One-shot execution
//! - `cisco-code login` — API key / OAuth setup
//! - `cisco-code doctor` — Environment health check
//! - `cisco-code init` — Create project config

use anyhow::Result;
use cisco_code_api::oauth::CodexAuth;
use cisco_code_api::Provider;
use cisco_code_protocol::{StopReason, StreamEvent};
use cisco_code_providers::{ModelClass, ModelConfig, ModelSpec, ProviderRegistry};
use cisco_code_runtime::{CommandRegistry, CommandResult, ConversationRuntime, RuntimeConfig, discover_skills};
use cisco_code_server::ProviderFactory;
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

    /// Model to use (e.g., claude-sonnet-4-6, openai/gpt-4o, bedrock/anthropic.claude-3-5-sonnet)
    #[arg(short, long, global = true)]
    model: Option<String>,

    /// Model class: small, medium, large
    #[arg(short = 'c', long, global = true)]
    class: Option<String>,

    /// Permission mode (default, accept-reads, bypass, deny-all)
    #[arg(long, global = true)]
    permission_mode: Option<String>,

    /// Sandbox mode (none, os-native, container)
    #[arg(long, global = true)]
    sandbox: Option<String>,

    /// Config profile to use
    #[arg(short, long, global = true)]
    profile: Option<String>,

    /// Print mode: non-interactive, no tool use, output text only (like Claude Code -p)
    #[arg(short = 'p', long)]
    print: bool,

    /// Output format for print mode: text, json, stream-json
    #[arg(long, default_value = "text")]
    output_format: String,

    /// Override the system prompt
    #[arg(long)]
    system_prompt: Option<String>,

    /// Maximum number of agent turns
    #[arg(long)]
    max_turns: Option<u32>,

    /// Continue the most recent conversation
    #[arg(long, short = 'r')]
    resume: bool,

    /// Verbose output (show tool calls, tokens, timing)
    #[arg(short, long)]
    verbose: bool,

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

    /// Attach to a running server via WebSocket (remote TUI)
    Attach {
        /// WebSocket URL of the running server
        #[arg(long, default_value = "ws://127.0.0.1:3000/api/v1/ws/default")]
        url: String,
        /// Session ID to attach to (overrides URL path)
        #[arg(long)]
        session: Option<String>,
    },

    /// Run as a persistent daemon with channels (Slack, Webex, cron)
    Daemon {
        /// Enable Slack channel
        #[arg(long)]
        slack: bool,
        /// Enable Webex channel
        #[arg(long)]
        webex: bool,
        /// Enable cron job scheduler
        #[arg(long)]
        cron: bool,
        /// Also start HTTP/WS server on this address
        #[arg(long)]
        listen: Option<String>,
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
        Some(Commands::Logout) => {
            let auth = CodexAuth::new();
            auth.logout()?;
            println!("OAuth credentials cleared.");
        }
        Some(Commands::Prompt { ref text }) => {
            if cli.print {
                run_print(text, &cli).await?;
            } else {
                run_prompt(text, &cli).await?;
            }
        }
        Some(Commands::Resume { ref session }) => {
            run_resume(session.as_deref(), &cli).await?;
        }
        Some(Commands::Server { ref listen }) => {
            run_server(listen, &cli).await?;
        }
        Some(Commands::Attach { ref url, ref session }) => {
            run_attach(url, session.as_deref()).await?;
        }
        Some(Commands::Daemon { ref slack, ref webex, ref cron, ref listen }) => {
            run_daemon(*slack, *webex, *cron, listen.as_deref(), &cli).await?;
        }
        None => {
            if cli.resume {
                // --resume flag: resume the most recent session
                run_resume(Some("last"), &cli).await?;
            } else if let Some(ref prompt) = cli.prompt {
                if cli.print {
                    run_print(prompt, &cli).await?;
                } else {
                    run_prompt(prompt, &cli).await?;
                }
            } else if cli.print {
                // -p with stdin
                run_print_stdin(&cli).await?;
            } else {
                run_repl(&cli).await?;
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Build runtime from CLI args + environment + providers
// ---------------------------------------------------------------------------

fn build_config(cli: &Cli) -> RuntimeConfig {
    let mut config = RuntimeConfig::load().unwrap_or_default();

    // CLI overrides
    if let Some(ref model) = cli.model {
        config.model = model.clone();
    }

    if let Some(ref class) = cli.class {
        config.model_class = Some(class.clone());
    }

    if let Some(ref mode) = cli.permission_mode {
        config.permission_mode = match mode.as_str() {
            "accept-reads" => cisco_code_runtime::PermissionMode::AcceptReads,
            "bypass" => cisco_code_runtime::PermissionMode::BypassPermissions,
            "deny-all" => cisco_code_runtime::PermissionMode::DenyAll,
            _ => cisco_code_runtime::PermissionMode::Default,
        };
    }

    if let Some(max_turns) = cli.max_turns {
        config.max_turns = max_turns;
    }

    // Print mode implies bypass permissions (non-interactive)
    if cli.print {
        config.permission_mode = cisco_code_runtime::PermissionMode::BypassPermissions;
    }

    config
}

fn resolve_provider(config: &mut RuntimeConfig) -> Result<Box<dyn Provider>> {
    let model_config = ModelConfig::default();
    let registry = ProviderRegistry::auto_discover(model_config)?;

    // If model_class is set, resolve it to a specific provider + model
    if let Some(ref class_str) = config.model_class {
        let class = ModelClass::from_str_loose(class_str)
            .ok_or_else(|| anyhow::anyhow!("Unknown model class: {class_str}. Use: small, medium, large"))?;
        let (provider, model) = registry.provider_for_class(class)?;
        config.model = model;
        eprintln!("Model class: {class} → {}", config.model);
        return Ok(provider);
    }

    // If a specific model is set (possibly with provider prefix), use it
    let spec = ModelSpec::parse(&config.model);
    if registry.has_provider(&spec.provider) {
        let (provider, model) = registry.provider_for_spec(&spec)?;
        config.model = model;
        return Ok(provider);
    }

    // Fallback: try each available provider
    let available = registry.available_providers();
    if available.is_empty() {
        anyhow::bail!("No LLM providers available");
    }

    // Default: use the first available provider with the configured model
    let first = available[0];
    let spec = ModelSpec::new(first, &config.model);
    let (provider, model) = registry.provider_for_spec(&spec)?;
    config.model = model;
    Ok(provider)
}

fn build_runtime(cli: &Cli) -> Result<ConversationRuntime<Box<dyn Provider>>> {
    let mut config = build_config(cli);
    let provider = resolve_provider(&mut config)?;
    let tools = ToolRegistry::with_builtins()?;
    Ok(ConversationRuntime::new(provider, tools, config))
}

/// Build a runtime with an existing session (for resume).
fn build_runtime_with_session(
    cli: &Cli,
    session: cisco_code_runtime::Session,
) -> Result<ConversationRuntime<Box<dyn Provider>>> {
    let mut config = build_config(cli);
    let provider = resolve_provider(&mut config)?;
    let tools = ToolRegistry::with_builtins()?;
    Ok(ConversationRuntime::with_session(provider, tools, config, session))
}

/// Get the sessions directory (~/.cisco-code/sessions/).
fn sessions_dir() -> std::path::PathBuf {
    dirs_next::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".cisco-code")
        .join("sessions")
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
                tool_name: _,
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
            StreamEvent::PermissionRequest {
                tool_name,
                input_summary,
                tool_use_id: _,
            } => {
                eprintln!("  [permission] {tool_name}: {input_summary}");
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

/// Print mode: non-interactive, outputs only the assistant's text.
/// Matches Claude Code's `-p` flag behavior.
async fn run_print(text: &str, cli: &Cli) -> Result<()> {
    let mut runtime = build_runtime(cli)?;

    if cli.verbose {
        eprintln!(
            "Model: {} | Tools: {}",
            runtime.config.model,
            runtime.tools.definitions().len()
        );
    }

    let events = runtime.submit_message(text).await?;

    match cli.output_format.as_str() {
        "json" => {
            // Collect all text and output as JSON
            let text: String = events
                .iter()
                .filter_map(|e| match e {
                    StreamEvent::TextDelta { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            let usage = runtime.total_usage();
            let output = serde_json::json!({
                "result": text,
                "model": runtime.config.model,
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "turns": runtime.turn_count(),
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        "stream-json" => {
            // Output each event as a JSONL line
            for event in &events {
                let json = serde_json::to_string(event)?;
                println!("{json}");
            }
        }
        _ => {
            // text mode: only output the assistant's text
            for event in &events {
                if let StreamEvent::TextDelta { text } = event {
                    print!("{text}");
                }
            }
            // Final newline if any text was produced
            let has_text = events
                .iter()
                .any(|e| matches!(e, StreamEvent::TextDelta { .. }));
            if has_text {
                println!();
            }
        }
    }

    if cli.verbose {
        let usage = runtime.total_usage();
        eprintln!(
            "({} tokens: {} in / {} out, {} turns)",
            usage.total(),
            usage.input_tokens,
            usage.output_tokens,
            runtime.turn_count(),
        );
    }

    Ok(())
}

/// Print mode with stdin: read prompt from stdin pipe.
async fn run_print_stdin(cli: &Cli) -> Result<()> {
    use std::io::Read;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let input = input.trim();
    if input.is_empty() {
        anyhow::bail!("No input provided via stdin. Usage: echo 'prompt' | cisco-code -p");
    }
    run_print(input, cli).await
}

async fn run_repl(cli: &Cli) -> Result<()> {
    eprintln!(
        "cisco-code v{} — Interactive Mode",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("Type your prompt, or /help for commands. Ctrl-D to exit.");
    eprintln!();

    let mut runtime = build_runtime(cli)?;
    let commands = CommandRegistry::with_builtins();

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
            eprintln!("\nGoodbye!");
            break;
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        // Parse through command registry
        let effective_input = match commands.parse(input) {
            CommandResult::NotACommand => input.to_string(),
            CommandResult::ExpandedPrompt(prompt) => prompt,
            CommandResult::BuiltinAction { action, args } => {
                match handle_builtin_action(&action, &args, &mut runtime, &commands) {
                    ActionOutcome::Continue => continue,
                    ActionOutcome::Exit => break,
                }
            }
            CommandResult::Unknown(name) => {
                eprintln!("Unknown command: /{name}. Type /help for available commands.");
                continue;
            }
        };

        // Submit to agent
        match runtime.submit_message(&effective_input).await {
            Ok(events) => render_events(&events),
            Err(e) => {
                eprintln!("[error] {e}");
                eprintln!("(The conversation continues — try again or /help)");
            }
        }
    }

    Ok(())
}

enum ActionOutcome {
    Continue,
    Exit,
}

fn handle_builtin_action(
    action: &str,
    args: &str,
    runtime: &mut ConversationRuntime<Box<dyn Provider>>,
    commands: &CommandRegistry,
) -> ActionOutcome {
    match action {
        "help" => {
            eprintln!("Prompt commands (sent to agent):");
            for cmd in commands.prompt_commands() {
                eprintln!("  /{:<16} — {}", cmd.name, cmd.description);
            }
            eprintln!();
            eprintln!("Built-in commands:");
            eprintln!("  /{:<16} — Show this help", "help");
            eprintln!("  /{:<16} — Show or switch model", "model");
            eprintln!("  /{:<16} — Show token usage", "usage");
            eprintln!("  /{:<16} — Show session cost", "cost");
            eprintln!("  /{:<16} — Show session status", "status");
            eprintln!("  /{:<16} — Show cisco-code version", "version");
            eprintln!("  /{:<16} — Show context window usage", "context");
            eprintln!("  /{:<16} — Show or change permissions", "permissions");
            eprintln!("  /{:<16} — Show current git diff", "diff");
            eprintln!("  /{:<16} — Show current git branch", "branch");
            eprintln!("  /{:<16} — Clear conversation", "clear");
            eprintln!("  /{:<16} — Force context compaction", "compact");
            eprintln!("  /{:<16} — Exit", "quit");

            // List available skills (bundled + user-defined)
            let cwd = std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string());
            let skills = discover_skills(&cwd);
            let invocable: Vec<_> = skills.iter().filter(|s| s.user_invocable).collect();
            if !invocable.is_empty() {
                eprintln!();
                eprintln!("Skills (slash commands):");
                for skill in &invocable {
                    let tag = if skill.bundled { "" } else { " [custom]" };
                    if skill.description.is_empty() {
                        eprintln!("  /{:<16}{}", skill.name, tag);
                    } else {
                        eprintln!("  /{:<16} — {}{}", skill.name, skill.description, tag);
                    }
                }
            }
            ActionOutcome::Continue
        }
        "model" => {
            if args.is_empty() {
                eprintln!("Model: {}", runtime.config.model);
                if let Some(ref class) = runtime.config.model_class {
                    eprintln!("Class: {class}");
                }
            } else {
                runtime.config.model = args.to_string();
                eprintln!("Switched to model: {args}");
            }
            ActionOutcome::Continue
        }
        "usage" | "cost" => {
            let usage = runtime.total_usage();
            let cost_usd = cisco_code_api::calculate_cost(
                &runtime.config.model,
                usage.input_tokens,
                usage.output_tokens,
            );
            eprintln!(
                "Session: {} turns, {} tokens ({} in / {} out)",
                runtime.turn_count(),
                usage.total(),
                usage.input_tokens,
                usage.output_tokens,
            );
            eprintln!("Estimated cost: ${:.4}", cost_usd);
            eprintln!("Messages: {}", runtime.session.messages.len());
            ActionOutcome::Continue
        }
        "status" => {
            let usage = runtime.total_usage();
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| ".".into());
            eprintln!("Model:    {}", runtime.config.model);
            eprintln!("Turns:    {}", runtime.turn_count());
            eprintln!("Tokens:   {} in / {} out", usage.input_tokens, usage.output_tokens);
            eprintln!("Messages: {}", runtime.session.messages.len());
            eprintln!("Tools:    {}", runtime.tools.definitions().len());
            eprintln!("CWD:      {cwd}");
            ActionOutcome::Continue
        }
        "version" => {
            eprintln!("cisco-code v{}", env!("CARGO_PKG_VERSION"));
            ActionOutcome::Continue
        }
        "context" => {
            let estimated = cisco_code_runtime::Compactor::estimate_tokens(&runtime.session.messages);
            let threshold = cisco_code_runtime::threshold_for_model(&runtime.config.model);
            let pct = if threshold > 0 {
                (estimated as f64 / threshold as f64 * 100.0) as u64
            } else {
                0
            };
            eprintln!("Context: ~{} tokens / {} threshold ({}%)", estimated, threshold, pct);
            eprintln!("Messages: {}", runtime.session.messages.len());
            ActionOutcome::Continue
        }
        "permissions" => {
            if args.is_empty() {
                eprintln!("Permission mode: {:?}", runtime.permissions.mode());
            } else {
                let new_mode = match args {
                    "accept-reads" => cisco_code_runtime::PermissionMode::AcceptReads,
                    "bypass" => cisco_code_runtime::PermissionMode::BypassPermissions,
                    "deny-all" => cisco_code_runtime::PermissionMode::DenyAll,
                    "default" => cisco_code_runtime::PermissionMode::Default,
                    _ => {
                        eprintln!("Unknown mode: {args}. Use: default, accept-reads, bypass, deny-all");
                        return ActionOutcome::Continue;
                    }
                };
                runtime.permissions.set_mode(new_mode);
                eprintln!("Permission mode set to: {args}");
            }
            ActionOutcome::Continue
        }
        "diff" => {
            // Shell out to git diff
            match std::process::Command::new("git")
                .args(["diff", "--stat"])
                .output()
            {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if stdout.is_empty() {
                        eprintln!("No uncommitted changes.");
                    } else {
                        eprint!("{stdout}");
                    }
                }
                Err(_) => eprintln!("git not available"),
            }
            ActionOutcome::Continue
        }
        "branch" => {
            match std::process::Command::new("git")
                .args(["branch", "--show-current"])
                .output()
            {
                Ok(output) => {
                    let branch = String::from_utf8_lossy(&output.stdout);
                    eprintln!("Branch: {}", branch.trim());
                }
                Err(_) => eprintln!("git not available"),
            }
            ActionOutcome::Continue
        }
        "clear" => {
            runtime.session = cisco_code_runtime::Session::new();
            eprintln!("Conversation cleared.");
            ActionOutcome::Continue
        }
        "compact" => {
            eprintln!("Context compaction will trigger on next turn.");
            ActionOutcome::Continue
        }
        "config" => {
            eprintln!("Model:       {}", runtime.config.model);
            eprintln!("Max tokens:  {}", runtime.config.max_tokens);
            eprintln!("Max turns:   {}", runtime.config.max_turns);
            eprintln!("Permissions: {:?}", runtime.permissions.mode());
            if let Some(budget) = runtime.config.max_budget_usd {
                eprintln!("Budget:      ${:.2}", budget);
            }
            if let Some(temp) = runtime.config.temperature {
                eprintln!("Temperature: {temp}");
            }
            ActionOutcome::Continue
        }
        "effort" => {
            if args.is_empty() {
                eprintln!("Usage: /effort low|medium|high");
            } else {
                match args {
                    "low" => {
                        runtime.config.max_tokens = 4096;
                        eprintln!("Effort: low (max_tokens=4096)");
                    }
                    "medium" => {
                        runtime.config.max_tokens = 16384;
                        eprintln!("Effort: medium (max_tokens=16384)");
                    }
                    "high" => {
                        runtime.config.max_tokens = 32768;
                        eprintln!("Effort: high (max_tokens=32768)");
                    }
                    _ => eprintln!("Unknown effort level: {args}. Use: low, medium, high"),
                }
            }
            ActionOutcome::Continue
        }
        "fast" => {
            // Toggle between normal and fast mode (lower max_tokens for faster output)
            if runtime.config.max_tokens > 8192 {
                runtime.config.max_tokens = 8192;
                eprintln!("Fast mode: ON (max_tokens=8192)");
            } else {
                runtime.config.max_tokens = 16384;
                eprintln!("Fast mode: OFF (max_tokens=16384)");
            }
            ActionOutcome::Continue
        }
        "export" => {
            let filename = if args.is_empty() {
                format!("conversation-{}.md", runtime.session.id.chars().take(8).collect::<String>())
            } else {
                args.to_string()
            };
            eprintln!("Export to {filename} is not yet implemented.");
            ActionOutcome::Continue
        }
        "quit" => {
            eprintln!("Goodbye!");
            ActionOutcome::Exit
        }
        _ => {
            eprintln!("Action '/{action}' is not yet implemented.");
            ActionOutcome::Continue
        }
    }
}

async fn run_doctor() -> Result<()> {
    println!("cisco-code doctor");
    println!("=================");
    println!();

    println!("[ok] Rust runtime: v{}", env!("CARGO_PKG_VERSION"));

    // Check ripgrep
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

    // Check provider credentials
    println!();
    println!("Providers:");

    if std::env::var("AWS_ACCESS_KEY_ID").is_ok()
        && std::env::var("AWS_SECRET_ACCESS_KEY").is_ok()
    {
        let region = std::env::var("AWS_REGION")
            .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_string());
        println!("[ok] Bedrock (region: {region})");
    } else {
        println!("[--] Bedrock (set AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY)");
    }

    if std::env::var("OPENAI_API_KEY").is_ok() {
        let base = std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "api.openai.com".into());
        println!("[ok] OpenAI — API key (endpoint: {base})");
    } else {
        let codex_auth = CodexAuth::new();
        if codex_auth.has_tokens() {
            println!("[ok] OpenAI — OAuth/Codex (ChatGPT subscription)");
        } else {
            println!("[--] OpenAI (set OPENAI_API_KEY or run `cisco-code login`)");
        }
    }

    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        println!("[ok] Anthropic (direct API)");
    } else {
        println!("[--] Anthropic (set ANTHROPIC_API_KEY)");
    }

    if let Ok(url) = std::env::var("CISCO_CODE_LOCAL_URL") {
        println!("[ok] Local endpoint: {url}");
    } else {
        println!("[--] Local models (set CISCO_CODE_LOCAL_URL=http://localhost:11434/v1)");
    }

    // Model classes
    println!();
    println!("Model classes (defaults):");
    let config = ModelConfig::default();
    println!("  small:  {}/{}", config.small.provider, config.small.model);
    println!(
        "  medium: {}/{}",
        config.medium.provider, config.medium.model
    );
    println!("  large:  {}/{}", config.large.provider, config.large.model);

    // Check config
    println!();
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

[general]
# Model class: small, medium, large
# model_class = "medium"
# Or specify a model directly: provider/model
# default_model = "bedrock/anthropic.claude-3-5-sonnet-20241022-v2:0"

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

async fn run_resume(session_id: Option<&str>, cli: &Cli) -> Result<()> {
    eprintln!("cisco-code v{}", env!("CARGO_PKG_VERSION"));
    eprintln!();

    let dir = sessions_dir();

    let session = match session_id {
        Some("last") | None => {
            // Find the most recent session
            let sessions = cisco_code_runtime::Session::list_sessions(&dir)?;
            if sessions.is_empty() {
                anyhow::bail!(
                    "No sessions found in {}. Start a conversation first.",
                    dir.display()
                );
            }
            let latest = &sessions[0];
            eprintln!(
                "Resuming session: {} — {}",
                &latest.id[..8.min(latest.id.len())],
                latest.display_name()
            );
            cisco_code_runtime::Session::load(&latest.path)?
        }
        Some(id) => {
            // Try exact session ID
            let path = dir.join(format!("{id}.jsonl"));
            if path.exists() {
                eprintln!("Resuming session: {id}");
                cisco_code_runtime::Session::load(&path)?
            } else {
                // Try prefix match
                let sessions = cisco_code_runtime::Session::list_sessions(&dir)?;
                let matches: Vec<_> = sessions
                    .iter()
                    .filter(|s| s.id.starts_with(id))
                    .collect();
                match matches.len() {
                    0 => anyhow::bail!("No session found matching '{id}'"),
                    1 => {
                        eprintln!("Resuming session: {}", matches[0].id);
                        cisco_code_runtime::Session::load(&matches[0].path)?
                    }
                    n => {
                        eprintln!("Multiple sessions match '{id}':");
                        for s in &matches {
                            eprintln!(
                                "  {} — {} ({} msgs)",
                                &s.id[..8.min(s.id.len())],
                                s.display_name(),
                                s.message_count,
                            );
                        }
                        anyhow::bail!("{n} sessions match '{id}' — be more specific");
                    }
                }
            }
        }
    };

    eprintln!(
        "Loaded {} messages ({} turns, ${:.4} cost)",
        session.messages.len(),
        session.metadata.turn_count,
        session.metadata.cost_usd,
    );
    if session.metadata.compaction_count > 0 {
        eprintln!(
            "Note: {} context compaction(s) occurred in this session",
            session.metadata.compaction_count
        );
    }
    eprintln!();

    let mut runtime = build_runtime_with_session(cli, session)?;
    let commands = CommandRegistry::with_builtins();

    eprintln!(
        "Model: {} | Tools: {}",
        runtime.config.model,
        runtime.tools.definitions().len()
    );
    eprintln!();

    // Enter REPL mode with the restored session (reuse command registry)
    let stdin = io::stdin();

    loop {
        eprint!("> ");
        let _ = io::stderr().flush();

        let mut line = String::new();
        let bytes = stdin.read_line(&mut line)?;
        if bytes == 0 {
            eprintln!("\nGoodbye!");
            break;
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        let effective_input = match commands.parse(input) {
            CommandResult::NotACommand => input.to_string(),
            CommandResult::ExpandedPrompt(prompt) => prompt,
            CommandResult::BuiltinAction { action, args } => {
                match handle_builtin_action(&action, &args, &mut runtime, &commands) {
                    ActionOutcome::Continue => continue,
                    ActionOutcome::Exit => break,
                }
            }
            CommandResult::Unknown(name) => {
                eprintln!("Unknown command: /{name}. Type /help for available commands.");
                continue;
            }
        };

        match runtime.submit_message(&effective_input).await {
            Ok(events) => render_events(&events),
            Err(e) => {
                eprintln!("[error] {e}");
                eprintln!("(The conversation continues — try again or /help)");
            }
        }
    }

    Ok(())
}

async fn run_server(listen: &str, cli: &Cli) -> Result<()> {
    use std::sync::Arc;
    use cisco_code_runtime::{SqliteStore, Store};
    use cisco_code_server::{AppState, DefaultProviderFactory};

    eprintln!("cisco-code server v{}", env!("CARGO_PKG_VERSION"));
    eprintln!();

    // 1. Init SQLite store at ~/.cisco-code/cisco-code.db
    let db_dir = dirs_next::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".cisco-code");
    std::fs::create_dir_all(&db_dir)?;
    let db_path = db_dir.join("cisco-code.db");
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open(&db_path.to_string_lossy())?);
    eprintln!("[ok] Store: {}", db_path.display());

    // 2. Discover providers
    let provider_factory = Arc::new(DefaultProviderFactory::auto_discover()?);
    eprintln!("[ok] Provider factory initialized");

    // 3. Build config (server defaults to bypass permissions — non-interactive)
    let mut config = build_config(cli);
    config.permission_mode = cisco_code_runtime::PermissionMode::BypassPermissions;

    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".into());

    // 4. Build AppState
    let state = AppState::new(
        store,
        provider_factory,
        config.clone(),
        cwd,
        10, // max concurrent jobs
    );

    eprintln!("[ok] Model: {}", config.model);
    eprintln!();
    eprintln!("Listening on {listen}");
    eprintln!("  REST API:   http://{listen}/api/v1/");
    eprintln!("  WebSocket:  ws://{listen}/api/v1/ws/{{session_id}}");
    eprintln!("  Health:     http://{listen}/api/v1/health");
    eprintln!();
    eprintln!("Press Ctrl+C to stop.");

    // 5. Start server with graceful shutdown
    cisco_code_server::routes::serve(state, listen).await?;

    eprintln!("Server stopped.");
    Ok(())
}

async fn run_attach(url: &str, session_override: Option<&str>) -> Result<()> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMsg;
    use cisco_code_server::websocket::{
        WsClientMessage, WsServerMessage,
    };

    eprintln!("cisco-code attach v{}", env!("CARGO_PKG_VERSION"));

    // Build URL — if session override given, replace the path
    let ws_url = if let Some(sid) = session_override {
        // Parse base URL and replace session_id
        let base = url.rsplit_once('/').map(|(b, _)| b).unwrap_or(url);
        format!("{base}/{sid}")
    } else {
        url.to_string()
    };

    eprintln!("Connecting to {ws_url}...");

    let (ws_stream, _response) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| anyhow::anyhow!("WebSocket connection failed: {e}"))?;

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Wait for Connected message
    let connected = loop {
        match ws_receiver.next().await {
            Some(Ok(WsMsg::Text(text))) => {
                let text = text.to_string();
                if let Ok(msg) = serde_json::from_str::<WsServerMessage>(&text) {
                    match msg {
                        WsServerMessage::Connected { session_id, server_version } => {
                            break (session_id, server_version);
                        }
                        WsServerMessage::Error { message, .. } => {
                            anyhow::bail!("Server error: {message}");
                        }
                        _ => continue,
                    }
                }
            }
            Some(Err(e)) => anyhow::bail!("WebSocket error: {e}"),
            None => anyhow::bail!("WebSocket closed before connected"),
            _ => continue,
        }
    };

    eprintln!("Connected to session {} (server v{})", connected.0, connected.1);
    eprintln!("Type your prompt, or Ctrl-D to disconnect.");
    eprintln!();

    let stdin = io::stdin();

    loop {
        eprint!("attach> ");
        let _ = io::stderr().flush();

        let mut line = String::new();
        let bytes = stdin.read_line(&mut line)?;
        if bytes == 0 {
            eprintln!("\nDisconnecting...");
            break;
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        // Handle local commands
        if input == "/quit" || input == "/exit" {
            eprintln!("Disconnecting...");
            break;
        }

        if input == "/status" {
            let msg = serde_json::to_string(&WsClientMessage::StatusRequest)?;
            ws_sender.send(WsMsg::Text(msg.into())).await?;

            // Wait for status response
            if let Some(Ok(WsMsg::Text(text))) = ws_receiver.next().await {
                let text = text.to_string();
                if let Ok(WsServerMessage::Status { job_id, status, turns }) =
                    serde_json::from_str(&text)
                {
                    eprintln!(
                        "Status: {} | Job: {} | Turns: {}",
                        status,
                        job_id.as_deref().unwrap_or("none"),
                        turns
                    );
                }
            }
            continue;
        }

        if input == "/cancel" {
            let msg = serde_json::to_string(&WsClientMessage::Cancel)?;
            ws_sender.send(WsMsg::Text(msg.into())).await?;
            eprintln!("[cancelled]");
            continue;
        }

        // Send user message
        let client_msg = WsClientMessage::UserMessage {
            content: input.to_string(),
            attachments: vec![],
        };
        let json = serde_json::to_string(&client_msg)?;
        ws_sender.send(WsMsg::Text(json.into())).await?;

        // Receive and render events until the turn ends
        let mut turn_complete = false;
        while !turn_complete {
            match ws_receiver.next().await {
                Some(Ok(WsMsg::Text(text))) => {
                    let text = text.to_string();
                    match serde_json::from_str::<WsServerMessage>(&text) {
                        Ok(WsServerMessage::Event { event }) => {
                            match &event {
                                StreamEvent::TextDelta { text } => {
                                    print!("{text}");
                                    let _ = io::stdout().flush();
                                }
                                StreamEvent::TurnStart { model, turn_number } => {
                                    if *turn_number > 1 {
                                        eprintln!("\n--- turn {turn_number} ({model}) ---\n");
                                    }
                                }
                                StreamEvent::ToolUseStart { tool_name, .. } => {
                                    eprintln!("\n[tool: {tool_name}]");
                                }
                                StreamEvent::ToolExecutionStart { description, .. } => {
                                    eprintln!("  > {description}");
                                }
                                StreamEvent::ToolResult { result, is_error, .. } => {
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
                                    if *stop_reason != StopReason::ToolUse {
                                        turn_complete = true;
                                    }
                                }
                                StreamEvent::PermissionRequest { tool_name, input_summary, .. } => {
                                    eprintln!("  [permission] {tool_name}: {input_summary}");
                                    // Auto-approve in attach mode (server runs with bypass)
                                }
                                StreamEvent::Error { message, recoverable } => {
                                    if *recoverable {
                                        eprintln!("[warning] {message}");
                                    } else {
                                        eprintln!("[error] {message}");
                                        turn_complete = true;
                                    }
                                }
                                _ => {}
                            }
                        }
                        Ok(WsServerMessage::Error { message, .. }) => {
                            eprintln!("[server error] {message}");
                            turn_complete = true;
                        }
                        Ok(WsServerMessage::Pong) => {}
                        _ => {}
                    }
                }
                Some(Ok(WsMsg::Close(_))) => {
                    eprintln!("\nServer closed connection.");
                    return Ok(());
                }
                Some(Err(e)) => {
                    eprintln!("\n[ws error] {e}");
                    return Ok(());
                }
                None => {
                    eprintln!("\nConnection closed.");
                    return Ok(());
                }
                _ => {}
            }
        }
        println!(); // newline after streamed output
    }

    // Close WebSocket gracefully
    let _ = ws_sender.send(WsMsg::Close(None)).await;
    eprintln!("Disconnected.");
    Ok(())
}

async fn run_daemon(
    slack: bool,
    webex: bool,
    cron: bool,
    listen: Option<&str>,
    cli: &Cli,
) -> Result<()> {
    use std::sync::Arc;
    use cisco_code_runtime::{
        SqliteStore, Store, SessionRouter, TriggerEvent,
        event_bus,
        channels::{ChannelManager, SlackChannel, SlackChannelConfig, WebexChannel, WebexChannelConfig},
    };
    use cisco_code_server::{AppState, DefaultProviderFactory};
    use futures::StreamExt;

    eprintln!("cisco-code daemon v{}", env!("CARGO_PKG_VERSION"));
    eprintln!();

    if !slack && !webex && !cron {
        anyhow::bail!(
            "No channels enabled. Use --slack, --webex, and/or --cron.\n\
             Example: cisco-code daemon --slack --cron"
        );
    }

    // 1. Init SQLite store
    let db_dir = dirs_next::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".cisco-code");
    std::fs::create_dir_all(&db_dir)?;
    let db_path = db_dir.join("cisco-code.db");
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open(&db_path.to_string_lossy())?);
    eprintln!("[ok] Store: {}", db_path.display());

    // 2. Discover providers
    let provider_factory = Arc::new(DefaultProviderFactory::auto_discover()?);
    eprintln!("[ok] Provider factory initialized");

    // 3. Build config
    let mut config = build_config(cli);
    config.permission_mode = cisco_code_runtime::PermissionMode::BypassPermissions;
    eprintln!("[ok] Model: {}", config.model);

    // 4. Session router
    let router = SessionRouter::new(store.clone());

    // 5. Event bus
    let (event_tx, mut event_rx) = event_bus(256);

    // 6. Start channels
    let channel_mgr = Arc::new(ChannelManager::new());

    if slack {
        match SlackChannelConfig::from_env() {
            Ok(cfg) => {
                channel_mgr.add(Box::new(SlackChannel::new(cfg))).await;
                eprintln!("[ok] Slack channel enabled");
            }
            Err(e) => eprintln!("[!!] Slack: {e}"),
        }
    }

    if webex {
        match WebexChannelConfig::from_env() {
            Ok(cfg) => {
                channel_mgr.add(Box::new(WebexChannel::new(cfg))).await;
                eprintln!("[ok] Webex channel enabled");
            }
            Err(e) => eprintln!("[!!] Webex: {e}"),
        }
    }

    // Start channel streams and forward into event bus
    let channel_tx = event_tx.clone();
    let channel_mgr_clone = channel_mgr.clone();
    let channel_handle = tokio::spawn(async move {
        match channel_mgr_clone.start_all().await {
            Ok(mut stream) => {
                while let Some(msg) = stream.next().await {
                    if channel_tx
                        .send(TriggerEvent::ChannelMessage(msg))
                        .await
                        .is_err()
                    {
                        break; // event bus closed
                    }
                }
            }
            Err(e) => {
                tracing::error!("Channel stream failed: {e}");
            }
        }
    });

    // 7. Cron scheduler
    let cron_handle = if cron {
        let cron_tx = event_tx.clone();
        let cron_store = store.clone();
        eprintln!("[ok] Cron scheduler enabled");

        Some(tokio::spawn(async move {
            // Load jobs from store
            let jobs = match cron_store.list_cron_jobs().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!("Failed to load cron jobs: {e}");
                    return;
                }
            };

            if jobs.is_empty() {
                tracing::info!("No cron jobs found — scheduler idle");
            } else {
                tracing::info!("Loaded {} cron jobs", jobs.len());
            }

            // Simple tick loop — check every 30 seconds
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                let now = chrono::Utc::now();

                let jobs = match cron_store.list_cron_jobs().await {
                    Ok(j) => j,
                    Err(_) => continue,
                };

                for job in &jobs {
                    if !job.enabled {
                        continue;
                    }
                    if let Some(next) = job.next_run {
                        if now >= next {
                            tracing::info!("Cron fired: {} ({})", job.name, job.id);
                            let _ = cron_tx
                                .send(TriggerEvent::CronFired { job: job.clone() })
                                .await;

                            // Update run state
                            let next_run = cisco_code_runtime::cron::compute_next_run(
                                &job.schedule,
                                &now,
                            );
                            let _ = cron_store
                                .update_cron_run(
                                    &job.id,
                                    now,
                                    next_run,
                                    job.run_count + 1,
                                )
                                .await;
                        }
                    }
                }
            }
        }))
    } else {
        None
    };

    // 8. Optionally start HTTP/WS server
    let server_handle = if let Some(addr) = listen {
        let state = AppState::new(
            store.clone(),
            provider_factory.clone(),
            config.clone(),
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".into()),
            10,
        );
        let addr = addr.to_string();
        eprintln!("[ok] HTTP/WS server on {addr}");
        Some(tokio::spawn(async move {
            if let Err(e) = cisco_code_server::routes::serve(state, &addr).await {
                tracing::error!("Server error: {e}");
            }
        }))
    } else {
        None
    };

    eprintln!();
    eprintln!("Daemon running. Press Ctrl+C to stop.");

    // 9. Signal handler
    let shutdown_tx = event_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        let _ = shutdown_tx.send(TriggerEvent::Shutdown).await;
    });

    // 10. Main event loop
    while let Some(event) = event_rx.recv().await {
        match event {
            TriggerEvent::ChannelMessage(msg) => {
                let session_id = router
                    .resolve_or_create(&msg.user_id, &msg.channel, msg.thread_id.as_deref())
                    .await?;

                tracing::info!(
                    channel = %msg.channel,
                    user = %msg.user_id,
                    session = %session_id,
                    "Processing message"
                );

                // Create provider + runtime for this message
                let provider = provider_factory.create(&config.model).await?;
                let tools = ToolRegistry::with_builtins()?;
                let mut runtime = cisco_code_runtime::ConversationRuntime::with_store(
                    provider,
                    tools,
                    config.clone(),
                    store.clone(),
                    Some(&session_id),
                )
                .await?;

                match runtime.submit_message(&msg.content).await {
                    Ok(events) => {
                        // Extract text from events
                        let response_text: String = events
                            .iter()
                            .filter_map(|e| match e {
                                StreamEvent::TextDelta { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect();

                        if !response_text.is_empty() {
                            let mut response = cisco_code_runtime::channels::OutgoingResponse::text(&response_text);
                            if let Some(ref tid) = msg.thread_id {
                                response = response.in_thread(tid.clone());
                            }
                            if let Err(e) = channel_mgr.respond(&msg, response).await {
                                tracing::error!("Failed to respond: {e}");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(session = %session_id, "Agent error: {e}");
                        let error_resp = cisco_code_runtime::channels::OutgoingResponse::text(
                            format!("Error: {e}"),
                        );
                        let _ = channel_mgr.respond(&msg, error_resp).await;
                    }
                }
            }

            TriggerEvent::CronFired { job } => {
                tracing::info!(cron_id = %job.id, name = %job.name, "Executing cron job");

                let _cwd = job.cwd.clone().unwrap_or_else(|| ".".into());
                let model = job.model.clone().unwrap_or_else(|| config.model.clone());

                let provider = match provider_factory.create(&model).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!(cron_id = %job.id, "Provider error: {e}");
                        continue;
                    }
                };

                let tools = ToolRegistry::with_builtins()?;
                let mut cron_config = config.clone();
                cron_config.model = model;

                let mut runtime = cisco_code_runtime::ConversationRuntime::with_store(
                    provider,
                    tools,
                    cron_config,
                    store.clone(),
                    None, // new session per cron execution
                )
                .await?;

                match runtime.submit_message(&job.prompt).await {
                    Ok(_events) => {
                        tracing::info!(
                            cron_id = %job.id,
                            turns = runtime.turn_count(),
                            "Cron job completed"
                        );
                    }
                    Err(e) => {
                        tracing::error!(cron_id = %job.id, "Cron execution failed: {e}");
                    }
                }
            }

            TriggerEvent::WebhookReceived { source, payload } => {
                tracing::info!(source = %source, "Webhook received (not yet handled)");
                let _ = payload; // future: route to appropriate handler
            }

            TriggerEvent::Shutdown => {
                eprintln!("\nShutting down daemon...");
                break;
            }
        }
    }

    // Cleanup
    channel_mgr.shutdown_all().await?;
    channel_handle.abort();
    if let Some(h) = cron_handle {
        h.abort();
    }
    if let Some(h) = server_handle {
        h.abort();
    }

    eprintln!("Daemon stopped.");
    Ok(())
}

async fn run_login(with_api_key: bool) -> Result<()> {
    if with_api_key {
        println!("Set API keys as environment variables:");
        println!();
        println!("  Bedrock:   export AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=...");
        println!("  OpenAI:    export OPENAI_API_KEY=sk-...");
        println!("  Anthropic: export ANTHROPIC_API_KEY=sk-ant-...");
        println!();
        println!("Or add them to your shell profile (~/.zshrc, ~/.bashrc).");
        return Ok(());
    }

    // OpenAI Codex OAuth — Device Code Flow
    let auth = CodexAuth::new();

    if auth.has_tokens() {
        println!("Already logged in (OAuth tokens found).");
        println!("Run `cisco-code logout` to clear credentials, then login again.");
        return Ok(());
    }

    println!("cisco-code — OpenAI OAuth Login");
    println!();

    let device = auth.request_device_code().await?;

    println!("Go to:     https://auth.openai.com/codex/device");
    println!("Enter code: {}", device.user_code);
    println!();
    println!("Waiting for authorization (this may take a moment)...");

    let tokens = auth
        .poll_for_auth(&device.device_auth_id, &device.user_code, device.interval)
        .await?;

    println!();
    println!("Login successful!");
    if let Some(ref acct) = tokens.account_id {
        println!("Account: {acct}");
    }
    println!();
    println!("You can now use OpenAI models via: cisco-code --class medium");

    Ok(())
}
