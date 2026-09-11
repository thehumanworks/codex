//! Process-local ChatGPT credentials. No credential store is read or written here.

use std::env;
use std::io;

use chrono::Utc;

use super::manager::CodexAuth;
use crate::token_data::parse_chatgpt_jwt_claims;
use crate::token_data::parse_jwt_expiration;

pub use codex_protocol::shell_environment::CHATGPT_AUTH_TOKEN_ENV_VAR;
pub const CHATGPT_ACCOUNT_ID_ENV_VAR: &str = "CHATGPT_ACCOUNT_ID";

/// Whether this process was launched with an environment-managed ChatGPT token.
/// Invalid Unicode is considered configured so callers can report it, not fall back.
pub fn is_chatgpt_auth_token_configured() -> bool {
    env::var_os(CHATGPT_AUTH_TOKEN_ENV_VAR)
        .is_some_and(|value| value.to_str().is_none_or(|value| !value.trim().is_empty()))
}

pub(super) fn auth_from_env() -> io::Result<Option<CodexAuth>> {
    let Some(token) = read_env(CHATGPT_AUTH_TOKEN_ENV_VAR)? else {
        return Ok(None);
    };
    let account_id = read_env(CHATGPT_ACCOUNT_ID_ENV_VAR)?;
    auth_from_values(&token, account_id.as_deref()).map(Some)
}

fn read_env(name: &str) -> io::Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value.trim().to_owned()).filter(|value| !value.is_empty())),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => {
            Err(invalid(format!("{name} must contain valid Unicode")))
        }
    }
}

fn auth_from_values(token: &str, account_id: Option<&str>) -> io::Result<CodexAuth> {
    let token = token.trim();
    if token.split('.').count() != 3
        || token
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || !byte.is_ascii_graphic())
    {
        return Err(invalid(
            "CHATGPT_AUTH_TOKEN must be a ChatGPT access-token JWT (without a Bearer prefix)",
        ));
    }
    // Decode metadata only. The service remains responsible for verifying the token.
    // Do not include parser errors: malformed claim values may contain credentials.
    let claims = parse_chatgpt_jwt_claims(token)
        .map_err(|_| invalid("CHATGPT_AUTH_TOKEN is not a valid ChatGPT access-token JWT"))?;
    let expires_at = parse_jwt_expiration(token)
        .map_err(|_| invalid("CHATGPT_AUTH_TOKEN contains an invalid expiration claim"))?;
    if expires_at.is_some_and(|expires_at| expires_at <= Utc::now()) {
        return Err(invalid(
            "CHATGPT_AUTH_TOKEN has expired; supply a fresh access token and restart Codex",
        ));
    }
    let account_id = account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| claims.chatgpt_account_id.as_deref().map(str::trim).filter(|value| !value.is_empty()))
        .ok_or_else(|| invalid("CHATGPT_AUTH_TOKEN does not contain a ChatGPT account ID; set CHATGPT_ACCOUNT_ID for the intended workspace"))?;
    if !account_id.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(invalid(
            "CHATGPT_ACCOUNT_ID must be a non-empty account ID without whitespace or control characters",
        ));
    }
    CodexAuth::from_external_chatgpt_tokens(token, account_id, /*chatgpt_plan_type*/ None)
        .map_err(|_| invalid("CHATGPT_AUTH_TOKEN could not be loaded"))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
#[path = "chatgpt_env_tests.rs"]
mod tests;
