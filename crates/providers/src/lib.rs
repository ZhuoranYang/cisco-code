//! cisco-code-providers: LLM provider discovery and model routing.
//!
//! Supports three model classes (Small, Medium, Large) that map to
//! specific provider + model combinations. Auto-discovers available
//! providers from environment variables.
//!
//! Backends:
//! - AWS Bedrock (Claude models via SigV4 auth)
//! - OpenAI (GPT models via API key / Cisco OAuth)
//! - Anthropic (Claude models via direct API)

pub mod registry;
pub mod routing;

pub use registry::*;
pub use routing::*;

/// Model capability classes — user picks one for the main agent,
/// subagents are dispatched by class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelClass {
    /// Fast, cheap — titles, classification, simple queries
    Small,
    /// Balanced — compaction, code review, summarization
    Medium,
    /// Most capable — main agent, complex coding, architecture
    Large,
}

impl ModelClass {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "small" | "s" | "sm" => Some(Self::Small),
            "medium" | "m" | "med" => Some(Self::Medium),
            "large" | "l" | "lg" => Some(Self::Large),
            _ => None,
        }
    }
}

impl std::fmt::Display for ModelClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Small => write!(f, "small"),
            Self::Medium => write!(f, "medium"),
            Self::Large => write!(f, "large"),
        }
    }
}

impl Default for ModelClass {
    fn default() -> Self {
        Self::Medium
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_class_from_str() {
        assert_eq!(ModelClass::from_str_loose("small"), Some(ModelClass::Small));
        assert_eq!(ModelClass::from_str_loose("s"), Some(ModelClass::Small));
        assert_eq!(ModelClass::from_str_loose("MEDIUM"), Some(ModelClass::Medium));
        assert_eq!(ModelClass::from_str_loose("m"), Some(ModelClass::Medium));
        assert_eq!(ModelClass::from_str_loose("large"), Some(ModelClass::Large));
        assert_eq!(ModelClass::from_str_loose("L"), Some(ModelClass::Large));
        assert_eq!(ModelClass::from_str_loose("unknown"), None);
    }

    #[test]
    fn test_model_class_display() {
        assert_eq!(ModelClass::Small.to_string(), "small");
        assert_eq!(ModelClass::Medium.to_string(), "medium");
        assert_eq!(ModelClass::Large.to_string(), "large");
    }

    #[test]
    fn test_model_class_default() {
        assert_eq!(ModelClass::default(), ModelClass::Medium);
    }

    #[test]
    fn test_model_class_serde_roundtrip() {
        let class = ModelClass::Large;
        let json = serde_json::to_string(&class).unwrap();
        assert_eq!(json, "\"large\"");
        let parsed: ModelClass = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ModelClass::Large);
    }
}
