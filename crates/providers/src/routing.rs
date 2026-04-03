//! Model tier routing — assigns the right model for each task.
//!
//! Design insight from Astro-Assistant: 5-tier routing saves 80%+ costs.
//! Title generation uses SMALL (~$0.001/1K), main agent uses LARGE (~$0.015/1K).

use std::collections::HashMap;

use crate::{Role, Tier};

/// Default role → tier mapping.
pub fn default_role_tiers() -> HashMap<Role, Tier> {
    HashMap::from([
        (Role::MainAgent, Tier::Large),
        (Role::Planner, Tier::Large),
        (Role::Executor, Tier::Large),
        (Role::Reviewer, Tier::Teammate),
        (Role::Compaction, Tier::Medium),
        (Role::Classifier, Tier::Small),
        (Role::Title, Tier::Small),
        (Role::Guardian, Tier::Small),
    ])
}

/// Tier configuration — maps tiers to specific model identifiers.
#[derive(Debug, Clone)]
pub struct TierConfig {
    pub small: String,
    pub medium: String,
    pub large: String,
    pub teammate: String,
    pub frontier: String,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            small: "anthropic/claude-haiku-4-5-20251001".to_string(),
            medium: "anthropic/claude-sonnet-4-6".to_string(),
            large: "anthropic/claude-opus-4-6".to_string(),
            teammate: "openai/gpt-5".to_string(),
            frontier: "anthropic/claude-opus-4-6".to_string(),
        }
    }
}

/// Task router — resolves roles to specific models.
pub struct TaskRouter {
    pub tier_config: TierConfig,
    pub role_tiers: HashMap<Role, Tier>,
}

impl TaskRouter {
    pub fn new(tier_config: TierConfig) -> Self {
        Self {
            tier_config,
            role_tiers: default_role_tiers(),
        }
    }

    /// Resolve a role to a specific model identifier.
    pub fn resolve(&self, role: Role) -> &str {
        let tier = self.role_tiers.get(&role).copied().unwrap_or(Tier::Large);
        match tier {
            Tier::Small => &self.tier_config.small,
            Tier::Medium => &self.tier_config.medium,
            Tier::Large => &self.tier_config.large,
            Tier::Teammate => &self.tier_config.teammate,
            Tier::Frontier => &self.tier_config.frontier,
        }
    }
}

impl Default for TaskRouter {
    fn default() -> Self {
        Self::new(TierConfig::default())
    }
}
