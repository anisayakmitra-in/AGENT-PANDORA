use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use url::Url;

pub const CONFIG_FORMAT_VERSION: u32 = 1;

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    InvalidFile,
    InvalidProviderUrl,
    InvalidProviderName,
    InvalidProviderModel,
    InvalidCredentialEnvironment,
    InvalidProviderFallback,
    UnknownProvider,
    InvalidPath(&'static str),
    Serialization(serde_json::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => formatter.write_str("could not read or write configuration"),
            Self::InvalidFile => formatter.write_str("configuration file is invalid"),
            Self::InvalidProviderUrl => formatter.write_str("provider URL is invalid"),
            Self::InvalidProviderName => formatter.write_str("provider name is invalid"),
            Self::InvalidProviderModel => formatter.write_str("provider model is invalid"),
            Self::InvalidCredentialEnvironment => {
                formatter.write_str("provider credential environment is invalid")
            }
            Self::InvalidProviderFallback => {
                formatter.write_str("provider fallback configuration is invalid")
            }
            Self::UnknownProvider => formatter.write_str("provider is not configured"),
            Self::InvalidPath(field) => write!(formatter, "{field} path is invalid"),
            Self::Serialization(_) => formatter.write_str("could not serialize configuration"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Serialization(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub const DEFAULT_PROVIDER_NAME: &str = "openai-compatible";
pub const DEFAULT_PROVIDER_API_KEY_ENV: &str = "PANDORA_PROVIDER_API_KEY";
const TOKENS_PER_MILLION: u128 = 1_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderPricing {
    input_micros_per_million_tokens: u64,
    output_micros_per_million_tokens: u64,
}

impl ProviderPricing {
    pub const fn new(
        input_micros_per_million_tokens: u64,
        output_micros_per_million_tokens: u64,
    ) -> Self {
        Self {
            input_micros_per_million_tokens,
            output_micros_per_million_tokens,
        }
    }

    pub const fn input_micros_per_million_tokens(&self) -> u64 {
        self.input_micros_per_million_tokens
    }

    pub const fn output_micros_per_million_tokens(&self) -> u64 {
        self.output_micros_per_million_tokens
    }

    pub fn cost_micros(&self, input_tokens: u32, output_tokens: u32) -> Option<u64> {
        let input_cost = u128::from(input_tokens)
            .checked_mul(u128::from(self.input_micros_per_million_tokens))?;
        let output_cost = u128::from(output_tokens)
            .checked_mul(u128::from(self.output_micros_per_million_tokens))?;
        u64::try_from((input_cost + output_cost) / TOKENS_PER_MILLION).ok()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderProfile {
    name: String,
    base_url: String,
    model: String,
    api_key_env: String,
    fallback_provider: Option<String>,
    pricing: Option<ProviderPricing>,
}

impl ProviderProfile {
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key_env: impl Into<String>,
    ) -> Result<Self, ConfigError> {
        let name = name.into();
        if !is_identifier(&name) {
            return Err(ConfigError::InvalidProviderName);
        }
        let base_url = validate_provider_url(base_url.into())?;
        let model = model.into();
        if !is_identifier(&model) {
            return Err(ConfigError::InvalidProviderModel);
        }
        let api_key_env = api_key_env.into();
        if !is_environment_name(&api_key_env) {
            return Err(ConfigError::InvalidCredentialEnvironment);
        }
        Ok(Self {
            name,
            base_url,
            model,
            api_key_env,
            fallback_provider: None,
            pricing: None,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn api_key_env(&self) -> &str {
        &self.api_key_env
    }

    pub fn with_fallback_provider(mut self, name: impl Into<String>) -> Result<Self, ConfigError> {
        let name = name.into();
        if !is_identifier(&name) {
            return Err(ConfigError::InvalidProviderFallback);
        }
        self.fallback_provider = Some(name);
        Ok(self)
    }

    pub fn fallback_provider(&self) -> Option<&str> {
        self.fallback_provider.as_deref()
    }

    pub fn with_pricing(mut self, pricing: ProviderPricing) -> Self {
        self.pricing = Some(pricing);
        self
    }

    pub fn pricing(&self) -> Option<ProviderPricing> {
        self.pricing
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigOverrides {
    config_path: Option<PathBuf>,
    provider_name: Option<String>,
    provider_url: Option<String>,
    provider_model: Option<String>,
    data_dir: Option<PathBuf>,
    workspace_dir: Option<PathBuf>,
}

impl ConfigOverrides {
    pub fn with_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    pub fn with_provider_name(mut self, name: impl Into<String>) -> Self {
        self.provider_name = Some(name.into());
        self
    }

    pub fn with_provider_url(mut self, url: impl Into<String>) -> Self {
        self.provider_url = Some(url.into());
        self
    }

    pub fn with_provider_model(mut self, model: impl Into<String>) -> Self {
        self.provider_model = Some(model.into());
        self
    }

    pub fn with_data_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(path.into());
        self
    }

    pub fn with_workspace_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace_dir = Some(path.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeConfig {
    config_path: PathBuf,
    provider_profiles: BTreeMap<String, ProviderProfile>,
    active_provider: Option<String>,
    provider_url: Option<String>,
    provider_model: Option<String>,
    data_dir: PathBuf,
    workspace_dir: PathBuf,
}

impl RuntimeConfig {
    pub fn load(overrides: ConfigOverrides) -> Result<Self, ConfigError> {
        let environment = std::env::vars().collect::<BTreeMap<_, _>>();
        let config_path = overrides
            .config_path
            .clone()
            .or_else(|| environment.get("PANDORA_CONFIG").map(PathBuf::from))
            .unwrap_or_else(default_config_path);
        let default_data_dir = default_data_dir();
        let default_workspace_dir = std::env::current_dir()?;
        Self::from_sources(
            &overrides,
            &environment,
            &config_path,
            default_data_dir,
            default_workspace_dir,
        )
    }

    pub fn from_sources(
        overrides: &ConfigOverrides,
        environment: &BTreeMap<String, String>,
        config_path: &Path,
        default_data_dir: PathBuf,
        default_workspace_dir: PathBuf,
    ) -> Result<Self, ConfigError> {
        let file = if config_path.exists() {
            let bytes = fs::read(config_path)?;
            serde_json::from_slice::<FileConfig>(&bytes).map_err(|_| ConfigError::InvalidFile)?
        } else {
            FileConfig::default()
        };
        if file
            .format_version
            .is_some_and(|version| version > CONFIG_FORMAT_VERSION)
        {
            return Err(ConfigError::InvalidFile);
        }

        let provider_url = overrides
            .provider_url
            .clone()
            .or_else(|| environment.get("PANDORA_PROVIDER_URL").cloned())
            .or(file.provider_url)
            .map(validate_provider_url)
            .transpose()?;
        let provider_model = overrides
            .provider_model
            .clone()
            .or_else(|| environment.get("PANDORA_PROVIDER_MODEL").cloned())
            .or(file.provider_model);
        let mut provider_profiles = file
            .providers
            .into_iter()
            .map(|(name, file_profile)| {
                let mut profile = ProviderProfile::new(
                    name.clone(),
                    file_profile.base_url,
                    file_profile.model,
                    file_profile.api_key_env,
                )?;
                if let Some(fallback) = file_profile.fallback_provider {
                    profile = profile.with_fallback_provider(fallback)?;
                }
                if let Some(pricing) = file_profile.pricing {
                    profile = profile.with_pricing(pricing);
                }
                Ok((name, profile))
            })
            .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
        if let Some(base_url) = provider_url.as_ref() {
            let name = DEFAULT_PROVIDER_NAME.to_owned();
            if let std::collections::btree_map::Entry::Vacant(entry) =
                provider_profiles.entry(name.clone())
            {
                let profile = ProviderProfile::new(
                    name.clone(),
                    base_url.clone(),
                    provider_model
                        .clone()
                        .unwrap_or_else(|| "default".to_owned()),
                    DEFAULT_PROVIDER_API_KEY_ENV,
                )?;
                entry.insert(profile);
            }
        }
        let active_provider = overrides
            .provider_name
            .clone()
            .or_else(|| environment.get("PANDORA_PROVIDER").cloned())
            .or(file.active_provider)
            .or_else(|| {
                provider_profiles
                    .contains_key(DEFAULT_PROVIDER_NAME)
                    .then(|| DEFAULT_PROVIDER_NAME.to_owned())
            })
            .or_else(|| provider_profiles.keys().next().cloned());
        if let Some(name) = active_provider.as_ref()
            && !provider_profiles.contains_key(name)
        {
            return Err(ConfigError::UnknownProvider);
        }
        validate_fallbacks(&provider_profiles)?;
        let data_dir = resolve_path(
            overrides
                .data_dir
                .clone()
                .or_else(|| environment.get("PANDORA_DATA_DIR").map(PathBuf::from))
                .or(file.data_dir.map(PathBuf::from)),
            default_data_dir,
            config_path,
            "data",
        )?;
        let workspace_dir = resolve_path(
            overrides
                .workspace_dir
                .clone()
                .or_else(|| environment.get("PANDORA_WORKSPACE").map(PathBuf::from))
                .or(file.workspace_dir.map(PathBuf::from)),
            default_workspace_dir,
            config_path,
            "workspace",
        )?;

        Ok(Self {
            config_path: overrides
                .config_path
                .clone()
                .unwrap_or_else(|| config_path.to_path_buf()),
            provider_profiles,
            active_provider,
            provider_url,
            provider_model,
            data_dir,
            workspace_dir,
        })
    }

    pub fn write(&self) -> Result<(), ConfigError> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut data = serde_json::to_vec_pretty(&FileConfig {
            format_version: Some(CONFIG_FORMAT_VERSION),
            provider_url: self.provider_url.clone(),
            provider_model: self.provider_model.clone(),
            providers: self
                .provider_profiles
                .iter()
                .map(|(name, profile)| {
                    (
                        name.clone(),
                        FileProviderProfile {
                            base_url: profile.base_url.clone(),
                            model: profile.model.clone(),
                            api_key_env: profile.api_key_env.clone(),
                            fallback_provider: profile.fallback_provider.clone(),
                            pricing: profile.pricing,
                        },
                    )
                })
                .collect(),
            active_provider: self.active_provider.clone(),
            data_dir: Some(self.data_dir.display().to_string()),
            workspace_dir: Some(self.workspace_dir.display().to_string()),
        })
        .map_err(ConfigError::Serialization)?;
        validate_fallbacks(&self.provider_profiles)?;
        data.push(b'\n');
        write_atomic(&self.config_path, &data)
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn provider_names(&self) -> Vec<String> {
        self.provider_profiles.keys().cloned().collect()
    }

    pub fn active_provider(&self) -> Option<&str> {
        self.active_provider.as_deref()
    }

    pub fn provider_profile(&self, name: &str) -> Option<&ProviderProfile> {
        self.provider_profiles.get(name)
    }

    pub fn provider_cost_micros(
        &self,
        name: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> Option<u64> {
        let profile = self.provider_profiles.get(name)?;
        if profile.fallback_provider().is_some() {
            return None;
        }
        profile
            .pricing()
            .and_then(|pricing| pricing.cost_micros(input_tokens, output_tokens))
    }

    pub fn set_provider_profile(&mut self, profile: ProviderProfile) {
        if profile.name() == DEFAULT_PROVIDER_NAME {
            self.provider_url = Some(profile.base_url().to_owned());
            self.provider_model = Some(profile.model().to_owned());
        }
        if self.active_provider.is_none() {
            self.active_provider = Some(profile.name().to_owned());
        }
        self.provider_profiles
            .insert(profile.name().to_owned(), profile);
    }

    pub fn set_active_provider(&mut self, name: impl Into<String>) -> Result<(), ConfigError> {
        let name = name.into();
        if !self.provider_profiles.contains_key(&name) {
            return Err(ConfigError::UnknownProvider);
        }
        self.active_provider = Some(name);
        Ok(())
    }

    pub fn provider_url(&self) -> Option<&str> {
        self.selected_provider()
            .map(ProviderProfile::base_url)
            .or(self.provider_url.as_deref())
    }

    pub fn provider_model(&self) -> Option<&str> {
        self.selected_provider()
            .map(ProviderProfile::model)
            .or(self.provider_model.as_deref())
    }

    pub fn provider_api_key_env(&self) -> Option<&str> {
        self.selected_provider()
            .map(ProviderProfile::api_key_env)
            .or_else(|| {
                self.provider_url
                    .as_ref()
                    .map(|_| DEFAULT_PROVIDER_API_KEY_ENV)
            })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }

    fn selected_provider(&self) -> Option<&ProviderProfile> {
        self.active_provider
            .as_deref()
            .and_then(|name| self.provider_profiles.get(name))
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct FileConfig {
    format_version: Option<u32>,
    provider_url: Option<String>,
    provider_model: Option<String>,
    #[serde(default)]
    providers: BTreeMap<String, FileProviderProfile>,
    active_provider: Option<String>,
    data_dir: Option<String>,
    workspace_dir: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FileProviderProfile {
    base_url: String,
    model: String,
    api_key_env: String,
    #[serde(default)]
    fallback_provider: Option<String>,
    #[serde(default)]
    pricing: Option<ProviderPricing>,
}

fn validate_fallbacks(
    provider_profiles: &BTreeMap<String, ProviderProfile>,
) -> Result<(), ConfigError> {
    for (name, profile) in provider_profiles {
        let Some(fallback) = profile.fallback_provider() else {
            continue;
        };
        if fallback == name {
            return Err(ConfigError::InvalidProviderFallback);
        }
        let fallback_profile = provider_profiles
            .get(fallback)
            .ok_or(ConfigError::UnknownProvider)?;
        if fallback_profile.fallback_provider().is_some() {
            return Err(ConfigError::InvalidProviderFallback);
        }
    }
    Ok(())
}

fn resolve_path(
    value: Option<PathBuf>,
    default: PathBuf,
    config_path: &Path,
    field: &'static str,
) -> Result<PathBuf, ConfigError> {
    let path = value.unwrap_or(default);
    if path.as_os_str().is_empty() {
        return Err(ConfigError::InvalidPath(field));
    }
    if path.is_absolute() {
        return Ok(path);
    }
    let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
    Ok(parent.join(path))
}

fn validate_provider_url(value: String) -> Result<String, ConfigError> {
    let parsed = Url::parse(&value).map_err(|_| ConfigError::InvalidProviderUrl)?;
    if parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ConfigError::InvalidProviderUrl);
    }
    let is_loopback_http = parsed.scheme() == "http"
        && parsed.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
    if parsed.scheme() != "https" && !is_loopback_http {
        return Err(ConfigError::InvalidProviderUrl);
    }
    if parsed.host_str().is_none() {
        return Err(ConfigError::InvalidProviderUrl);
    }
    Ok(value)
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn is_environment_name(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit())
        })
}

pub fn default_config_path() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Pandora")
            .join("config.json")
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library")
            .join("Application Support")
            .join("Pandora")
            .join("config.json")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("pandora")
            .join("config.json")
    }
}

fn default_data_dir() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Pandora")
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library")
            .join("Application Support")
            .join("Pandora")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
            .join("pandora")
    }
}

fn set_private_permissions(file: &std::fs::File) -> Result<(), ConfigError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), ConfigError> {
    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "configuration path is a directory",
        )
        .into());
    }
    let mut file = atomic_write_file::AtomicWriteFile::open(path)?;
    set_private_permissions(file.as_file())?;
    file.write_all(data)?;
    file.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_atomic;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn atomic_write_replaces_existing_configuration() {
        let directory = fixture("replace");
        let path = directory.join("config.json");
        fs::write(&path, b"old configuration\n").unwrap();

        write_atomic(&path, b"new configuration\n").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new configuration\n");
        assert_no_temporary_files(&directory);
    }

    #[test]
    fn atomic_write_cleans_up_when_destination_cannot_be_replaced() {
        let directory = fixture("failed-replace");
        let path = directory.join("config.json");
        fs::create_dir(&path).unwrap();

        assert!(write_atomic(&path, b"configuration").is_err());

        assert!(path.is_dir());
        assert_no_temporary_files(&directory);
    }

    fn fixture(name: &str) -> PathBuf {
        crate::test_support::new_temp_dir(&format!("pandora-config-{name}")).unwrap()
    }

    fn assert_no_temporary_files(directory: &std::path::Path) {
        let entries = fs::read_dir(directory)
            .unwrap()
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_name(), "config.json");
    }
}
