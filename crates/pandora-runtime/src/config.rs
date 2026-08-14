use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use url::Url;

pub const CONFIG_FORMAT_VERSION: u32 = 1;

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    InvalidFile,
    InvalidProviderUrl,
    InvalidPath(&'static str),
    Serialization(serde_json::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => formatter.write_str("could not read or write configuration"),
            Self::InvalidFile => formatter.write_str("configuration file is invalid"),
            Self::InvalidProviderUrl => formatter.write_str("provider URL is invalid"),
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigOverrides {
    config_path: Option<PathBuf>,
    provider_url: Option<String>,
    data_dir: Option<PathBuf>,
    workspace_dir: Option<PathBuf>,
}

impl ConfigOverrides {
    pub fn with_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    pub fn with_provider_url(mut self, url: impl Into<String>) -> Self {
        self.provider_url = Some(url.into());
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
    provider_url: Option<String>,
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
            provider_url,
            data_dir,
            workspace_dir,
        })
    }

    pub fn write(&self) -> Result<(), ConfigError> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec_pretty(&FileConfig {
            format_version: Some(CONFIG_FORMAT_VERSION),
            provider_url: self.provider_url.clone(),
            data_dir: Some(self.data_dir.display().to_string()),
            workspace_dir: Some(self.workspace_dir.display().to_string()),
        })
        .map_err(ConfigError::Serialization)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.config_path)?;
        set_private_permissions(&file)?;
        file.write_all(&data)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn provider_url(&self) -> Option<&str> {
        self.provider_url.as_deref()
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct FileConfig {
    format_version: Option<u32>,
    provider_url: Option<String>,
    data_dir: Option<String>,
    workspace_dir: Option<String>,
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
