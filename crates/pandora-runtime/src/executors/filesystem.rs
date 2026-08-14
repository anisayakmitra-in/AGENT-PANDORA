use crate::ConsumedPermit;
use pandora_types::{
    Capability, EffectOutcome, EffectReceipt, EffectTarget, Operation, ReceiptId, ResourceScope,
    Timestamp,
};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 2_048;
const MAX_SEARCH_ENTRIES: usize = 10_000;
static NEXT_RECEIPT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilesystemError {
    InvalidWorkspace,
    EmptyPath,
    AbsolutePath,
    ParentTraversal,
    PathOutsideWorkspace,
    SymlinkNotAllowed,
    NotDirectory,
    NotFile,
    PermissionDenied,
    ReadLimitExceeded,
    TooManyEntries,
    TargetExists,
    Io { operation: &'static str },
}

impl FilesystemError {
    fn code(&self) -> &'static str {
        match self {
            Self::InvalidWorkspace => "invalid_workspace",
            Self::EmptyPath => "empty_path",
            Self::AbsolutePath => "absolute_path",
            Self::ParentTraversal => "parent_traversal",
            Self::PathOutsideWorkspace => "path_outside_workspace",
            Self::SymlinkNotAllowed => "symlink_not_allowed",
            Self::NotDirectory => "not_directory",
            Self::NotFile => "not_file",
            Self::PermissionDenied => "permission_denied",
            Self::ReadLimitExceeded => "read_limit_exceeded",
            Self::TooManyEntries => "too_many_entries",
            Self::TargetExists => "target_exists",
            Self::Io { .. } => "filesystem_io",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRoot {
    canonical: PathBuf,
}

impl WorkspaceRoot {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, FilesystemError> {
        let path = path.as_ref();
        let metadata = fs::symlink_metadata(path).map_err(|_| FilesystemError::InvalidWorkspace)?;
        if metadata.file_type().is_symlink() {
            return Err(FilesystemError::SymlinkNotAllowed);
        }
        let canonical = fs::canonicalize(path).map_err(|_| FilesystemError::InvalidWorkspace)?;
        if !canonical.is_dir() {
            return Err(FilesystemError::InvalidWorkspace);
        }
        Ok(Self { canonical })
    }

    pub fn root(&self) -> &Path {
        &self.canonical
    }

    pub fn path(&self, value: &str) -> Result<WorkspacePath, FilesystemError> {
        if value.trim().is_empty() {
            return Err(FilesystemError::EmptyPath);
        }
        let relative = Path::new(value);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
        {
            return Err(FilesystemError::AbsolutePath);
        }
        if relative
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(FilesystemError::ParentTraversal);
        }

        let absolute = self.canonical.join(relative);
        validate_components(&self.canonical, relative)?;
        ensure_contained(&self.canonical, &absolute)?;
        Ok(WorkspacePath {
            root: self.canonical.clone(),
            relative: relative.to_path_buf(),
            source: value.to_owned(),
            absolute,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspacePath {
    root: PathBuf,
    relative: PathBuf,
    source: String,
    absolute: PathBuf,
}

impl WorkspacePath {
    pub fn relative(&self) -> &Path {
        &self.relative
    }

    pub fn absolute(&self) -> &Path {
        &self.absolute
    }
}

pub struct FilesystemExecutor;

pub struct FilesystemResult<T> {
    result: Result<T, FilesystemError>,
    receipt: EffectReceipt,
}

impl<T> FilesystemResult<T> {
    pub fn result(&self) -> Result<&T, &FilesystemError> {
        self.result.as_ref()
    }

    pub fn into_result(self) -> Result<T, FilesystemError> {
        self.result
    }

    pub fn receipt(&self) -> &EffectReceipt {
        &self.receipt
    }
}

impl FilesystemExecutor {
    pub const fn new() -> Self {
        Self
    }

    pub fn read(
        &self,
        permit: &ConsumedPermit,
        target: &WorkspacePath,
        now: Timestamp,
    ) -> FilesystemResult<Vec<u8>> {
        self.execute(
            permit,
            target,
            Capability::FilesystemRead,
            Operation::Read,
            now,
            || {
                let metadata = checked_metadata(target, "read metadata")?;
                if !metadata.is_file() {
                    return Err(FilesystemError::NotFile);
                }
                let mut file = File::open(target.absolute()).map_err(|_| FilesystemError::Io {
                    operation: "open file",
                })?;
                read_bounded(&mut file)
            },
        )
    }

    pub fn list(
        &self,
        permit: &ConsumedPermit,
        target: &WorkspacePath,
        now: Timestamp,
    ) -> FilesystemResult<Vec<String>> {
        self.execute(
            permit,
            target,
            Capability::FilesystemRead,
            Operation::Read,
            now,
            || {
                let metadata = checked_metadata(target, "list metadata")?;
                if !metadata.is_dir() {
                    return Err(FilesystemError::NotDirectory);
                }
                let mut entries = Vec::new();
                for entry in fs::read_dir(target.absolute()).map_err(|_| FilesystemError::Io {
                    operation: "read directory",
                })? {
                    if entries.len() == MAX_DIRECTORY_ENTRIES {
                        return Err(FilesystemError::TooManyEntries);
                    }
                    let entry = entry.map_err(|_| FilesystemError::Io {
                        operation: "read directory entry",
                    })?;
                    if entry
                        .file_type()
                        .map_err(|_| FilesystemError::Io {
                            operation: "read directory entry type",
                        })?
                        .is_symlink()
                    {
                        return Err(FilesystemError::SymlinkNotAllowed);
                    }
                    entries.push(entry.file_name().to_string_lossy().into_owned());
                }
                entries.sort();
                Ok(entries)
            },
        )
    }

    pub fn search(
        &self,
        permit: &ConsumedPermit,
        target: &WorkspacePath,
        query: &str,
        now: Timestamp,
    ) -> FilesystemResult<Vec<String>> {
        self.execute(
            permit,
            target,
            Capability::FilesystemRead,
            Operation::Read,
            now,
            || {
                if query.is_empty() {
                    return Err(FilesystemError::EmptyPath);
                }
                let metadata = checked_metadata(target, "search metadata")?;
                if !metadata.is_dir() {
                    return Err(FilesystemError::NotDirectory);
                }
                let mut pending = vec![target.absolute().to_path_buf()];
                let mut matches = Vec::new();
                let mut visited = 0;
                while let Some(directory) = pending.pop() {
                    for entry in fs::read_dir(&directory).map_err(|_| FilesystemError::Io {
                        operation: "search directory",
                    })? {
                        visited += 1;
                        if visited > MAX_SEARCH_ENTRIES {
                            return Err(FilesystemError::TooManyEntries);
                        }
                        let entry = entry.map_err(|_| FilesystemError::Io {
                            operation: "search entry",
                        })?;
                        let file_type = entry.file_type().map_err(|_| FilesystemError::Io {
                            operation: "search entry type",
                        })?;
                        if file_type.is_symlink() {
                            return Err(FilesystemError::SymlinkNotAllowed);
                        }
                        if file_type.is_dir() {
                            pending.push(entry.path());
                            continue;
                        }
                        if !file_type.is_file() {
                            continue;
                        }
                        let mut file =
                            File::open(entry.path()).map_err(|_| FilesystemError::Io {
                                operation: "search file",
                            })?;
                        let contents = read_bounded(&mut file)?;
                        if String::from_utf8_lossy(&contents).contains(query) {
                            let entry_path = entry.path();
                            let relative = entry_path
                                .strip_prefix(target.root.as_path())
                                .map_err(|_| FilesystemError::PathOutsideWorkspace)?;
                            matches.push(relative.to_string_lossy().into_owned());
                        }
                    }
                }
                matches.sort();
                Ok(matches)
            },
        )
    }

    pub fn write_patch(
        &self,
        permit: &ConsumedPermit,
        target: &WorkspacePath,
        content: &[u8],
        now: Timestamp,
    ) -> FilesystemResult<()> {
        self.execute(
            permit,
            target,
            Capability::FilesystemWrite,
            Operation::Write,
            now,
            || {
                if content.len() as u64 > MAX_FILE_BYTES {
                    return Err(FilesystemError::ReadLimitExceeded);
                }
                if let Ok(metadata) = fs::symlink_metadata(target.absolute()) {
                    if metadata.file_type().is_symlink() {
                        return Err(FilesystemError::SymlinkNotAllowed);
                    }
                    if !metadata.is_file() {
                        return Err(FilesystemError::NotFile);
                    }
                }
                let parent = target
                    .absolute()
                    .parent()
                    .ok_or(FilesystemError::PathOutsideWorkspace)?;
                let temp = parent.join(format!(
                    ".pandora-write-{}",
                    NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
                ));
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temp)
                    .map_err(|_| FilesystemError::Io {
                        operation: "create temporary file",
                    })?;
                let write_result = file
                    .write_all(content)
                    .and_then(|_| file.sync_all())
                    .map_err(|_| FilesystemError::Io {
                        operation: "write temporary file",
                    });
                if write_result.is_err() {
                    let _ = fs::remove_file(&temp);
                    return write_result;
                }
                fs::rename(&temp, target.absolute()).map_err(|_| {
                    let _ = fs::remove_file(&temp);
                    if target.absolute().exists() {
                        FilesystemError::TargetExists
                    } else {
                        FilesystemError::Io {
                            operation: "commit temporary file",
                        }
                    }
                })
            },
        )
    }

    fn execute<T, F>(
        &self,
        permit: &ConsumedPermit,
        target: &WorkspacePath,
        capability: Capability,
        operation: Operation,
        now: Timestamp,
        action: F,
    ) -> FilesystemResult<T>
    where
        F: FnOnce() -> Result<T, FilesystemError>,
    {
        let result = if request_matches(permit, target, capability, operation) {
            action()
        } else {
            Err(FilesystemError::PermissionDenied)
        };
        let outcome = match &result {
            Ok(_) => EffectOutcome::Succeeded,
            Err(error) => EffectOutcome::Failed {
                code: error.code().to_owned(),
            },
        };
        FilesystemResult {
            result,
            receipt: receipt_for(permit, now, outcome),
        }
    }
}

impl Default for FilesystemExecutor {
    fn default() -> Self {
        Self::new()
    }
}

fn request_matches(
    permit: &ConsumedPermit,
    target: &WorkspacePath,
    capability: Capability,
    operation: Operation,
) -> bool {
    let request = permit.request();
    request.capability() == capability
        && request.operation() == operation
        && matches!(
            request.resource_scope(),
            ResourceScope::Workspace { .. } | ResourceScope::Path { .. }
        )
        && matches!(request.target(), EffectTarget::Path { path } if path == &target.source)
}

fn receipt_for(permit: &ConsumedPermit, now: Timestamp, outcome: EffectOutcome) -> EffectReceipt {
    let receipt_id = ReceiptId::new(format!(
        "receipt-{}",
        NEXT_RECEIPT_ID.fetch_add(1, Ordering::Relaxed)
    ))
    .expect("generated receipt ID is valid");
    EffectReceipt::new(
        receipt_id,
        permit.permit().permit_id().clone(),
        permit.permit().request_digest().clone(),
        now,
        outcome,
    )
}

fn checked_metadata(
    target: &WorkspacePath,
    operation: &'static str,
) -> Result<fs::Metadata, FilesystemError> {
    let metadata =
        fs::symlink_metadata(target.absolute()).map_err(|_| FilesystemError::Io { operation })?;
    if metadata.file_type().is_symlink() {
        return Err(FilesystemError::SymlinkNotAllowed);
    }
    Ok(metadata)
}

fn read_bounded(reader: &mut impl Read) -> Result<Vec<u8>, FilesystemError> {
    let mut contents = Vec::new();
    reader
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut contents)
        .map_err(|_| FilesystemError::Io {
            operation: "read file",
        })?;
    if contents.len() as u64 > MAX_FILE_BYTES {
        return Err(FilesystemError::ReadLimitExceeded);
    }
    Ok(contents)
}

fn validate_components(root: &Path, relative: &Path) -> Result<(), FilesystemError> {
    let components: Vec<_> = relative.components().collect();
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            continue;
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(FilesystemError::SymlinkNotAllowed);
                }
                if index + 1 < components.len() && !metadata.is_dir() {
                    return Err(FilesystemError::PathOutsideWorkspace);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if index + 1 < components.len() {
                    return Err(FilesystemError::PathOutsideWorkspace);
                }
            }
            Err(_) => {
                return Err(FilesystemError::Io {
                    operation: "inspect path",
                });
            }
        }
    }
    Ok(())
}

fn ensure_contained(root: &Path, candidate: &Path) -> Result<(), FilesystemError> {
    let check = if candidate.exists() {
        fs::canonicalize(candidate).map_err(|_| FilesystemError::PathOutsideWorkspace)
    } else {
        candidate
            .parent()
            .ok_or(FilesystemError::PathOutsideWorkspace)
            .and_then(|parent| {
                fs::canonicalize(parent).map_err(|_| FilesystemError::PathOutsideWorkspace)
            })
    }?;
    if check.starts_with(root) {
        Ok(())
    } else {
        Err(FilesystemError::PathOutsideWorkspace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Parliament, ReferenceMonitor};
    use pandora_types::{
        EffectTarget, ExecutionId, GeneId, OperationRequest, PolicyContext, PrincipalId, SessionId,
    };
    use std::sync::atomic::AtomicU64;

    #[test]
    fn reads_inside_workspace_and_returns_receipt() {
        let fixture = Fixture::new();
        let target = fixture.root.path("README.md").unwrap();
        let request = fixture.request(Capability::FilesystemRead, Operation::Read, "README.md");
        let permit = fixture.permit(request);

        let response = fixture.executor.read(&permit, &target, fixture.now());

        assert_eq!(response.result().unwrap().as_slice(), b"fixture\n");
        assert!(matches!(
            response.receipt().outcome(),
            EffectOutcome::Succeeded
        ));
    }

    #[test]
    fn rejects_parent_escape() {
        let fixture = Fixture::new();

        assert_eq!(
            fixture.root.path("../outside.txt"),
            Err(FilesystemError::ParentTraversal)
        );
    }

    #[test]
    fn rejects_absolute_path() {
        let fixture = Fixture::new();
        let absolute = fixture.root.root().join("outside.txt");

        assert_eq!(
            fixture.root.path(absolute.to_string_lossy().as_ref()),
            Err(FilesystemError::AbsolutePath)
        );
    }

    #[test]
    fn rejects_symlink_to_outside() {
        let fixture = Fixture::new();
        let outside = fixture.temp.path().join("outside.txt");
        fs::write(&outside, b"outside").unwrap();
        let link = fixture.root.root().join("outside-link");
        if create_file_symlink(&outside, &link).is_err() {
            return;
        }

        assert_eq!(
            fixture.root.path("outside-link"),
            Err(FilesystemError::SymlinkNotAllowed)
        );
    }

    #[test]
    fn write_requires_a_write_permit() {
        let fixture = Fixture::new();
        let target = fixture.root.path("README.md").unwrap();
        let request = fixture.request(Capability::FilesystemRead, Operation::Read, "README.md");
        let permit = fixture.permit(request);

        let response = fixture
            .executor
            .write_patch(&permit, &target, b"changed\n", fixture.now());

        assert_eq!(
            response.result().unwrap_err(),
            &FilesystemError::PermissionDenied
        );
        assert!(matches!(
            response.receipt().outcome(),
            EffectOutcome::Failed { .. }
        ));
    }

    struct Fixture {
        temp: TempDir,
        root: WorkspaceRoot,
        executor: FilesystemExecutor,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new();
            let workspace = temp.path().join("workspace");
            fs::create_dir(&workspace).unwrap();
            fs::write(workspace.join("README.md"), b"fixture\n").unwrap();
            let root = WorkspaceRoot::new(&workspace).unwrap();
            Self {
                temp,
                root,
                executor: FilesystemExecutor::new(),
            }
        }

        fn now(&self) -> Timestamp {
            Timestamp::from_unix_seconds(10)
        }

        fn request(
            &self,
            capability: Capability,
            operation: Operation,
            path: &str,
        ) -> OperationRequest {
            OperationRequest::new(
                ExecutionId::new("execution-1").unwrap(),
                SessionId::new("session-1").unwrap(),
                PrincipalId::new("principal-1").unwrap(),
                GeneId::new("filesystem").unwrap(),
                None,
                capability,
                operation,
                EffectTarget::path(path),
                ResourceScope::workspace("workspace-1"),
            )
            .unwrap()
        }

        fn permit(&self, request: OperationRequest) -> ConsumedPermit {
            let monitor = ReferenceMonitor::new(1, 60);
            let context = PolicyContext::new(1, [request.capability()], []);
            let decision = Parliament::new(1).decide(&request, &context);
            let permit = monitor
                .authorize(request.clone(), decision, self.now())
                .unwrap();
            monitor
                .store()
                .consume(permit, &request, self.now())
                .unwrap()
        }
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(1);
            let path = std::env::temp_dir().join(format!(
                "pandora-filesystem-test-{}-{}",
                std::process::id(),
                NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(unix)]
    fn create_file_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(source, destination)
    }

    #[cfg(windows)]
    fn create_file_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(source, destination)
    }

    #[cfg(not(any(unix, windows)))]
    fn create_file_symlink(_source: &Path, _destination: &Path) -> std::io::Result<()> {
        Err(std::io::Error::other("symlink test unsupported"))
    }
}
