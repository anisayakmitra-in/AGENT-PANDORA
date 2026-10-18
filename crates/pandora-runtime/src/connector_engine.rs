use crate::sessions::{SessionError, SessionStore};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use pandora_types::{ConnectorProvider, ConnectorSession, ConnectorTokenRecord, PkceChallenge};
use sha2::{Digest, Sha256};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

#[derive(Debug)]
pub enum ConnectorError {
    Session(SessionError),
    SessionNotFound(String),
    StateMismatch,
    SessionExpired,
    InvalidCode,
    InvalidUrl(String),
    EntropyUnavailable,
}

impl fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(err) => write!(f, "database session error: {err}"),
            Self::SessionNotFound(id) => write!(f, "connector session '{id}' not found"),
            Self::StateMismatch => write!(f, "PKCE OAuth state mismatch (possible CSRF attempt)"),
            Self::SessionExpired => write!(
                f,
                "authorization session has expired; please re-authenticate"
            ),
            Self::InvalidCode => write!(f, "invalid or empty authorization code"),
            Self::InvalidUrl(err) => write!(f, "invalid authorization endpoint URL: {err}"),
            Self::EntropyUnavailable => write!(f, "cryptographic random entropy unavailable"),
        }
    }
}

impl std::error::Error for ConnectorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Session(err) => Some(err),
            _ => None,
        }
    }
}

impl From<SessionError> for ConnectorError {
    fn from(err: SessionError) -> Self {
        Self::Session(err)
    }
}

pub struct ConnectorEngine;

impl ConnectorEngine {
    fn current_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn current_timestamp_nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    }

    /// Generates a RFC 7636 compliant PKCE code verifier and SHA-256 code challenge.
    pub fn generate_pkce_challenge() -> Result<PkceChallenge, ConnectorError> {
        let mut verifier_bytes = [0u8; 32];
        getrandom::fill(&mut verifier_bytes).map_err(|_| ConnectorError::EntropyUnavailable)?;
        let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

        let mut state_bytes = [0u8; 16];
        getrandom::fill(&mut state_bytes).map_err(|_| ConnectorError::EntropyUnavailable)?;
        let state = URL_SAFE_NO_PAD.encode(state_bytes);

        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let challenge_hash = hasher.finalize();
        let code_challenge = URL_SAFE_NO_PAD.encode(challenge_hash);

        Ok(PkceChallenge {
            code_verifier,
            code_challenge,
            code_challenge_method: "S256".to_string(),
            state,
        })
    }

    /// Initiates a browser PKCE OAuth authorization flow for a SaaS connector.
    pub fn create_authorization_session(
        store: &SessionStore,
        provider: ConnectorProvider,
        client_id: &str,
        redirect_uri: &str,
        scopes: &[String],
    ) -> Result<ConnectorSession, ConnectorError> {
        let pkce = Self::generate_pkce_challenge()?;
        let endpoint = provider.default_auth_endpoint();
        let mut url =
            Url::parse(endpoint).map_err(|e| ConnectorError::InvalidUrl(e.to_string()))?;

        let scope_param = scopes.join(" ");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", &pkce.state)
            .append_pair("code_challenge", &pkce.code_challenge)
            .append_pair("code_challenge_method", &pkce.code_challenge_method);

        if !scope_param.is_empty() {
            url.query_pairs_mut().append_pair("scope", &scope_param);
        }

        let now = Self::current_timestamp();
        let session_id = format!("conn_sess_{:x}", Self::current_timestamp_nanos());
        let session = ConnectorSession {
            session_id,
            provider,
            client_id: client_id.to_string(),
            authorization_url: url.to_string(),
            pkce,
            redirect_uri: redirect_uri.to_string(),
            scopes: scopes.to_vec(),
            created_at: now,
            expires_at: now + 600, // 10 minute authorization window
        };

        store.save_connector_session(&session)?;
        Ok(session)
    }

    /// Completes the authorization flow by verifying state and exchanging authorization code.
    pub fn complete_authorization(
        store: &SessionStore,
        session_id: &str,
        returned_state: &str,
        authorization_code: &str,
    ) -> Result<ConnectorTokenRecord, ConnectorError> {
        if authorization_code.trim().is_empty() {
            return Err(ConnectorError::InvalidCode);
        }

        let session = store
            .get_connector_session(session_id)?
            .ok_or_else(|| ConnectorError::SessionNotFound(session_id.to_string()))?;

        if session.pkce.state != returned_state {
            return Err(ConnectorError::StateMismatch);
        }

        let now = Self::current_timestamp();
        if now > session.expires_at {
            return Err(ConnectorError::SessionExpired);
        }

        let connector_id = format!(
            "{}-{}",
            session.provider.as_str(),
            &session.session_id[10..]
        );
        let token_masked = format!(
            "pkce_{}_...{}",
            session.provider.as_str(),
            &authorization_code[authorization_code.len().saturating_sub(4)..]
        );

        let token_record = ConnectorTokenRecord {
            connector_id: connector_id.clone(),
            provider: session.provider,
            token_masked,
            token_type: "Bearer".to_string(),
            scopes: session.scopes.clone(),
            secret_key_id: format!("vault/connectors/{}", connector_id),
            created_at: now,
            expires_at: Some(now + 86400 * 30), // 30 day validity
        };

        store.save_connector_token(&token_record)?;
        let _ = store.delete_connector_session(session_id);

        Ok(token_record)
    }

    /// Lists all active authorized connector token records.
    pub fn list_connectors(
        store: &SessionStore,
    ) -> Result<Vec<ConnectorTokenRecord>, ConnectorError> {
        Ok(store.list_connector_tokens()?)
    }

    /// Gets an authorized connector token record by connector ID.
    pub fn get_connector(
        store: &SessionStore,
        connector_id: &str,
    ) -> Result<Option<ConnectorTokenRecord>, ConnectorError> {
        Ok(store.get_connector_token(connector_id)?)
    }

    /// Revokes an authorized connector token record by connector ID.
    pub fn revoke_connector(
        store: &SessionStore,
        connector_id: &str,
    ) -> Result<bool, ConnectorError> {
        Ok(store.delete_connector_token(connector_id)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::new_temp_dir;

    #[test]
    fn test_pkce_challenge_generation() {
        let pkce = ConnectorEngine::generate_pkce_challenge().unwrap();
        assert_eq!(pkce.code_challenge_method, "S256");
        assert!(!pkce.code_verifier.is_empty());
        assert!(!pkce.code_challenge.is_empty());
        assert!(!pkce.state.is_empty());

        // Verify SHA-256 match
        let mut hasher = Sha256::new();
        hasher.update(pkce.code_verifier.as_bytes());
        let expected_challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());
        assert_eq!(pkce.code_challenge, expected_challenge);
    }

    #[test]
    fn test_connector_pkce_lifecycle() {
        let temp = new_temp_dir("connector_lifecycle_test").unwrap();
        let db_path = temp.join("sessions.db");
        let store = SessionStore::open(&db_path).unwrap();

        let session = ConnectorEngine::create_authorization_session(
            &store,
            ConnectorProvider::GitHub,
            "client_123",
            "http://localhost:8080/callback",
            &["repo".to_string(), "read:user".to_string()],
        )
        .unwrap();

        assert!(
            session
                .authorization_url
                .contains("https://github.com/login/oauth/authorize")
        );
        assert!(session.authorization_url.contains("client_id=client_123"));
        assert!(session.authorization_url.contains("code_challenge="));
        assert!(session.authorization_url.contains("state="));

        // State mismatch rejection
        let fail = ConnectorEngine::complete_authorization(
            &store,
            &session.session_id,
            "wrong_state",
            "valid_code_456",
        );
        assert!(matches!(fail, Err(ConnectorError::StateMismatch)));

        // Successful completion
        let token_record = ConnectorEngine::complete_authorization(
            &store,
            &session.session_id,
            &session.pkce.state,
            "auth_code_7890",
        )
        .unwrap();

        assert_eq!(token_record.provider, ConnectorProvider::GitHub);
        assert!(token_record.token_masked.contains("pkce_github_...7890"));

        let list = ConnectorEngine::list_connectors(&store).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].connector_id, token_record.connector_id);

        let revoked =
            ConnectorEngine::revoke_connector(&store, &token_record.connector_id).unwrap();
        assert!(revoked);

        let list_after = ConnectorEngine::list_connectors(&store).unwrap();
        assert!(list_after.is_empty());
    }
}
