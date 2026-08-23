use std::fs;
use std::io;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use pandora_types::{
    ExecutionProfile, ExecutionProfileBinding, ExecutionProfileBindingKind, hash_artifact,
};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(1);

pub(crate) fn execution_profile(executor: &str) -> ExecutionProfile {
    ExecutionProfile::new(
        "2.0.0-alpha.6",
        "windows",
        "x86_64",
        1,
        "workspace-1",
        hash_artifact(b"containment"),
        vec![
            ExecutionProfileBinding::new(
                ExecutionProfileBindingKind::Executor,
                executor,
                Some("2.0.0-alpha.6"),
                hash_artifact(executor.as_bytes()),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

pub(crate) fn new_temp_dir(prefix: &str) -> io::Result<PathBuf> {
    for _ in 0..100 {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("{prefix}-{}-{timestamp}-{sequence}", process::id()));

        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a unique temporary test directory",
    ))
}

#[cfg(test)]
mod tests {
    use super::new_temp_dir;

    #[test]
    fn creates_distinct_existing_directories() {
        let first = new_temp_dir("pandora-runtime-test-support").unwrap();
        let second = new_temp_dir("pandora-runtime-test-support").unwrap();

        assert_ne!(first, second);
        assert!(first.is_dir());
        assert!(second.is_dir());

        let _ = std::fs::remove_dir_all(first);
        let _ = std::fs::remove_dir_all(second);
    }
}
