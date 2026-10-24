//! Unit and integration tests for Job Object process confinement and handle verification.

use pandora_sandbox::{Availability, PlatformFamily};
use std::fs::{self, File};

#[test]
fn availability_reports_job_object_support_on_windows() {
    let win = Availability::probe(PlatformFamily::Windows);
    assert!(win.job_object_supported());
    assert!(win.supports_job_object());
    assert!(win.has_process_confinement());

    let unsupported = Availability::probe(PlatformFamily::Unsupported);
    assert!(!unsupported.job_object_supported());
    assert!(!unsupported.has_process_confinement());
}

#[cfg(windows)]
#[test]
fn windows_job_object_creation_and_limits_verification() {
    use pandora_sandbox::job_object::{
        DEFAULT_ACTIVE_PROCESS_LIMIT, DEFAULT_JOB_MEMORY_LIMIT_BYTES, JobObject,
    };

    let job = JobObject::create_confined().expect("JobObject::create_confined should succeed");
    let limits = job.query_limits().expect("query_limits should succeed");

    assert_eq!(limits.active_process_limit, DEFAULT_ACTIVE_PROCESS_LIMIT);
    assert_eq!(
        limits.job_memory_limit_bytes,
        DEFAULT_JOB_MEMORY_LIMIT_BYTES
    );
    assert!(limits.kill_on_job_close);
    assert!(limits.die_on_unhandled_exception);
    assert!(limits.breakaway_prevented);
}

#[cfg(windows)]
#[test]
fn assign_process_to_job_object_succeeds() {
    use pandora_sandbox::job_object::JobObject;
    use std::os::windows::io::AsRawHandle;
    use std::process::Command;

    let job = JobObject::create_confined().expect("create_confined should succeed");
    let mut child = Command::new("cmd")
        .args(["/C", "echo test"])
        .spawn()
        .expect("spawn should succeed");

    let assign_result = job.assign_process_handle(child.as_raw_handle());
    assert!(
        assign_result.is_ok(),
        "assigning child to job object must succeed"
    );

    let status = child.wait().expect("wait should succeed");
    assert!(status.success());
}

#[test]
fn verify_file_handle_in_root_accepts_contained_file() {
    let temp_dir = std::env::temp_dir().join(format!("pandora-handle-test-{}", std::process::id()));
    fs::create_dir_all(&temp_dir).unwrap();
    let file_path = temp_dir.join("test.txt");
    fs::write(&file_path, b"test content").unwrap();

    let file = File::open(&file_path).unwrap();
    let result = pandora_sandbox::handle::verify_file_handle_in_root(&file, &temp_dir);
    assert!(
        result.is_ok(),
        "contained file handle must verify successfully"
    );

    let _ = fs::remove_file(&file_path);
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn verify_file_handle_in_root_rejects_file_outside_root() {
    let base_temp = std::env::temp_dir();
    let root_dir = base_temp.join(format!("pandora-root-{}", std::process::id()));
    let outside_dir = base_temp.join(format!("pandora-outside-{}", std::process::id()));
    fs::create_dir_all(&root_dir).unwrap();
    fs::create_dir_all(&outside_dir).unwrap();

    let outside_file_path = outside_dir.join("outside.txt");
    fs::write(&outside_file_path, b"outside content").unwrap();

    let file = File::open(&outside_file_path).unwrap();
    let result = pandora_sandbox::handle::verify_file_handle_in_root(&file, &root_dir);
    assert!(
        result.is_err(),
        "file handle pointing outside root must be rejected"
    );
    assert_eq!(
        result.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );

    let _ = fs::remove_file(&outside_file_path);
    let _ = fs::remove_dir_all(&root_dir);
    let _ = fs::remove_dir_all(&outside_dir);
}
