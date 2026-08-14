use crate::package_admission::PackageAdmission;
use pandora_types::{SkillId, SkillManifest};
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillState {
    Verified,
    Installed,
    Disabled,
    Enabled,
    Suspended,
    Removed,
}

impl SkillState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Installed => "installed",
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::Suspended => "suspended",
            Self::Removed => "removed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "verified" => Some(Self::Verified),
            "installed" => Some(Self::Installed),
            "disabled" => Some(Self::Disabled),
            "enabled" => Some(Self::Enabled),
            "suspended" => Some(Self::Suspended),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillError {
    Io,
    InvalidRoot,
    InvalidManifest,
    SymlinkRejected,
    NotFound,
    DuplicateId,
    PathEscape,
    DirectExecutionDisabled,
    Collision,
    CorruptState,
    RollbackFailed,
}

impl fmt::Display for SkillError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Io => "skill filesystem operation failed",
            Self::InvalidRoot => "skill root is not a directory",
            Self::InvalidManifest => "skill manifest is invalid",
            Self::SymlinkRejected => "skill symlinks are not allowed",
            Self::NotFound => "skill was not found",
            Self::DuplicateId => "skill id is duplicated",
            Self::PathEscape => "skill path escapes its admission root",
            Self::DirectExecutionDisabled => "skill scripts must run through ToolEngine",
            Self::Collision => "skill destination already exists",
            Self::CorruptState => "skill state is invalid",
            Self::RollbackFailed => "skill rollback failed",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for SkillError {}

impl From<io::Error> for SkillError {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillProvenance {
    source: PathBuf,
}

impl SkillProvenance {
    pub fn source(&self) -> &Path {
        &self.source
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillRecord {
    manifest: SkillManifest,
    root: PathBuf,
    state: SkillState,
    provenance: SkillProvenance,
}

impl SkillRecord {
    pub fn manifest(&self) -> &SkillManifest {
        &self.manifest
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state(&self) -> SkillState {
        self.state
    }

    pub fn provenance(&self) -> &SkillProvenance {
        &self.provenance
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillInspection {
    manifest: SkillManifest,
    root: PathBuf,
    state: SkillState,
    provenance: SkillProvenance,
    body: String,
    resources: Vec<String>,
    scripts: Vec<PathBuf>,
}

impl SkillInspection {
    pub fn manifest(&self) -> &SkillManifest {
        &self.manifest
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state(&self) -> SkillState {
        self.state
    }

    pub fn provenance(&self) -> &SkillProvenance {
        &self.provenance
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn resources(&self) -> &[String] {
        &self.resources
    }

    pub fn scripts(&self) -> &[PathBuf] {
        &self.scripts
    }
}

#[derive(Debug)]
pub struct RemovalReceipt {
    id: SkillId,
    original_root: PathBuf,
    backup_root: PathBuf,
    previous_state: SkillState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillEngine {
    display_root: PathBuf,
    root: PathBuf,
    state_root: PathBuf,
    removed_root: PathBuf,
}

impl SkillEngine {
    pub fn discover(root: impl AsRef<Path>) -> Result<Self, SkillError> {
        let display_root = if root.as_ref().is_absolute() {
            root.as_ref().to_path_buf()
        } else {
            std::env::current_dir()?.join(root.as_ref())
        };
        let root = fs::canonicalize(&display_root)?;
        if !fs::symlink_metadata(&root)?.is_dir() {
            return Err(SkillError::InvalidRoot);
        }
        let state_root = root.join(".pandora-state");
        let removed_root = root.join(".pandora-removed");
        fs::create_dir_all(&state_root)?;
        fs::create_dir_all(&removed_root)?;
        let engine = Self {
            display_root,
            root,
            state_root,
            removed_root,
        };
        engine.load_records()?;
        Ok(engine)
    }

    pub fn list(&self) -> Result<Vec<SkillRecord>, SkillError> {
        self.load_records()
    }

    pub fn inspect(&self, id: &str) -> Result<SkillInspection, SkillError> {
        let record = self.find_record(id)?;
        let (manifest, body) = read_skill_document(&record.root)?;
        let mut scripts = Vec::new();
        collect_scripts(&record.root, &record.root.join("scripts"), &mut scripts)?;
        Ok(SkillInspection {
            resources: manifest.resources().to_vec(),
            manifest,
            root: record.root,
            state: record.state,
            provenance: record.provenance,
            body,
            scripts,
        })
    }

    pub fn enable(&self, id: &str) -> Result<SkillRecord, SkillError> {
        self.transition(id, SkillState::Enabled)
    }

    pub fn disable(&self, id: &str) -> Result<SkillRecord, SkillError> {
        self.transition(id, SkillState::Disabled)
    }

    pub fn suspend(&self, id: &str) -> Result<SkillRecord, SkillError> {
        self.transition(id, SkillState::Suspended)
    }

    pub fn execute_script(&self, id: &str, _script: &str) -> Result<(), SkillError> {
        let _ = self.find_record(id)?;
        Err(SkillError::DirectExecutionDisabled)
    }

    pub fn remove(&self, id: &str) -> Result<RemovalReceipt, SkillError> {
        let record = self.find_record(id)?;
        let backup_root = self.unique_removed_path(record.manifest.id())?;
        if fs::symlink_metadata(&backup_root).is_ok() {
            return Err(SkillError::Collision);
        }
        fs::rename(&record.root, &backup_root)?;
        if let Err(error) = self.write_state(record.manifest.id(), SkillState::Removed) {
            let _ = fs::rename(&backup_root, &record.root);
            return Err(error);
        }
        Ok(RemovalReceipt {
            id: record.manifest.id().clone(),
            original_root: record.root,
            backup_root,
            previous_state: record.state,
        })
    }

    pub fn rollback(&self, receipt: RemovalReceipt) -> Result<(), SkillError> {
        if !is_within(&self.root, &receipt.original_root)
            || !is_within(&self.removed_root, &receipt.backup_root)
            || fs::symlink_metadata(&receipt.original_root).is_ok()
            || !fs::symlink_metadata(&receipt.backup_root)
                .map_err(|_| SkillError::RollbackFailed)?
                .is_dir()
        {
            return Err(SkillError::RollbackFailed);
        }
        fs::rename(&receipt.backup_root, &receipt.original_root)
            .map_err(|_| SkillError::RollbackFailed)?;
        if let Err(error) = self.write_state(&receipt.id, receipt.previous_state) {
            let _ = fs::rename(&receipt.original_root, &receipt.backup_root);
            return Err(error);
        }
        Ok(())
    }

    fn load_records(&self) -> Result<Vec<SkillRecord>, SkillError> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|value| value.to_str());
            if matches!(name, Some(".pandora-state") | Some(".pandora-removed")) {
                continue;
            }
            paths.push(path);
        }
        paths.sort();

        let mut records = Vec::new();
        let mut ids = HashSet::new();
        for path in paths {
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(SkillError::SymlinkRejected);
            }
            if !metadata.is_dir() {
                continue;
            }
            let manifest_path = path.join("SKILL.md");
            let manifest_metadata = match fs::symlink_metadata(&manifest_path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if manifest_metadata.file_type().is_symlink() {
                return Err(SkillError::SymlinkRejected);
            }
            let record = self.load_record(&path)?;
            if !ids.insert(record.manifest.id().clone()) {
                return Err(SkillError::DuplicateId);
            }
            records.push(record);
        }
        Ok(records)
    }

    fn load_record(&self, path: &Path) -> Result<SkillRecord, SkillError> {
        let root = fs::canonicalize(path)?;
        if !is_within(&self.root, &root) {
            return Err(SkillError::PathEscape);
        }
        let (manifest, _) = read_skill_document(&root)?;
        PackageAdmission::validate_skill(&manifest).map_err(|_| SkillError::InvalidManifest)?;
        let directory_name = root
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or(SkillError::InvalidManifest)?;
        if manifest.id().as_str() != directory_name {
            return Err(SkillError::InvalidManifest);
        }
        let mut scripts = Vec::new();
        collect_scripts(&root, &root.join("scripts"), &mut scripts)?;
        let state = self.read_state(manifest.id())?;
        Ok(SkillRecord {
            manifest,
            root: root.clone(),
            state,
            provenance: SkillProvenance {
                source: self.display_root.join(
                    root.strip_prefix(&self.root)
                        .map_err(|_| SkillError::PathEscape)?,
                ),
            },
        })
    }

    fn find_record(&self, id: &str) -> Result<SkillRecord, SkillError> {
        self.load_records()?
            .into_iter()
            .find(|record| record.manifest.id().as_str() == id)
            .ok_or(SkillError::NotFound)
    }

    fn transition(&self, id: &str, state: SkillState) -> Result<SkillRecord, SkillError> {
        let record = self.find_record(id)?;
        self.write_state(record.manifest.id(), state)?;
        self.find_record(id)
    }

    fn state_path(&self, id: &SkillId) -> PathBuf {
        self.state_root.join(format!("{}.state", id.as_str()))
    }

    fn read_state(&self, id: &SkillId) -> Result<SkillState, SkillError> {
        let path = self.state_path(id);
        match fs::read_to_string(path) {
            Ok(value) => SkillState::parse(value.trim()).ok_or(SkillError::CorruptState),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(SkillState::Disabled),
            Err(error) => Err(error.into()),
        }
    }

    fn write_state(&self, id: &SkillId, state: SkillState) -> Result<(), SkillError> {
        let path = self.state_path(id);
        let temporary = path.with_extension(format!("state-{}", unique_suffix()));
        fs::write(&temporary, state.as_str())?;
        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        Ok(())
    }

    fn unique_removed_path(&self, id: &SkillId) -> Result<PathBuf, SkillError> {
        for _ in 0..100 {
            let path = self
                .removed_root
                .join(format!("{}-{}", id.as_str(), unique_suffix()));
            if fs::symlink_metadata(&path).is_err() {
                return Ok(path);
            }
        }
        Err(SkillError::Collision)
    }
}

fn read_skill_document(root: &Path) -> Result<(SkillManifest, String), SkillError> {
    let manifest_path = root.join("SKILL.md");
    let metadata = fs::symlink_metadata(&manifest_path)?;
    if metadata.file_type().is_symlink() {
        return Err(SkillError::SymlinkRejected);
    }
    let content = fs::read_to_string(manifest_path)?.replace("\r\n", "\n");
    let front_matter = content
        .strip_prefix("---\n")
        .ok_or(SkillError::InvalidManifest)?;
    let (front_matter, body) = front_matter
        .split_once("\n---\n")
        .ok_or(SkillError::InvalidManifest)?;

    let mut id = None;
    let mut version = None;
    let mut name = None;
    let mut description = None;
    let mut publisher = None;
    let mut resources = Vec::new();
    for line in front_matter.lines() {
        let Some((key, value)) = line.split_once(':') else {
            return Err(SkillError::InvalidManifest);
        };
        let value = unquote(value.trim());
        match key.trim() {
            "id" => id = Some(value),
            "version" => version = Some(value),
            "name" => name = Some(value),
            "description" => description = Some(value),
            "publisher" => {
                if !value.is_empty() {
                    publisher = Some(value);
                }
            }
            "resources" => {
                resources = value
                    .split(',')
                    .map(str::trim)
                    .filter(|resource| !resource.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            _ => {}
        }
    }

    let manifest = SkillManifest::new(
        id.ok_or(SkillError::InvalidManifest)?,
        version.ok_or(SkillError::InvalidManifest)?,
        name.ok_or(SkillError::InvalidManifest)?,
        description.ok_or(SkillError::InvalidManifest)?,
        publisher,
        resources,
    )
    .map_err(|_| SkillError::InvalidManifest)?;
    Ok((manifest, body.to_owned()))
}

fn collect_scripts(
    package_root: &Path,
    scripts_root: &Path,
    scripts: &mut Vec<PathBuf>,
) -> Result<(), SkillError> {
    let metadata = match fs::symlink_metadata(scripts_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(SkillError::SymlinkRejected);
    }
    if !metadata.is_dir() {
        return Err(SkillError::InvalidManifest);
    }
    for entry in fs::read_dir(scripts_root)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(SkillError::SymlinkRejected);
        }
        if metadata.is_dir() {
            let canonical = fs::canonicalize(&path)?;
            if !is_within(package_root, &canonical) {
                return Err(SkillError::PathEscape);
            }
            collect_scripts(package_root, &canonical, scripts)?;
        } else if metadata.is_file() {
            let canonical = fs::canonicalize(&path)?;
            if !is_within(package_root, &canonical) {
                return Err(SkillError::PathEscape);
            }
            scripts.push(canonical);
        }
    }
    scripts.sort();
    Ok(())
}

fn unquote(value: &str) -> String {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_owned()
    } else {
        value.to_owned()
    }
}

fn is_within(root: &Path, candidate: &Path) -> bool {
    candidate.starts_with(root)
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "pandora-skills-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("alpha/scripts")).unwrap();
            fs::write(root.join("alpha/SKILL.md"), skill_text()).unwrap();
            fs::write(root.join("alpha/scripts/check.py"), "print('ok')").unwrap();
            Self { root }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn skill_text() -> &'static str {
        "---\nid: alpha\nversion: 0.1.0\nname: Alpha Skill\ndescription: A bounded test skill\npublisher: pandora\nresources: workspace.read, workspace.search\n---\n# Alpha\n\nUse the read tool.\n"
    }

    #[test]
    fn discovery_loads_metadata_without_enabling_the_skill() {
        let fixture = Fixture::new();
        let engine = SkillEngine::discover(&fixture.root).unwrap();

        let skills = engine.list().unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].manifest().id().as_str(), "alpha");
        assert_eq!(skills[0].state(), SkillState::Disabled);
        let inspection = engine.inspect("alpha").unwrap();
        assert!(inspection.body().contains("Use the read tool"));
        assert_eq!(inspection.manifest().publisher(), Some("pandora"));
        assert_eq!(
            inspection.resources(),
            &["workspace.read", "workspace.search"]
        );
    }

    #[test]
    fn malformed_front_matter_fails_closed() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("alpha/SKILL.md"), "# no front matter\n").unwrap();

        assert_eq!(
            SkillEngine::discover(&fixture.root),
            Err(SkillError::InvalidManifest)
        );
    }

    #[test]
    fn state_transitions_are_explicit_and_scripts_cannot_execute_directly() {
        let fixture = Fixture::new();
        let engine = SkillEngine::discover(&fixture.root).unwrap();

        assert_eq!(engine.enable("alpha").unwrap().state(), SkillState::Enabled);
        assert_eq!(
            engine.suspend("alpha").unwrap().state(),
            SkillState::Suspended
        );
        assert_eq!(
            engine.disable("alpha").unwrap().state(),
            SkillState::Disabled
        );
        assert_eq!(
            engine.execute_script("alpha", "scripts/check.py"),
            Err(SkillError::DirectExecutionDisabled)
        );
    }

    #[test]
    fn removal_is_reversible_and_preserves_provenance() {
        let fixture = Fixture::new();
        let engine = SkillEngine::discover(&fixture.root).unwrap();
        let receipt = engine.remove("alpha").unwrap();

        assert!(engine.list().unwrap().is_empty());
        engine.rollback(receipt).unwrap();
        let inspection = engine.inspect("alpha").unwrap();
        assert_eq!(inspection.manifest().publisher(), Some("pandora"));
        assert_eq!(inspection.provenance().source(), fixture.root.join("alpha"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_skill_files_are_rejected() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let target = fixture.root.join("outside.md");
        fs::write(&target, skill_text()).unwrap();
        fs::remove_file(fixture.root.join("alpha/SKILL.md")).unwrap();
        symlink(&target, fixture.root.join("alpha/SKILL.md")).unwrap();

        assert_eq!(
            SkillEngine::discover(&fixture.root),
            Err(SkillError::SymlinkRejected)
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_script_directories_cannot_escape_the_skill_root() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let outside = fixture.root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::remove_dir_all(fixture.root.join("alpha/scripts")).unwrap();
        symlink(&outside, fixture.root.join("alpha/scripts")).unwrap();

        assert_eq!(
            SkillEngine::discover(&fixture.root),
            Err(SkillError::SymlinkRejected)
        );
    }
}
