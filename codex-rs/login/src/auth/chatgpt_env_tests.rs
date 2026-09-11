use super::*;
use base64::Engine;
use codex_protocol::auth::AuthMode;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;

fn jwt(claims: Value) -> String {
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
    format!("eyJhbGciOiJIUzI1NiJ9.{payload}.test-signature")
}

#[test]
fn derives_account_and_preserves_metadata_without_refresh_credentials() {
    let token = jwt(json!({
        "email": "test@example.com",
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "account-from-token",
            "chatgpt_plan_type": "pro",
            "chatgpt_user_id": "test-user"
        }
    }));
    let auth = auth_from_values(&token, None).unwrap();
    let expected =
        CodexAuth::from_external_chatgpt_tokens(&token, "account-from-token", None).unwrap();
    assert_eq!(auth.api_auth_mode(), AuthMode::ChatgptAuthTokens);
    assert_eq!(
        auth.get_token_data().unwrap(),
        expected.get_token_data().unwrap()
    );
    assert!(auth.get_token_data().unwrap().refresh_token.is_empty());
}

#[test]
fn explicit_account_overrides_claim_and_surrounding_whitespace_is_trimmed() {
    let token = jwt(json!({"https://api.openai.com/auth": {"chatgpt_account_id": "original"}}));
    let padded = format!(" \n{token}\t ");
    let auth = auth_from_values(&padded, Some(" selected-account \n")).unwrap();
    let expected =
        CodexAuth::from_external_chatgpt_tokens(&token, "selected-account", None).unwrap();
    assert_eq!(
        auth.get_token_data().unwrap(),
        expected.get_token_data().unwrap()
    );
}

#[test]
fn account_override_is_only_required_when_claim_is_missing() {
    for claims in [
        json!({}),
        json!({"https://api.openai.com/auth": {"chatgpt_account_id": " "}}),
    ] {
        let token = jwt(claims);
        let error = auth_from_values(&token, None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("set CHATGPT_ACCOUNT_ID"));
        let auth = auth_from_values(&token, Some("explicit-account")).unwrap();
        assert_eq!(auth.get_account_id(), Some("explicit-account".to_string()));
    }
}

#[test]
fn empty_override_uses_claim() {
    let token = jwt(json!({"https://api.openai.com/auth": {"chatgpt_account_id": "from-claim"}}));
    for account_id in [None, Some(""), Some(" \t")] {
        assert_eq!(
            auth_from_values(&token, account_id)
                .unwrap()
                .get_account_id(),
            Some("from-claim".to_string())
        );
    }
}

#[test]
fn rejects_invalid_jwts_without_echoing_credentials() {
    let private_claim = jwt(
        json!({"https://api.openai.com/auth": {"chatgpt_account_id": {"private-secret": "do-not-echo"}}}),
    );
    for token in [
        "opaque-private-secret",
        "a.b.c.d",
        "a.!!!.c",
        "Bearer a.b.c",
        "a.e30.c\r\ninjected",
        &private_claim,
    ] {
        let error = auth_from_values(token, Some("account")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("CHATGPT_AUTH_TOKEN"));
        assert!(!error.to_string().contains(token));
        assert!(!error.to_string().contains("private-secret"));
        assert!(!error.to_string().contains("do-not-echo"));
    }
}

#[test]
fn rejects_expired_and_invalid_expiration_claims() {
    for exp in [json!(0), json!(-1), json!("private-expiration")] {
        let token = jwt(json!({"exp": exp}));
        let error = auth_from_values(&token, Some("account")).unwrap_err();
        assert!(error.to_string().contains("expir"));
        assert!(!error.to_string().contains("private-expiration"));
    }
    let token = jwt(json!({"exp": 4102444800_i64}));
    assert!(auth_from_values(&token, Some("account")).is_ok());
}

#[test]
fn rejects_invalid_account_headers() {
    let token = jwt(json!({}));
    for account_id in [
        "account\r\nHeader:value",
        "account id",
        "account\tname",
        "accöunt",
    ] {
        let error = auth_from_values(&token, Some(account_id)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("CHATGPT_ACCOUNT_ID"));
        assert!(!error.to_string().contains(account_id));
    }
}
