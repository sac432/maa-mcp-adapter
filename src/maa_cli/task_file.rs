use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::domain::DailyPlan;
pub use crate::domain::RunId;
use crate::planning::DomainError;
use crate::planning::normalize::{CanonicalTask, normalize_plan};
use crate::storage::atomic::{WriteMode, write_atomically};
use crate::storage::{StorageError, resolve_state_directory};

use super::MaaCliAdapter;
use super::process::{self, ProcessError};

#[derive(Debug, Error)]
pub enum TaskFileError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("failed to serialize generated MAA task file: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("task file operation failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("run directory already exists")]
    RunAlreadyExists,
    #[error("generated task path escaped the configured state directory")]
    PathEscapedStateDirectory,
    #[error("dry-run profile does not match the generated task artifact")]
    ProfileMismatch,
    #[error("generated task file changed after validation")]
    IntegrityMismatch,
    #[error(transparent)]
    Process(#[from] ProcessError),
}

pub struct TaskFileStore {
    state_directory: PathBuf,
    runs_directory: PathBuf,
    policy: TaskFilePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskFilePolicy {
    mall_blacklist: Vec<String>,
}

impl TaskFilePolicy {
    pub fn new(mut mall_blacklist: Vec<String>) -> Self {
        if !mall_blacklist.iter().any(|item| item == "加急许可") {
            mall_blacklist.push("加急许可".to_owned());
        }
        Self { mall_blacklist }
    }
}

impl Default for TaskFilePolicy {
    fn default() -> Self {
        Self::new(vec!["加急许可".to_owned()])
    }
}

#[derive(Debug, Clone)]
pub struct TaskFileArtifact {
    path: PathBuf,
    sha256: [u8; 32],
    profile: String,
}

impl TaskFileArtifact {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn sha256_hex(&self) -> String {
        let mut value = String::with_capacity(64);
        for byte in self.sha256 {
            use std::fmt::Write as _;
            write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
        }
        value
    }

    pub(crate) fn verify_unchanged(&self) -> Result<(), TaskFileError> {
        let bytes = fs::read(&self.path).map_err(|source| TaskFileError::Io {
            path: self.path.clone(),
            source,
        })?;
        if sha256(&bytes) == self.sha256 {
            Ok(())
        } else {
            Err(TaskFileError::IntegrityMismatch)
        }
    }
}

impl TaskFileStore {
    pub fn new(state_directory: PathBuf) -> Result<Self, TaskFileError> {
        Self::with_policy(state_directory, TaskFilePolicy::default())
    }

    pub fn with_policy(
        state_directory: PathBuf,
        policy: TaskFilePolicy,
    ) -> Result<Self, TaskFileError> {
        let state_directory = resolve_state_directory(Some(&state_directory), None)?;
        fs::create_dir_all(&state_directory).map_err(|source| TaskFileError::Io {
            path: state_directory.clone(),
            source,
        })?;
        let state_directory =
            fs::canonicalize(&state_directory).map_err(|source| TaskFileError::Io {
                path: state_directory,
                source,
            })?;
        let runs_directory = state_directory.join("runs");
        fs::create_dir_all(&runs_directory).map_err(|source| TaskFileError::Io {
            path: runs_directory.clone(),
            source,
        })?;

        Ok(Self {
            state_directory,
            runs_directory,
            policy,
        })
    }

    pub fn generate(
        &self,
        run_id: RunId,
        plan: &DailyPlan,
    ) -> Result<TaskFileArtifact, TaskFileError> {
        let rendered = render_task_file_with_policy(plan, &self.policy)?;
        let run_directory = self.runs_directory.join(run_id.to_string());
        match fs::create_dir(&run_directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(TaskFileError::RunAlreadyExists);
            }
            Err(source) => {
                return Err(TaskFileError::Io {
                    path: run_directory,
                    source,
                });
            }
        }
        let run_directory =
            fs::canonicalize(&run_directory).map_err(|source| TaskFileError::Io {
                path: run_directory,
                source,
            })?;
        if !run_directory.starts_with(&self.state_directory) {
            return Err(TaskFileError::PathEscapedStateDirectory);
        }

        let path = run_directory.join("task.toml");
        let bytes = rendered.as_bytes();
        write_atomically(&path, bytes, WriteMode::Create)?;
        let path = fs::canonicalize(&path).map_err(|source| TaskFileError::Io { path, source })?;
        if !path.starts_with(&self.state_directory) {
            return Err(TaskFileError::PathEscapedStateDirectory);
        }

        Ok(TaskFileArtifact {
            path,
            sha256: sha256(bytes),
            profile: plan.profile.clone(),
        })
    }
}

#[derive(Debug, Serialize)]
pub struct GeneratedTaskFile {
    tasks: Vec<GeneratedTask>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "params")]
pub enum GeneratedTask {
    #[serde(rename = "StartUp")]
    StartUp(StartUpParams),
    #[serde(rename = "Fight")]
    Fight(FightParams),
    #[serde(rename = "Recruit")]
    Recruit(RecruitParams),
    #[serde(rename = "Infrast")]
    Infrast(InfrastParams),
    #[serde(rename = "Mall")]
    Mall(MallParams),
    #[serde(rename = "Award")]
    Award(AwardParams),
    #[serde(rename = "CloseDown")]
    CloseDown(CloseDownParams),
}

#[derive(Debug, Serialize)]
pub struct StartUpParams {
    start_game_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct FightParams {
    stage: String,
    medicine: u8,
    stone: u8,
    times: u32,
}

#[derive(Debug, Serialize)]
pub struct RecruitParams {
    select: [u8; 2],
    confirm: Vec<u8>,
    times: u8,
    expedite: bool,
    expedite_times: u8,
}

#[derive(Debug, Serialize)]
pub struct InfrastParams {
    mode: u32,
    facility: [&'static str; 7],
}

#[derive(Debug, Serialize)]
pub struct MallParams {
    visit_friends: bool,
    shopping: bool,
    blacklist: Vec<String>,
    force_shopping_if_credit_full: bool,
}

#[derive(Debug, Serialize)]
pub struct AwardParams {
    award: bool,
    mail: bool,
    recruit: bool,
    orundum: bool,
    mining: bool,
    specialaccess: bool,
}

#[derive(Debug, Serialize)]
pub struct CloseDownParams {}

pub fn render_task_file(plan: &DailyPlan) -> Result<String, TaskFileError> {
    render_task_file_with_policy(plan, &TaskFilePolicy::default())
}

pub fn render_task_file_with_policy(
    plan: &DailyPlan,
    policy: &TaskFilePolicy,
) -> Result<String, TaskFileError> {
    let normalized = normalize_plan(plan)?;
    let tasks = normalized
        .tasks
        .into_iter()
        .map(|task| generated_task(task, policy))
        .collect();
    Ok(toml::to_string_pretty(&GeneratedTaskFile { tasks })?)
}

fn generated_task(task: CanonicalTask, policy: &TaskFilePolicy) -> GeneratedTask {
    match task {
        CanonicalTask::StartUp { start_game } => GeneratedTask::StartUp(StartUpParams {
            start_game_enabled: start_game,
        }),
        CanonicalTask::Fight {
            stage,
            times,
            medicine,
            stone,
            ..
        } => GeneratedTask::Fight(FightParams {
            stage,
            medicine,
            stone,
            times,
        }),
        CanonicalTask::Recruit {
            max_recruits,
            confirm_recruitment,
            use_expedited_plans,
        } => GeneratedTask::Recruit(RecruitParams {
            select: [4, 5],
            confirm: if confirm_recruitment {
                vec![3, 4]
            } else {
                Vec::new()
            },
            times: max_recruits,
            expedite: use_expedited_plans,
            expedite_times: if use_expedited_plans { max_recruits } else { 0 },
        }),
        CanonicalTask::Infrast {} => GeneratedTask::Infrast(InfrastParams {
            mode: 0,
            facility: [
                "Mfg",
                "Trade",
                "Power",
                "Control",
                "Reception",
                "Office",
                "Dorm",
            ],
        }),
        CanonicalTask::Mall {
            visit_friends,
            shopping,
            force_shopping_if_credit_full,
            ..
        } => GeneratedTask::Mall(MallParams {
            visit_friends,
            shopping,
            blacklist: policy.mall_blacklist.clone(),
            force_shopping_if_credit_full,
        }),
        CanonicalTask::Award {
            daily,
            weekly,
            mail,
            event,
        } => GeneratedTask::Award(AwardParams {
            award: daily || weekly,
            mail,
            recruit: event,
            orundum: event,
            mining: event,
            specialaccess: event,
        }),
        CanonicalTask::CloseDown {} => GeneratedTask::CloseDown(CloseDownParams {}),
    }
}

impl MaaCliAdapter {
    pub async fn dry_run_generated_task(
        &self,
        artifact: &TaskFileArtifact,
        profile: &str,
        address_override: Option<&str>,
    ) -> Result<(), TaskFileError> {
        if profile != artifact.profile {
            return Err(TaskFileError::ProfileMismatch);
        }
        artifact.verify_unchanged()?;
        let mut args = vec![
            OsString::from("--batch"),
            OsString::from("run"),
            artifact.path.as_os_str().to_owned(),
            OsString::from("--dry-run"),
            OsString::from("--profile"),
            OsString::from(&artifact.profile),
        ];
        if let Some(address) = address_override {
            args.push(OsString::from("--addr"));
            args.push(OsString::from(address));
        }
        process::run(&self.executable, &args, self.timeout).await?;
        artifact.verify_unchanged()
    }

    pub fn launch_generated_task(
        &self,
        run_id: &RunId,
        artifact: &TaskFileArtifact,
        stdout_path: &Path,
        stderr_path: &Path,
    ) -> Result<process::BackgroundProcess, TaskFileError> {
        artifact.verify_unchanged()?;
        let args = [
            OsString::from("--batch"),
            OsString::from("run"),
            artifact.path.as_os_str().to_owned(),
            OsString::from("--profile"),
            OsString::from(&artifact.profile),
        ];
        Ok(process::spawn_background(
            run_id,
            &self.executable,
            &args,
            stdout_path,
            stderr_path,
        )?)
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
