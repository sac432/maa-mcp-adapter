use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

pub mod capabilities;
mod inspect;
pub mod process;

pub use inspect::MaaCliError;

#[derive(Debug, Clone)]
pub struct MaaCliAdapter {
    executable: PathBuf,
    timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaaEnvironment {
    pub executable: PathBuf,
    pub cli_version: String,
    pub core_version: Option<String>,
    pub config_dir: PathBuf,
    pub profiles: Vec<String>,
    pub capabilities: MaaCliCapabilities,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaaCliCapabilities {
    pub batch: bool,
    pub profile: bool,
    pub dry_run: bool,
    pub absolute_task_path: bool,
    pub log_file: bool,
}

impl MaaEnvironment {
    pub fn can_plan(&self) -> bool {
        true
    }

    pub fn can_execute(&self) -> bool {
        !self.profiles.is_empty()
            && self.capabilities.batch
            && self.capabilities.profile
            && self.capabilities.dry_run
            && self.capabilities.absolute_task_path
            && self.capabilities.log_file
    }
}

impl MaaCliAdapter {
    pub fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            timeout: Duration::from_secs(10),
        }
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub async fn inspect_environment(
        &self,
        state_directory: &Path,
    ) -> Result<MaaEnvironment, MaaCliError> {
        inspect::inspect_environment(self, state_directory).await
    }
}
