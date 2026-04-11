//! OAuth token management for OpenAI Codex (ChatGPT subscription).
//!
//! Implements the OAuth Device Code flow (RFC 8628) for OpenAI's Codex
//! authentication. Supports file-based token storage, auto-refresh with
//! 60s safety margin, and the interactive device code login flow.
//!
//! Pattern from astro-assistant: Device code → poll → exchange → refresh.
//! Storage: ~/.config/cisco-code/auth.json (0o600 permissions).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_ISSUER: &str = "https://auth.openai.com";

/// Codex API endpoint — all OAuth requests route here.
pub const CODEX_API_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";

/// Token store key for OpenAI Codex tokens.
const OPENAI_CODEX_KEY: &str = "openai_codex";

// ---------------------------------------------------------------------------
// OAuth token types
// ---------------------------------------------------------------------------

/// Stored OAuth tokens for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub expires_at: f64,
    #[serde(default)]
    pub account_id: Option<String>,
}

impl OAuthTokens {
    /// Check if the token is expired (with 60s safety margin).
    pub fn is_expired(&self) -> bool {
        now_secs() >= (self.expires_at - 60.0)
    }
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ---------------------------------------------------------------------------
// File-based token store
// ---------------------------------------------------------------------------

/// File-based token store at ~/.config/cisco-code/auth.json.
///
/// Stores tokens per provider in a single JSON file.
/// File permissions are set to 0o600 (user-only read/write).
pub struct TokenStore {
    path: PathBuf,
}

impl TokenStore {
    pub fn new() -> Self {
        Self {
            path: Self::default_path(),
        }
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home)
            .join(".config")
            .join("cisco-code")
            .join("auth.json")
    }

    /// Load tokens for a provider. Returns None if not found.
    pub fn load(&self, provider: &str) -> Result<Option<OAuthTokens>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&self.path)?;
        let store: serde_json::Value = serde_json::from_str(&content)?;
        match store.get(provider) {
            Some(v) => Ok(Some(serde_json::from_value(v.clone())?)),
            None => Ok(None),
        }
    }

    /// Save tokens for a provider. Creates the file and directories if needed.
    pub fn save(&self, provider: &str, tokens: &OAuthTokens) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut store: serde_json::Value = if self.path.exists() {
            let content = std::fs::read_to_string(&self.path)?;
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        store[provider] = serde_json::to_value(tokens)?;
        std::fs::write(&self.path, serde_json::to_string_pretty(&store)?)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &self.path,
                std::fs::Permissions::from_mode(0o600),
            );
        }

        Ok(())
    }

    /// Delete tokens for a provider.
    pub fn delete(&self, provider: &str) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let content = std::fs::read_to_string(&self.path)?;
        let mut store: serde_json::Value = serde_json::from_str(&content)?;
        if let Some(obj) = store.as_object_mut() {
            obj.remove(provider);
        }
        std::fs::write(&self.path, serde_json::to_string_pretty(&store)?)?;
        Ok(())
    }

    /// Check if tokens exist for a provider.
    pub fn has_tokens(&self, provider: &str) -> bool {
        self.load(provider).ok().flatten().is_some()
    }
}

// ---------------------------------------------------------------------------
// Device Code flow types
// ---------------------------------------------------------------------------

/// Response from the device authorization endpoint.
#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_auth_id: String,
    pub user_code: String,
    pub interval: u64,
}

// ---------------------------------------------------------------------------
// Codex Auth Manager
// ---------------------------------------------------------------------------

/// OpenAI Codex OAuth manager with token caching and auto-refresh.
///
/// Implements the Device Code flow for initial authentication and
/// automatic token refresh for ongoing use.
pub struct CodexAuth {
    http: reqwest::Client,
    store: TokenStore,
    cached: RwLock<Option<OAuthTokens>>,
}

impl CodexAuth {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            store: TokenStore::new(),
            cached: RwLock::new(None),
        }
    }

    #[cfg(test)]
    pub fn with_store(store: TokenStore) -> Self {
        Self {
            http: reqwest::Client::new(),
            store,
            cached: RwLock::new(None),
        }
    }

    /// Check if stored OAuth tokens exist.
    pub fn has_tokens(&self) -> bool {
        self.store.has_tokens(OPENAI_CODEX_KEY)
    }

    /// Get a valid access token, auto-refreshing if expired.
    ///
    /// Returns (access_token, optional account_id).
    pub async fn get_access_token(&self) -> Result<(String, Option<String>)> {
        // Fast path: check in-memory cache
        {
            let cached = self.cached.read().await;
            if let Some(ref tokens) = *cached {
                if !tokens.is_expired() {
                    return Ok((tokens.access_token.clone(), tokens.account_id.clone()));
                }
            }
        }

        // Slow path: acquire write lock and load/refresh
        let mut cached = self.cached.write().await;

        // Double-check after acquiring write lock (another task may have refreshed)
        if let Some(ref tokens) = *cached {
            if !tokens.is_expired() {
                return Ok((tokens.access_token.clone(), tokens.account_id.clone()));
            }
        }

        let stored = self.store.load(OPENAI_CODEX_KEY)?;
        let tokens = match stored {
            Some(t) if !t.is_expired() => t,
            Some(t) => {
                let refresh = t.refresh_token.ok_or_else(|| {
                    anyhow::anyhow!("Token expired, no refresh token. Run `cisco-code login`.")
                })?;
                let new_tokens = self.refresh_tokens(&refresh).await?;
                self.store.save(OPENAI_CODEX_KEY, &new_tokens)?;
                tracing::info!("OAuth token refreshed successfully");
                new_tokens
            }
            None => {
                anyhow::bail!(
                    "No OAuth tokens found. Run `cisco-code login` to authenticate."
                );
            }
        };

        let result = (tokens.access_token.clone(), tokens.account_id.clone());
        *cached = Some(tokens);
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Device Code Flow (for `cisco-code login`)
    // -----------------------------------------------------------------------

    /// Step 1: Request a device code for interactive login.
    pub async fn request_device_code(&self) -> Result<DeviceCodeResponse> {
        let resp = self
            .http
            .post(format!(
                "{OPENAI_ISSUER}/api/accounts/deviceauth/usercode"
            ))
            .json(&serde_json::json!({"client_id": OPENAI_CLIENT_ID}))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to request device code: {text}");
        }

        Ok(resp.json().await?)
    }

    /// Step 2: Poll for authorization completion.
    ///
    /// Blocks until the user completes the flow or the code expires.
    /// Returns tokens on success.
    pub async fn poll_for_auth(
        &self,
        device_auth_id: &str,
        user_code: &str,
        interval: u64,
    ) -> Result<OAuthTokens> {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

            let resp = self
                .http
                .post(format!(
                    "{OPENAI_ISSUER}/api/accounts/deviceauth/token"
                ))
                .json(&serde_json::json!({
                    "device_auth_id": device_auth_id,
                    "user_code": user_code,
                }))
                .send()
                .await?;

            let body: serde_json::Value = resp.json().await?;

            // Check for polling errors
            if let Some(error) = body["error"].as_str() {
                match error {
                    "authorization_pending" | "slow_down" => continue,
                    "expired_token" => {
                        anyhow::bail!("Device code expired. Please try again.")
                    }
                    other => anyhow::bail!("Authorization error: {other}"),
                }
            }

            // Success — exchange auth code for tokens
            if let Some(auth_code) = body["authorization_code"].as_str() {
                let code_verifier = body["code_verifier"].as_str().unwrap_or("");
                let tokens = self.exchange_code(auth_code, code_verifier).await?;
                self.store.save(OPENAI_CODEX_KEY, &tokens)?;
                return Ok(tokens);
            }

            anyhow::bail!("Unexpected auth response: {body}");
        }
    }

    // -----------------------------------------------------------------------
    // Token exchange and refresh
    // -----------------------------------------------------------------------

    /// Exchange an authorization code for access + refresh tokens.
    async fn exchange_code(&self, code: &str, code_verifier: &str) -> Result<OAuthTokens> {
        let resp = self
            .http
            .post(format!("{OPENAI_ISSUER}/oauth/token"))
            .json(&serde_json::json!({
                "grant_type": "authorization_code",
                "code": code,
                "redirect_uri": format!("{OPENAI_ISSUER}/deviceauth/callback"),
                "client_id": OPENAI_CLIENT_ID,
                "code_verifier": code_verifier,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Token exchange failed: {text}");
        }

        self.parse_token_response(resp.json().await?)
    }

    /// Refresh an expired access token using the refresh token.
    async fn refresh_tokens(&self, refresh_token: &str) -> Result<OAuthTokens> {
        let resp = self
            .http
            .post(format!("{OPENAI_ISSUER}/oauth/token"))
            .json(&serde_json::json!({
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
                "client_id": OPENAI_CLIENT_ID,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Token refresh failed: {text}");
        }

        self.parse_token_response(resp.json().await?)
    }

    /// Parse a token endpoint response into OAuthTokens.
    fn parse_token_response(&self, body: serde_json::Value) -> Result<OAuthTokens> {
        let expires_in = body["expires_in"].as_f64().unwrap_or(3600.0);

        // Extract account_id from id_token JWT claims (without verification)
        let account_id = body["id_token"]
            .as_str()
            .and_then(extract_jwt_claim_account_id);

        Ok(OAuthTokens {
            access_token: body["access_token"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("No access_token in response"))?
                .to_string(),
            refresh_token: body["refresh_token"].as_str().map(String::from),
            expires_at: now_secs() + expires_in,
            account_id,
        })
    }

    /// Clear stored tokens (logout).
    pub fn logout(&self) -> Result<()> {
        self.store.delete(OPENAI_CODEX_KEY)
    }
}

// ---------------------------------------------------------------------------
// JWT helpers (minimal, no verification — display only)
// ---------------------------------------------------------------------------

/// Extract chatgpt_account_id from a JWT's payload without verification.
fn extract_jwt_claim_account_id(jwt: &str) -> Option<String> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload = base64url_decode(parts[1])?;
    let json: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    json["chatgpt_account_id"].as_str().map(String::from)
}

/// Minimal base64url decoder (no external dependency needed).
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut s = input.replace('-', "+").replace('_', "/");
    while s.len() % 4 != 0 {
        s.push('=');
    }

    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [0xFFu8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }

    let bytes: Vec<u8> = s
        .bytes()
        .filter(|&b| b != b'=' && b != b'\n' && b != b'\r')
        .collect();
    let mut result = Vec::with_capacity(bytes.len() * 3 / 4);

    for chunk in bytes.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let a = lookup[chunk[0] as usize];
        let b = lookup[chunk[1] as usize];
        if a == 0xFF || b == 0xFF {
            return None;
        }
        result.push((a << 2) | (b >> 4));

        if chunk.len() > 2 {
            let c = lookup[chunk[2] as usize];
            if c == 0xFF {
                return None;
            }
            result.push(((b & 0x0f) << 4) | (c >> 2));

            if chunk.len() > 3 {
                let d = lookup[chunk[3] as usize];
                if d == 0xFF {
                    return None;
                }
                result.push(((c & 0x03) << 6) | d);
            }
        }
    }

    Some(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oauth_tokens_not_expired() {
        let tokens = OAuthTokens {
            access_token: "test".into(),
            refresh_token: None,
            expires_at: now_secs() + 3600.0,
            account_id: None,
        };
        assert!(!tokens.is_expired());
    }

    #[test]
    fn test_oauth_tokens_expired() {
        let tokens = OAuthTokens {
            access_token: "test".into(),
            refresh_token: None,
            expires_at: now_secs() - 100.0,
            account_id: None,
        };
        assert!(tokens.is_expired());
    }

    #[test]
    fn test_oauth_tokens_expired_within_margin() {
        let tokens = OAuthTokens {
            access_token: "test".into(),
            refresh_token: None,
            expires_at: now_secs() + 30.0, // Within 60s safety margin
            account_id: None,
        };
        assert!(tokens.is_expired());
    }

    #[test]
    fn test_token_store_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let store = TokenStore::with_path(path);

        let tokens = OAuthTokens {
            access_token: "access123".into(),
            refresh_token: Some("refresh456".into()),
            expires_at: 9999999999.0,
            account_id: Some("acct-789".into()),
        };

        store.save("test_provider", &tokens).unwrap();
        let loaded = store.load("test_provider").unwrap().unwrap();
        assert_eq!(loaded.access_token, "access123");
        assert_eq!(loaded.refresh_token.as_deref(), Some("refresh456"));
        assert_eq!(loaded.account_id.as_deref(), Some("acct-789"));
    }

    #[test]
    fn test_token_store_missing_provider() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let store = TokenStore::with_path(path);

        let tokens = OAuthTokens {
            access_token: "x".into(),
            refresh_token: None,
            expires_at: 0.0,
            account_id: None,
        };
        store.save("a", &tokens).unwrap();
        assert!(store.load("b").unwrap().is_none());
    }

    #[test]
    fn test_token_store_delete() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let store = TokenStore::with_path(path);

        let tokens = OAuthTokens {
            access_token: "x".into(),
            refresh_token: None,
            expires_at: 0.0,
            account_id: None,
        };
        store.save("p", &tokens).unwrap();
        assert!(store.has_tokens("p"));
        store.delete("p").unwrap();
        assert!(!store.has_tokens("p"));
    }

    #[test]
    fn test_token_store_nonexistent_file() {
        let store =
            TokenStore::with_path(PathBuf::from("/tmp/nonexistent_cisco_code_test/auth.json"));
        assert!(store.load("any").unwrap().is_none());
        assert!(!store.has_tokens("any"));
    }

    #[test]
    fn test_token_store_multiple_providers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        let store = TokenStore::with_path(path);

        let t1 = OAuthTokens {
            access_token: "a1".into(),
            refresh_token: None,
            expires_at: 0.0,
            account_id: None,
        };
        let t2 = OAuthTokens {
            access_token: "a2".into(),
            refresh_token: None,
            expires_at: 0.0,
            account_id: None,
        };

        store.save("provider1", &t1).unwrap();
        store.save("provider2", &t2).unwrap();

        assert_eq!(
            store.load("provider1").unwrap().unwrap().access_token,
            "a1"
        );
        assert_eq!(
            store.load("provider2").unwrap().unwrap().access_token,
            "a2"
        );
    }

    #[test]
    fn test_oauth_tokens_serde_roundtrip() {
        let tokens = OAuthTokens {
            access_token: "abc".into(),
            refresh_token: Some("def".into()),
            expires_at: 123456.789,
            account_id: Some("acct".into()),
        };
        let json = serde_json::to_string(&tokens).unwrap();
        let parsed: OAuthTokens = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, "abc");
        assert_eq!(parsed.refresh_token.as_deref(), Some("def"));
        assert_eq!(parsed.expires_at, 123456.789);
    }

    #[test]
    fn test_oauth_tokens_serde_defaults() {
        // Minimal JSON (only required fields)
        let json = r#"{"access_token":"tok","expires_at":100.0}"#;
        let parsed: OAuthTokens = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.access_token, "tok");
        assert!(parsed.refresh_token.is_none());
        assert!(parsed.account_id.is_none());
    }

    #[test]
    fn test_base64url_decode_hello() {
        let decoded = base64url_decode("aGVsbG8").unwrap();
        assert_eq!(decoded, b"hello");
    }

    #[test]
    fn test_base64url_decode_hello_world() {
        let decoded = base64url_decode("SGVsbG8gV29ybGQ").unwrap();
        assert_eq!(decoded, b"Hello World");
    }

    #[test]
    fn test_base64url_decode_with_url_safe_chars() {
        // Standard base64 "+/" are replaced with "-_" in URL-safe variant
        let standard = base64url_decode("abc-def_ghi").unwrap();
        assert!(!standard.is_empty());
    }

    #[test]
    fn test_extract_jwt_claim_valid() {
        // JWT with payload {"chatgpt_account_id":"acct-123"}
        // base64url("{"chatgpt_account_id":"acct-123"}") = eyJjaGF0Z3B0X2FjY291bnRfaWQiOiJhY2N0LTEyMyJ9
        let jwt = "eyJhbGciOiJub25lIn0.eyJjaGF0Z3B0X2FjY291bnRfaWQiOiJhY2N0LTEyMyJ9.sig";
        let account_id = extract_jwt_claim_account_id(jwt);
        assert_eq!(account_id.as_deref(), Some("acct-123"));
    }

    #[test]
    fn test_extract_jwt_claim_missing() {
        // JWT with payload {"sub":"123"} — no chatgpt_account_id
        let jwt = "eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMifQ.sig";
        assert!(extract_jwt_claim_account_id(jwt).is_none());
    }

    #[test]
    fn test_extract_jwt_claim_invalid() {
        assert!(extract_jwt_claim_account_id("not-a-jwt").is_none());
        assert!(extract_jwt_claim_account_id("").is_none());
        assert!(extract_jwt_claim_account_id("a").is_none());
    }

    #[test]
    fn test_codex_auth_no_tokens_initially() {
        let auth = CodexAuth::with_store(TokenStore::with_path(
            PathBuf::from("/tmp/nonexistent_test_cisco/auth.json"),
        ));
        assert!(!auth.has_tokens());
    }

    #[tokio::test]
    async fn test_codex_auth_get_token_no_stored() {
        let auth = CodexAuth::with_store(TokenStore::with_path(
            PathBuf::from("/tmp/nonexistent_test_cisco/auth.json"),
        ));
        let result = auth.get_access_token().await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No OAuth tokens found"));
    }

    #[tokio::test]
    async fn test_codex_auth_get_token_valid_cached() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::with_path(dir.path().join("auth.json"));

        let tokens = OAuthTokens {
            access_token: "valid-token".into(),
            refresh_token: Some("refresh".into()),
            expires_at: now_secs() + 3600.0,
            account_id: Some("acct-test".into()),
        };
        store.save("openai_codex", &tokens).unwrap();

        let auth = CodexAuth::with_store(store);
        let (token, acct) = auth.get_access_token().await.unwrap();
        assert_eq!(token, "valid-token");
        assert_eq!(acct.as_deref(), Some("acct-test"));
    }

    #[test]
    fn test_token_store_default_path() {
        let path = TokenStore::default_path();
        assert!(path.to_str().unwrap().contains("cisco-code"));
        assert!(path.to_str().unwrap().contains("auth.json"));
    }
}
