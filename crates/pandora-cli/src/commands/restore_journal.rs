use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Component, Path};

const FORMAT_VERSION: u32 = 1;
const MAX_ENTRIES: usize = 4096;
const MAX_FIELD_BYTES: usize = 4096;
const MAX_JOURNAL_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RestoreJournalState {
    Prepared,
    Applying,
    Completed,
    RollingBack,
    RolledBack,
    RollbackFailed,
}

impl RestoreJournalState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Applying => "applying",
            Self::Completed => "completed",
            Self::RollingBack => "rolling_back",
            Self::RolledBack => "rolled_back",
            Self::RollbackFailed => "rollback_failed",
        }
    }
}

#[derive(Debug)]
pub(crate) enum RestoreJournalError {
    Io(String),
    Json(String),
    Invalid(String),
    EntryNotFound,
}

impl fmt::Display for RestoreJournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "restore journal I/O failed: {message}"),
            Self::Json(message) => write!(formatter, "restore journal JSON is invalid: {message}"),
            Self::Invalid(message) => write!(formatter, "restore journal is invalid: {message}"),
            Self::EntryNotFound => formatter.write_str("restore journal entry was not found"),
        }
    }
}

impl std::error::Error for RestoreJournalError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreSidecar {
    suffix: String,
    original: Option<String>,
}

impl RestoreSidecar {
    pub(crate) fn new(
        suffix: impl Into<String>,
        original: Option<String>,
    ) -> Result<Self, RestoreJournalError> {
        let sidecar = Self {
            suffix: suffix.into(),
            original,
        };
        sidecar.validate()?;
        Ok(sidecar)
    }

    pub(crate) fn suffix(&self) -> &str {
        &self.suffix
    }

    pub(crate) fn original(&self) -> Option<&str> {
        self.original.as_deref()
    }

    fn validate(&self) -> Result<(), RestoreJournalError> {
        if !matches!(self.suffix.as_str(), "wal" | "shm" | "journal") {
            return Err(RestoreJournalError::Invalid(
                "unsupported SQLite sidecar suffix".to_owned(),
            ));
        }
        if let Some(original) = &self.original {
            validate_relative_path("sidecar original", original)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreJournalEntry {
    target: String,
    staged: String,
    original: Option<String>,
    #[serde(default)]
    sidecars: Vec<RestoreSidecar>,
    applied: bool,
}

impl RestoreJournalEntry {
    pub(crate) fn new(
        target: impl Into<String>,
        staged: impl Into<String>,
        original: Option<String>,
    ) -> Result<Self, RestoreJournalError> {
        let entry = Self {
            target: target.into(),
            staged: staged.into(),
            original,
            sidecars: Vec::new(),
            applied: false,
        };
        entry.validate()?;
        Ok(entry)
    }

    pub(crate) fn target(&self) -> &str {
        &self.target
    }

    pub(crate) fn staged(&self) -> &str {
        &self.staged
    }

    pub(crate) fn original(&self) -> Option<&str> {
        self.original.as_deref()
    }

    pub(crate) const fn applied(&self) -> bool {
        self.applied
    }

    pub(crate) fn sidecars(&self) -> &[RestoreSidecar] {
        &self.sidecars
    }

    pub(crate) fn with_sidecars(
        mut self,
        sidecars: Vec<RestoreSidecar>,
    ) -> Result<Self, RestoreJournalError> {
        self.sidecars = sidecars;
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), RestoreJournalError> {
        validate_relative_path("target", &self.target)?;
        validate_relative_path("staged", &self.staged)?;
        if let Some(original) = &self.original {
            validate_relative_path("original", original)?;
        }
        let mut suffixes = std::collections::BTreeSet::new();
        for sidecar in &self.sidecars {
            sidecar.validate()?;
            if !suffixes.insert(sidecar.suffix.clone()) {
                return Err(RestoreJournalError::Invalid(
                    "journal contains duplicate sidecar suffixes".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestoreJournal {
    format_version: u32,
    transaction_id: String,
    state: RestoreJournalState,
    entries: Vec<RestoreJournalEntry>,
}

impl RestoreJournal {
    pub(crate) fn create(
        path: impl AsRef<Path>,
        transaction_id: impl Into<String>,
        entries: Vec<RestoreJournalEntry>,
    ) -> Result<Self, RestoreJournalError> {
        let journal = Self {
            format_version: FORMAT_VERSION,
            transaction_id: transaction_id.into(),
            state: RestoreJournalState::Prepared,
            entries,
        };
        journal.validate()?;
        journal.write(path.as_ref())?;
        Ok(journal)
    }

    pub(crate) fn load(path: impl AsRef<Path>) -> Result<Self, RestoreJournalError> {
        let path = path.as_ref();
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                RestoreJournalError::Invalid("journal does not exist".to_owned())
            } else {
                RestoreJournalError::Io(error.to_string())
            }
        })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_JOURNAL_BYTES
        {
            return Err(RestoreJournalError::Invalid(
                "journal path is unsafe or too large".to_owned(),
            ));
        }
        let bytes = fs::read(path).map_err(|error| RestoreJournalError::Io(error.to_string()))?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| RestoreJournalError::Json(error.to_string()))?;
        let object = value.as_object().ok_or_else(|| {
            RestoreJournalError::Invalid("journal root must be an object".to_owned())
        })?;
        if object.len() != 4
            || !object.contains_key("format_version")
            || !object.contains_key("transaction_id")
            || !object.contains_key("state")
            || !object.contains_key("entries")
        {
            return Err(RestoreJournalError::Invalid(
                "journal root has an unsupported shape".to_owned(),
            ));
        }
        let journal: Self = serde_json::from_value(value).map_err(|error| {
            RestoreJournalError::Invalid(format!("journal schema is invalid: {error}"))
        })?;
        journal.validate()?;
        Ok(journal)
    }

    pub(crate) const fn state(&self) -> RestoreJournalState {
        self.state
    }

    pub(crate) fn transaction_id(&self) -> &str {
        &self.transaction_id
    }

    pub(crate) fn entries(&self) -> &[RestoreJournalEntry] {
        &self.entries
    }

    pub(crate) const fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            RestoreJournalState::Completed
                | RestoreJournalState::RolledBack
                | RestoreJournalState::RollbackFailed
        )
    }

    pub(crate) fn set_state(
        &mut self,
        path: impl AsRef<Path>,
        state: RestoreJournalState,
    ) -> Result<(), RestoreJournalError> {
        self.state = state;
        self.validate()?;
        self.write(path.as_ref())
    }

    pub(crate) fn mark_applied(
        &mut self,
        path: impl AsRef<Path>,
        target: &str,
    ) -> Result<(), RestoreJournalError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.target == target)
            .ok_or(RestoreJournalError::EntryNotFound)?;
        entry.applied = true;
        self.write(path.as_ref())
    }

    fn validate(&self) -> Result<(), RestoreJournalError> {
        if self.format_version != FORMAT_VERSION {
            return Err(RestoreJournalError::Invalid(
                "unsupported journal format version".to_owned(),
            ));
        }
        validate_field("transaction ID", &self.transaction_id, 128)?;
        if self.entries.is_empty() || self.entries.len() > MAX_ENTRIES {
            return Err(RestoreJournalError::Invalid(
                "journal entry count is outside the allowed bounds".to_owned(),
            ));
        }
        let mut targets = std::collections::BTreeSet::new();
        let mut staged = std::collections::BTreeSet::new();
        for entry in &self.entries {
            entry.validate()?;
            if !targets.insert(entry.target.clone()) {
                return Err(RestoreJournalError::Invalid(
                    "journal contains duplicate targets".to_owned(),
                ));
            }
            if !staged.insert(entry.staged.clone()) {
                return Err(RestoreJournalError::Invalid(
                    "journal contains duplicate staged paths".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn write(&self, path: &Path) -> Result<(), RestoreJournalError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| RestoreJournalError::Io(error.to_string()))?;
        }
        if let Ok(metadata) = fs::symlink_metadata(path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(RestoreJournalError::Invalid(
                "journal path is unsafe".to_owned(),
            ));
        }
        let encoded = serde_json::to_vec_pretty(self)
            .map_err(|error| RestoreJournalError::Json(error.to_string()))?;
        if encoded.len() as u64 > MAX_JOURNAL_BYTES {
            return Err(RestoreJournalError::Invalid(
                "journal exceeds the size limit".to_owned(),
            ));
        }
        let mut file = AtomicWriteFile::open(path)
            .map_err(|error| RestoreJournalError::Io(error.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| RestoreJournalError::Io(error.to_string()))?;
        }
        file.write_all(&encoded)
            .map_err(|error| RestoreJournalError::Io(error.to_string()))?;
        file.commit()
            .map_err(|error| RestoreJournalError::Io(error.to_string()))
    }
}

fn validate_field(name: &str, value: &str, max_bytes: usize) -> Result<(), RestoreJournalError> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(RestoreJournalError::Invalid(format!(
            "{name} is outside the allowed bounds"
        )));
    }
    Ok(())
}

fn validate_relative_path(name: &str, value: &str) -> Result<(), RestoreJournalError> {
    validate_field(name, value, MAX_FIELD_BYTES)?;
    if value.contains('\\') {
        return Err(RestoreJournalError::Invalid(format!(
            "{name} must use forward-slash relative paths"
        )));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(RestoreJournalError::Invalid(format!(
            "{name} must be a normalized relative path"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        RestoreJournal, RestoreJournalEntry, RestoreJournalError, RestoreJournalState,
        RestoreSidecar,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_PATH: AtomicU64 = AtomicU64::new(1);

    fn path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "pandora-restore-journal-{}-{}.json",
            std::process::id(),
            NEXT_PATH.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn entry(target: &str) -> RestoreJournalEntry {
        RestoreJournalEntry::new(
            target,
            format!("staged/{target}"),
            Some(format!("original/{target}")),
        )
        .unwrap()
    }

    #[test]
    fn journal_round_trips_and_records_durable_progress() {
        let path = path();
        let sqlite_entry = entry("data/sessions.sqlite3")
            .with_sidecars(vec![
                RestoreSidecar::new("wal", Some("original/data/sessions.sqlite3-wal".to_owned()))
                    .unwrap(),
            ])
            .unwrap();
        let mut journal = RestoreJournal::create(
            &path,
            "restore-20260924-abc",
            vec![sqlite_entry, entry("config/config.json")],
        )
        .unwrap();
        assert_eq!(journal.state(), RestoreJournalState::Prepared);
        journal
            .mark_applied(&path, "data/sessions.sqlite3")
            .unwrap();
        journal
            .set_state(&path, RestoreJournalState::Applying)
            .unwrap();

        let loaded = RestoreJournal::load(&path).unwrap();
        assert_eq!(loaded.state(), RestoreJournalState::Applying);
        assert!(loaded.entries()[0].applied());
        assert_eq!(loaded.entries()[0].sidecars()[0].suffix(), "wal");
        assert!(!loaded.entries()[1].applied());
        assert!(!loaded.is_terminal());

        journal
            .mark_applied(&path, "data/sessions.sqlite3")
            .unwrap();
        journal
            .set_state(&path, RestoreJournalState::Completed)
            .unwrap();
        assert!(RestoreJournal::load(&path).unwrap().is_terminal());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn journal_rejects_traversal_duplicates_and_unknown_state() {
        let path = path();
        assert!(matches!(
            RestoreJournalEntry::new("../escape", "staged/escape", None),
            Err(RestoreJournalError::Invalid(_))
        ));
        assert!(matches!(
            RestoreJournal::create(&path, "restore-1", vec![entry("data/a"), entry("data/a")],),
            Err(RestoreJournalError::Invalid(_))
        ));

        let journal = RestoreJournal::create(&path, "restore-1", vec![entry("data/a")]).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["state"] = serde_json::Value::String("unknown".to_owned());
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            RestoreJournal::load(&path),
            Err(RestoreJournalError::Invalid(_))
        ));
        drop(journal);
        fs::remove_file(path).unwrap();
    }
}
