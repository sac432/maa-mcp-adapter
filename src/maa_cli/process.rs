use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use thiserror::Error;
use tokio::process::Command;

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
