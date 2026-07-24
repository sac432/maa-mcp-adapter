use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use thiserror::Error;
use tokio::process::Command;

use crate::domain::ProcessStartIdentity;
use crate::platform::{PlatformError, process_identity_for_child};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("failed to start maa-cli: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("maa-cli command timed out after {0:?}")]
    Timeout(Duration),
    #[error("maa-cli command failed with exit code {code:?}")]
    NonZeroExit { code: Option<i32> },
    #[error("maa-cli stdout is not valid UTF-8: {0}")]
    InvalidStdout(#[source] std::string::FromUtf8Error),
    #[error("maa-cli stderr is not valid UTF-8: {0}")]
    InvalidStderr(#[source] std::string::FromUtf8Error),
    #[error("failed to create background log file {path}: {source}")]
    LogFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to inspect spawned process identity: {0}")]
    Identity(#[from] PlatformError),
}

pub struct BackgroundProcess {
    child: std::process::Child,
    identity: ProcessStartIdentity,
}

impl BackgroundProcess {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn identity(&self) -> ProcessStartIdentity {
        self.identity
    }

    pub fn detach(self) {
        drop(self);
    }
}

pub async fn run(
    executable: &Path,
    args: &[OsString],
    timeout: Duration,
) -> Result<ProcessOutput, ProcessError> {
    let mut command = Command::new(executable);
    command.args(args).stdin(Stdio::null()).kill_on_drop(true);

    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| ProcessError::Timeout(timeout))?
        .map_err(ProcessError::Spawn)?;
    let stderr = String::from_utf8(output.stderr).map_err(ProcessError::InvalidStderr)?;

    if !output.status.success() {
        return Err(ProcessError::NonZeroExit {
            code: output.status.code(),
        });
    }

    Ok(ProcessOutput {
        stdout: String::from_utf8(output.stdout).map_err(ProcessError::InvalidStdout)?,
        stderr,
    })
}

pub fn spawn_background(
    executable: &Path,
    args: &[OsString],
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<BackgroundProcess, ProcessError> {
    let stdout = create_log(stdout_path)?;
    let stderr = match create_log(stderr_path) {
        Ok(stderr) => stderr,
        Err(error) => {
            let _ = fs::remove_file(stdout_path);
            return Err(error);
        }
    };
    let mut command = std::process::Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    }
    let mut child = command.spawn().map_err(ProcessError::Spawn)?;
    let identity = match process_identity_for_child(&child) {
        Ok(identity) => identity,
        Err(error) => {
            let _ = child.kill();
            return Err(ProcessError::Identity(error));
        }
    };
    Ok(BackgroundProcess { child, identity })
}

fn create_log(path: &Path) -> Result<std::fs::File, ProcessError> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|source| ProcessError::LogFile {
            path: path.to_path_buf(),
            source,
        })
}
