use serde::{Deserialize, Serialize};
use std::fmt;

const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_NAME_BYTES: usize = 256;
const MAX_URL_BYTES: usize = 2_048;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManifestError {
    EmptyField(&'static str),
    InvalidIdentifier(&'static str),
    FieldTooLong(&'static str),
    InvalidEnvironmentName,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::InvalidIdentifier(field) => write!(formatter, "invalid {field}"),
            Self::FieldTooLong(field) => write!(formatter, "{field} exceeds its size limit"),
            Self::InvalidEnvironmentName => {
                formatter.write_str("invalid credential environment name")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

macro_rules! define_provider_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ManifestError> {
                Ok(Self(validate_identifier($label, value.into())?))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

define_provider_id!(ProviderId, "provider identifier");
define_provider_id!(ModelId, "model identifier");

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    OpenAiCompatible,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderManifest {
    id: ProviderId,
    name: String,
    protocol: ProviderProtocol,
    base_url: String,
    default_model: ModelId,
    api_key_env: String,
}

impl ProviderManifest {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        base_url: impl Into<String>,
        default_model: impl Into<String>,
        api_key_env: impl Into<String>,
    ) -> Result<Self, ManifestError> {
        let name = bounded_text("provider name", name.into(), MAX_NAME_BYTES)?;
        let base_url = bounded_text("provider URL", base_url.into(), MAX_URL_BYTES)?;
        let api_key_env = api_key_env.into();
        if !is_environment_name(&api_key_env) {
            return Err(ManifestError::InvalidEnvironmentName);
        }
        Ok(Self {
            id: ProviderId::new(id)?,
            name,
            protocol: ProviderProtocol::OpenAiCompatible,
            base_url,
            default_model: ModelId::new(default_model)?,
            api_key_env,
        })
    }

    pub fn id(&self) -> &ProviderId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn protocol(&self) -> ProviderProtocol {
        self.protocol
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    pub fn api_key_env(&self) -> &str {
        &self.api_key_env
    }
}

fn validate_identifier(label: &'static str, value: String) -> Result<String, ManifestError> {
    if value.trim().is_empty() {
        return Err(ManifestError::EmptyField(label));
    }
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(ManifestError::FieldTooLong(label));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ManifestError::InvalidIdentifier(label));
    }
    Ok(value)
}

fn bounded_text(
    label: &'static str,
    value: String,
    max_bytes: usize,
) -> Result<String, ManifestError> {
    if value.trim().is_empty() {
        return Err(ManifestError::EmptyField(label));
    }
    if value.len() > max_bytes {
        return Err(ManifestError::FieldTooLong(label));
    }
    Ok(value)
}

fn is_environment_name(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_keeps_credentials_as_environment_references() {
        let manifest = ProviderManifest::new(
            "openai",
            "OpenAI",
            "https://api.openai.com/v1",
            "gpt-5",
            "PANDORA_OPENAI_API_KEY",
        )
        .unwrap();

        assert_eq!(manifest.id().as_str(), "openai");
        assert_eq!(manifest.default_model().as_str(), "gpt-5");
        assert_eq!(manifest.api_key_env(), "PANDORA_OPENAI_API_KEY");
        assert!(!format!("{manifest:?}").contains("sk-live"));
    }

    #[test]
    fn manifest_rejects_empty_identifiers() {
        assert!(
            ProviderManifest::new(
                "",
                "OpenAI",
                "https://api.openai.com/v1",
                "gpt-5",
                "PANDORA_OPENAI_API_KEY",
            )
            .is_err()
        );
    }
}
