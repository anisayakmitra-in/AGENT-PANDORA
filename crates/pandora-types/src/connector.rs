use serde::{Deserialize, Serialize};

/// Supported SaaS tool connector providers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectorProvider {
    GitHub,
    Linear,
    Notion,
    Asana,
    Custom,
}

impl ConnectorProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::Linear => "linear",
            Self::Notion => "notion",
            Self::Asana => "asana",
            Self::Custom => "custom",
        }
    }

    pub fn default_auth_endpoint(&self) -> &'static str {
        match self {
            Self::GitHub => "https://github.com/login/oauth/authorize",
            Self::Linear => "https://linear.app/oauth/authorize",
            Self::Notion => "https://api.notion.com/v1/oauth/authorize",
            Self::Asana => "https://app.asana.com/-/oauth_authorize",
            Self::Custom => "https://oauth.example.com/authorize",
        }
    }
}

impl std::str::FromStr for ConnectorProvider {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "github" => Ok(Self::GitHub),
            "linear" => Ok(Self::Linear),
            "notion" => Ok(Self::Notion),
            "asana" => Ok(Self::Asana),
            "custom" => Ok(Self::Custom),
            _ => Err(()),
        }
    }
}

/// Standards-compliant OAuth 2.1 Proof Key for Code Exchange (PKCE) challenge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PkceChallenge {
    pub code_verifier: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
}

/// Active PKCE authorization session awaiting browser authorization and loopback code exchange.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectorSession {
    pub session_id: String,
    pub provider: ConnectorProvider,
    pub client_id: String,
    pub authorization_url: String,
    pub pkce: PkceChallenge,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub created_at: u64,
    pub expires_at: u64,
}

/// Authorized connector credential record. The raw token is stored in the encrypted vault.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectorTokenRecord {
    pub connector_id: String,
    pub provider: ConnectorProvider,
    pub token_masked: String,
    pub token_type: String,
    pub scopes: Vec<String>,
    pub secret_key_id: String,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}
