//! macOS process control. Never search by executable name or signal a shared group.
use crate::domain::{ProcessStartIdentity, RunId};
use std::{
    process::Child,
    time::{Duration, Instant},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("macOS process API failed: {0}")]
    MacOs(#[from] std::io::Error),
    #[error("process identity no longer matches the recorded process")]
    IdentityMismatch,
    #[error("terminated process did not exit within the safety timeout")]
    TerminationTimeout,
}

fn info(pid: u32) -> Result<libc::proc_bsdinfo, PlatformError> {
    if pid <= 1 || pid > i32::MAX as u32 {
        return Err(PlatformError::IdentityMismatch);
    }
    // SAFETY: proc_bsdinfo is plain C data and the buffer size matches its allocation.
    let mut data: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&data) as i32;
    let count = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut data as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if count != size {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(data)
}
fn identity(data: &libc::proc_bsdinfo) -> ProcessStartIdentity {
    ProcessStartIdentity {
        creation_time_100ns: data.pbi_start_tvsec * 10_000_000 + data.pbi_start_tvusec * 10,
    }
}
pub fn harden_stdio_inheritance() -> Result<(), PlatformError> {
    for fd in [0, 1, 2] {
        // SAFETY: fcntl operates only on our standard descriptors; spawned stdio is explicit.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}
pub fn process_identity_for_child(child: &Child) -> Result<ProcessStartIdentity, PlatformError> {
    Ok(identity(&info(child.id())?))
}
pub fn process_identity_matches(
    pid: u32,
    expected: ProcessStartIdentity,
) -> Result<bool, PlatformError> {
    let data = info(pid)?;
    Ok(identity(&data) == expected && data.pbi_status != 5)
}
pub fn assign_new_job(_: &RunId, child: &Child) -> Result<(), PlatformError> {
    if info(child.id())?.pbi_pgid != child.id() {
        return Err(PlatformError::IdentityMismatch);
    }
    Ok(())
}
fn signal_group(
    pid: u32,
    expected: ProcessStartIdentity,
    signal: i32,
) -> Result<(), PlatformError> {
    let data = info(pid)?;
    if identity(&data) != expected || data.pbi_pgid != pid || data.pbi_status == 5 {
        return Err(PlatformError::IdentityMismatch);
    }
    // SAFETY: a strictly positive PID is a verified, adapter-created group leader.
    if unsafe { libc::kill(-(pid as i32), signal) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
pub fn request_graceful_stop(
    pid: u32,
    expected: ProcessStartIdentity,
) -> Result<(), PlatformError> {
    signal_group(pid, expected, libc::SIGTERM)
}
pub fn terminate_job(
    _: &RunId,
    _: u32,
    pid: u32,
    expected: ProcessStartIdentity,
) -> Result<(), PlatformError> {
    signal_group(pid, expected, libc::SIGKILL)?;
    wait_for_exit(pid, expected, Duration::from_secs(5))?
        .ok_or(PlatformError::TerminationTimeout)?;
    Ok(())
}
pub fn wait_for_exit(
    pid: u32,
    expected: ProcessStartIdentity,
    timeout: Duration,
) -> Result<Option<i32>, PlatformError> {
    let deadline = Instant::now() + timeout;
    loop {
        match info(pid) {
            Ok(data) if identity(&data) != expected => return Err(PlatformError::IdentityMismatch),
            Ok(data) if data.pbi_status == 5 => return Ok(Some(-1)),
            Err(PlatformError::MacOs(e)) if e.raw_os_error() == Some(libc::ESRCH) => {
                return Ok(Some(-1));
            }
            Err(e) => return Err(e),
            _ => {}
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
pub fn terminate_if_matches(pid: u32, expected: ProcessStartIdentity) -> Result<(), PlatformError> {
    // Emulator shutdown targets only the recorded process, never unrelated app instances.
    if !process_identity_matches(pid, expected)? {
        return Err(PlatformError::IdentityMismatch);
    }
    // SAFETY: positive, identity-checked PID; no name-based or global shutdown.
    if unsafe { libc::kill(pid as i32, libc::SIGKILL) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    wait_for_exit(pid, expected, Duration::from_secs(5))?
        .ok_or(PlatformError::TerminationTimeout)?;
    Ok(())
}

pub fn process_identity_for_pid(pid: u32) -> Result<ProcessStartIdentity, PlatformError> {
    Ok(identity(&info(pid)?))
}
