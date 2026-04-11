//! Provider setup with credential gating for E2E tests.

use cisco_code_api::bedrock::BedrockClient;

pub const HAIKU_MODEL: &str = "us.anthropic.claude-sonnet-4-6";
pub const E2E_MAX_TOKENS: u32 = 1024;
pub const E2E_TEMPERATURE: f64 = 0.0;
pub const E2E_MAX_TURNS: u32 = 5;

/// Try to create a BedrockClient from environment variables.
/// Returns None if AWS credentials are not set.
pub fn bedrock_client() -> Option<BedrockClient> {
    if std::env::var("AWS_ACCESS_KEY_ID").is_err()
        || std::env::var("AWS_SECRET_ACCESS_KEY").is_err()
    {
        eprintln!("Skipping E2E test: AWS credentials not set (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY)");
        return None;
    }
    BedrockClient::from_env().ok()
}

/// Macro to skip a test if AWS credentials are not available.
/// Usage: `let client = skip_without_bedrock!();`
#[macro_export]
macro_rules! skip_without_bedrock {
    () => {
        match $crate::helpers::provider::bedrock_client() {
            Some(c) => c,
            None => return,
        }
    };
}

/// Default timeout for E2E tests (60 seconds).
pub const TEST_TIMEOUT_SECS: u64 = 60;
