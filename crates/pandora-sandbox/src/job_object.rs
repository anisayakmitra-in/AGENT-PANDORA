//! Windows Job Object process confinement primitives.
//!
//! Enforces OS-level resource limits, process tree bounding, and breakaway prevention
//! via Win32 Job Objects.

#[cfg(windows)]
use std::io::{self, Error};
#[cfg(windows)]
use std::mem::size_of;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_JOB_MEMORY,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// Default active process limit in a sandboxed job object.
pub const DEFAULT_ACTIVE_PROCESS_LIMIT: u32 = 32;

/// Default job memory limit (2 GB).
pub const DEFAULT_JOB_MEMORY_LIMIT_BYTES: usize = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobLimits {
    pub active_process_limit: u32,
    pub job_memory_limit_bytes: usize,
    pub kill_on_job_close: bool,
    pub die_on_unhandled_exception: bool,
    pub breakaway_prevented: bool,
}

#[cfg(windows)]
#[derive(Debug)]
pub struct JobObject {
    handle: HANDLE,
}

#[cfg(windows)]
#[allow(unsafe_code)]
impl JobObject {
    /// Creates a new anonymous Job Object with standard security limits applied:
    /// - Active process limit (32)
    /// - Job memory limit (2 GB)
    /// - Kill on job close
    /// - Die on unhandled exception
    /// - Breakaway prevention (disallowing child processes from breaking out of the job)
    pub fn create_confined() -> io::Result<Self> {
        Self::create_with_limits(DEFAULT_ACTIVE_PROCESS_LIMIT, DEFAULT_JOB_MEMORY_LIMIT_BYTES)
    }

    pub fn create_with_limits(
        active_process_limit: u32,
        memory_limit_bytes: usize,
    ) -> io::Result<Self> {
        // SAFETY: Calling CreateJobObjectW with null security attributes and name.
        let handle = unsafe { CreateJobObjectW(core::ptr::null(), core::ptr::null()) };
        if handle.is_null() {
            return Err(Error::last_os_error());
        }

        let job = Self { handle };
        job.apply_limits(active_process_limit, memory_limit_bytes)?;
        Ok(job)
    }

    fn apply_limits(&self, active_process_limit: u32, memory_limit_bytes: usize) -> io::Result<()> {
        // SAFETY: Initializing JOBOBJECT_EXTENDED_LIMIT_INFORMATION struct.
        unsafe {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = core::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
                | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
                | JOB_OBJECT_LIMIT_JOB_MEMORY;
            info.BasicLimitInformation.ActiveProcessLimit = active_process_limit;
            info.JobMemoryLimit = memory_limit_bytes;

            let result = SetInformationJobObject(
                self.handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if result == 0 {
                return Err(Error::last_os_error());
            }
        }
        Ok(())
    }

    /// Assigns the current process to this Job Object.
    pub fn assign_current_process(&self) -> io::Result<()> {
        // SAFETY: GetCurrentProcess returns a pseudo-handle for the calling process.
        unsafe {
            let current = GetCurrentProcess();
            let result = AssignProcessToJobObject(self.handle, current);
            if result == 0 {
                return Err(Error::last_os_error());
            }
        }
        Ok(())
    }

    /// Assigns an external process handle to this Job Object.
    pub fn assign_process_handle(&self, handle: std::os::windows::io::RawHandle) -> io::Result<()> {
        // SAFETY: AssignProcessToJobObject assigns the target process to the job object.
        unsafe {
            let result = AssignProcessToJobObject(self.handle, handle as HANDLE);
            if result == 0 {
                return Err(Error::last_os_error());
            }
        }
        Ok(())
    }

    /// Queries the extended limit information from the Job Object to verify active settings.
    pub fn query_limits(&self) -> io::Result<JobLimits> {
        // SAFETY: Querying JobObjectExtendedLimitInformation into zeroed buffer.
        unsafe {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = core::mem::zeroed();
            let mut return_length: u32 = 0;
            let result = QueryInformationJobObject(
                self.handle,
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                &mut return_length,
            );
            if result == 0 {
                return Err(Error::last_os_error());
            }

            let flags = info.BasicLimitInformation.LimitFlags;
            Ok(JobLimits {
                active_process_limit: info.BasicLimitInformation.ActiveProcessLimit,
                job_memory_limit_bytes: info.JobMemoryLimit,
                kill_on_job_close: (flags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE) != 0,
                die_on_unhandled_exception: (flags & JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION)
                    != 0,
                breakaway_prevented: (flags & 0x00000800/* JOB_OBJECT_LIMIT_BREAKAWAY_OK */) == 0,
            })
        }
    }

    pub fn raw_handle(&self) -> HANDLE {
        self.handle
    }
}

#[cfg(windows)]
impl Drop for JobObject {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: Closing valid handle.
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(not(windows))]
#[derive(Debug)]
pub struct JobObject;

#[cfg(not(windows))]
impl JobObject {
    pub fn create_confined() -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Job Objects are only supported on Windows",
        ))
    }
}

pub fn is_supported() -> bool {
    #[cfg(windows)]
    {
        true
    }
    #[cfg(not(windows))]
    {
        false
    }
}
