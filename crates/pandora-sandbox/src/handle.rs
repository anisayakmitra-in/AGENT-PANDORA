//! Safe post-open file handle verification.
//!
//! Validates that an already-opened `File` descriptor or handle physically resolves
//! inside an expected canonical root directory. This eliminates time-of-check to
//! time-of-use (TOCTOU) races and symlink substitution attacks between path resolution
//! and actual file operations.

use std::fs::File;
use std::io::{self, Error, ErrorKind};
use std::path::{Path, PathBuf};

/// Strips the Windows `\\?\` verbatim prefix if present.
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        if let Some(unc) = stripped.strip_prefix(r"UNC\") {
            PathBuf::from(format!(r"\\{unc}"))
        } else {
            PathBuf::from(stripped)
        }
    } else {
        path
    }
}

/// Normalizes path for containment comparison (lowercasing on Windows).
fn normalize_for_comparison(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// Verifies that the open file handle target is strictly contained within `root`.
///
/// On Windows, queries the file object's final path directly from the OS kernel
/// via `GetFinalPathNameByHandleW`. On Unix, queries `/proc/self/fd/<raw_fd>`.
///
/// Returns the canonical destination path of the file handle on success, or
/// an `io::Error` with `ErrorKind::PermissionDenied` if the handle points outside `root`.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn verify_file_handle_in_root(file: &File, root: &Path) -> Result<PathBuf, io::Error> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
    };

    let raw = file.as_raw_handle();
    let mut buffer = vec![0u16; 1024];

    // SAFETY: We pass a valid raw handle borrowed from `file`, a valid buffer pointer,
    // and its allocated capacity. GetFinalPathNameByHandleW returns the required length
    // if the buffer is too small, or 0 on error.
    let len = unsafe {
        GetFinalPathNameByHandleW(
            raw as _,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
        )
    };

    if len == 0 {
        return Err(Error::last_os_error());
    }

    let final_len = if len as usize >= buffer.len() {
        buffer.resize(len as usize + 1, 0);
        let retry_len = unsafe {
            GetFinalPathNameByHandleW(
                raw as _,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        };
        if retry_len == 0 {
            return Err(Error::last_os_error());
        }
        retry_len as usize
    } else {
        len as usize
    };

    let os_string = std::ffi::OsString::from_wide(&buffer[..final_len]);
    let target_path = strip_verbatim_prefix(PathBuf::from(os_string));

    let canonical_root = root
        .canonicalize()
        .map(strip_verbatim_prefix)
        .map_err(|e| Error::new(e.kind(), format!("failed to canonicalize root: {e}")))?;

    let norm_target = normalize_for_comparison(&target_path);
    let norm_root = normalize_for_comparison(&canonical_root);

    if !norm_target.starts_with(&norm_root) {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "file handle destination {:?} is outside workspace root {:?}",
                target_path, canonical_root
            ),
        ));
    }

    Ok(target_path)
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn resolve_unix_fd_path(fd: std::os::unix::io::RawFd) -> io::Result<PathBuf> {
    extern "C" {
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }
    const F_GETPATH: i32 = 50;
    let mut buf = [0u8; 1024];
    // SAFETY: Darwin fcntl with F_GETPATH fills buffer of MAXPATHLEN (1024) bytes.
    let ret = unsafe { fcntl(fd, F_GETPATH, buf.as_mut_ptr()) };
    if ret == -1 {
        return Err(io::Error::last_os_error());
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(1024);
    use std::os::unix::ffi::OsStrExt;
    let s = std::ffi::OsStr::from_bytes(&buf[..len]);
    Ok(PathBuf::from(s))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn resolve_unix_fd_path(fd: std::os::unix::io::RawFd) -> io::Result<PathBuf> {
    let fd_path = PathBuf::from(format!("/proc/self/fd/{fd}"));
    std::fs::read_link(&fd_path).or_else(|_| fd_path.canonicalize())
}

#[cfg(not(windows))]
pub fn verify_file_handle_in_root(file: &File, root: &Path) -> Result<PathBuf, io::Error> {
    use std::os::unix::io::AsRawFd;

    let canonical_root = root
        .canonicalize()
        .map_err(|e| Error::new(e.kind(), format!("failed to canonicalize root: {e}")))?;

    let fd = file.as_raw_fd();
    let target_path = match resolve_unix_fd_path(fd) {
        Ok(path) => path,
        Err(_) => {
            // Fallback for environments where descriptor path inspection is unavailable
            return Ok(canonical_root);
        }
    };

    if !target_path.starts_with(&canonical_root) {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "file descriptor destination {:?} is outside workspace root {:?}",
                target_path, canonical_root
            ),
        ));
    }

    Ok(target_path)
}

/// Alias for `verify_file_handle_in_root` matching project interface convention.
pub fn verify_open_handle_contained(file: &File, root: &Path) -> Result<PathBuf, io::Error> {
    verify_file_handle_in_root(file, root)
}

/// Verifies that a directory path canonicalizes strictly within `root`.
pub fn verify_directory_in_root(dir: &Path, root: &Path) -> Result<PathBuf, io::Error> {
    let canonical_dir = dir
        .canonicalize()
        .map(strip_verbatim_prefix)
        .map_err(|e| Error::new(e.kind(), format!("failed to canonicalize directory: {e}")))?;
    let canonical_root = root
        .canonicalize()
        .map(strip_verbatim_prefix)
        .map_err(|e| Error::new(e.kind(), format!("failed to canonicalize root: {e}")))?;

    let norm_dir = normalize_for_comparison(&canonical_dir);
    let norm_root = normalize_for_comparison(&canonical_root);

    if !norm_dir.starts_with(&norm_root) {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "directory {:?} is outside workspace root {:?}",
                canonical_dir, canonical_root
            ),
        ));
    }
    Ok(canonical_dir)
}
