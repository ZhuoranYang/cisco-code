//! cisco-code-planning: Plan mode for the agent loop.
//!
//! Matches Claude Code v2.1.88's plan mode architecture:
//! - Plan mode is a permission mode variant (read-only, no code changes)
//! - Plans stored as markdown files at `~/.cisco-code/plans/{slug}.md`
//! - Slug is a random word-based identifier (adjective-noun-verb)
//! - EnterPlanMode/ExitPlanMode are tools that transition modes
//! - PlanManager handles slug caching, file I/O, resume/fork

pub mod plan;
pub mod manager;

pub use plan::{
    PlanModeState, PlanSlugCache,
    plan_file_path, read_plan, resolve_plans_directory, write_plan,
};
pub use manager::PlanManager;
