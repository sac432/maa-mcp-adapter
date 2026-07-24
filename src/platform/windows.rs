use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::process::Child;
use std::time::Duration;
use std::{ffi::OsStr, ptr};

use thiserror::Error;
use windows_sys::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, FILETIME, HANDLE, HANDLE_FLAG_INHERIT,
    INVALID_HANDLE_VALUE, STILL_ACTIVE, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, OpenJobObjectW, TerminateJobObject,
};
use windows_sys::Win32::System::SystemServices::JOB_OBJECT_TERMINATE;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

use crate::domain::ProcessStartIdentity;

const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("Windows process API failed: {0}")]
    Windows(#[source] std::io::Error),
    #[error("process identity no longer matches the recorded process")]
    IdentityMismatch,
    #[error("terminated process did not exit within the safety timeout")]
    TerminationTimeout,
    #[error("named run job already exists")]
    JobAlreadyExists,
}

pub fn harden_stdio_inheritance() -> Result<(), PlatformError> {
    for standard_handle in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: the constants select one of the current process standard handles.
        let handle = unsafe { GetStdHandle(standard_handle) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            continue;
        }
        // SAFETY: handle is a valid current-process handle; mask clears only inheritance.
        if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(last_error());
        }
    }
    Ok(())
}

pub fn process_identity_for_child(child: &Child) -> Result<ProcessStartIdentity, PlatformError> {
    let handle = child.as_raw_handle().cast();
    identity_from_handle(handle)
}

pub fn assign_new_job(run_id: &crate::domain::RunId, child: &Child) -> Result<(), PlatformError> {
    let name = job_name(run_id);
    // SAFETY: the name is a live NUL-terminated UTF-16 buffer and security attributes are null.
    let job = unsafe { CreateJobObjectW(ptr::null(), name.as_ptr()) };
    if job.is_null() {
        return Err(last_error());
    }
    let already_exists = std::io::Error::last_os_error().raw_os_error() == Some(183);
    if already_exists {
        close_handle(job);
        return Err(PlatformError::JobAlreadyExists);
    }
    let process = child.as_raw_handle().cast();
    // SAFETY: both handles are valid for the duration of this call.
    let assigned = unsafe { AssignProcessToJobObject(job, process) };
    let result = (|| {
        if assigned == 0 {
            return Err(last_error());
        }
        let mut remote_job_handle: HANDLE = ptr::null_mut();
        // SAFETY: source and target process handles are valid. The duplicate intentionally
        // remains in the child so the named job survives this server process.
        if unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                job,
                process,
                &mut remote_job_handle,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(last_error());
        }
        Ok(())
    })();
    close_handle(job);
    result
}

pub fn terminate_job(run_id: &crate::domain::RunId, exit_code: u32) -> Result<(), PlatformError> {
    let name = job_name(run_id);
    // SAFETY: the name is a live NUL-terminated UTF-16 buffer and inheritance is disabled.
    let job = unsafe { OpenJobObjectW(JOB_OBJECT_TERMINATE, 0, name.as_ptr()) };
    if job.is_null() {
        return Err(last_error());
    }
    // SAFETY: job was opened with termination access and remains valid until closed below.
    let result = if unsafe { TerminateJobObject(job, exit_code) } == 0 {
        Err(last_error())
    } else {
        Ok(())
    };
    close_handle(job);
    result
}

pub fn request_graceful_stop(pid: u32) -> Result<(), PlatformError> {
    // SAFETY: pid is the verified process-group id created with CREATE_NEW_PROCESS_GROUP.
    if unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) } == 0 {
        Err(last_error())
    } else {
        Ok(())
    }
}

pub fn wait_for_exit(
    pid: u32,
    expected: ProcessStartIdentity,
    timeout: Duration,
) -> Result<Option<i32>, PlatformError> {
    let handle = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE_ACCESS)?;
    let result = (|| {
        if identity_from_handle(handle)? != expected {
            return Err(PlatformError::IdentityMismatch);
        }
        let timeout_ms = timeout.as_millis().min(u128::from(u32::MAX)) as u32;
        // SAFETY: handle remains valid until the surrounding function closes it.
        match unsafe { WaitForSingleObject(handle, timeout_ms) } {
            WAIT_OBJECT_0 => {
                let mut exit_code = 0;
                // SAFETY: handle is valid and exit_code is writable.
                if unsafe { GetExitCodeProcess(handle, &mut exit_code) } == 0 {
                    Err(last_error())
                } else {
                    Ok(Some(exit_code as i32))
                }
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(last_error()),
        }
    })();
    close_handle(handle);
    result
}

pub fn process_identity_matches(
    pid: u32,
    expected: ProcessStartIdentity,
) -> Result<bool, PlatformError> {
    let handle = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let result = (|| {
        let identity = identity_from_handle(handle)?;
        let mut exit_code = 0;
        // SAFETY: handle is a valid process handle opened above and exit_code is writable.
        if unsafe { GetExitCodeProcess(handle, &mut exit_code) } == 0 {
            return Err(last_error());
        }
        Ok(identity == expected && exit_code == STILL_ACTIVE as u32)
    })();
    close_handle(handle);
    result
}

pub fn terminate_if_matches(pid: u32, expected: ProcessStartIdentity) -> Result<(), PlatformError> {
    let handle = open_process(
        pid,
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | SYNCHRONIZE_ACCESS,
    )?;
    let result = (|| {
        if identity_from_handle(handle)? != expected {
            return Err(PlatformError::IdentityMismatch);
        }
        // SAFETY: identity was checked on the same process handle immediately before termination.
        if unsafe { TerminateProcess(handle, 1) } == 0 {
            return Err(last_error());
        }
        // SAFETY: handle remains valid until the surrounding function closes it.
        if unsafe { WaitForSingleObject(handle, 5_000) } != WAIT_OBJECT_0 {
            return Err(PlatformError::TerminationTimeout);
        }
        Ok(())
    })();
    close_handle(handle);
    result
}

fn open_process(pid: u32, access: u32) -> Result<HANDLE, PlatformError> {
    // SAFETY: OpenProcess is called with a concrete PID and no inherited handle.
    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        Err(last_error())
    } else {
        Ok(handle)
    }
}

fn identity_from_handle(handle: HANDLE) -> Result<ProcessStartIdentity, PlatformError> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: handle belongs to a process and all FILETIME pointers are valid for writes.
    let succeeded =
        unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    if succeeded == 0 {
        Err(last_error())
    } else {
        Ok(ProcessStartIdentity {
            creation_time_100ns: (u64::from(creation.dwHighDateTime) << 32)
                | u64::from(creation.dwLowDateTime),
        })
    }
}

fn close_handle(handle: HANDLE) {
    // SAFETY: handle was returned by OpenProcess and is closed exactly once.
    unsafe {
        CloseHandle(handle);
    }
}

fn last_error() -> PlatformError {
    PlatformError::Windows(std::io::Error::last_os_error())
}

fn job_name(run_id: &crate::domain::RunId) -> Vec<u16> {
    OsStr::new(&format!("Local\\maa-mcp-adapter-{run_id}"))
        .encode_wide()
        .chain(Some(0))
        .collect()
}
