use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use thiserror::Error;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use super::capabilities;
use super::process::{self, ProcessError};
use super::{MaaCliAdapter, MaaEnvironment};

const PROBE_TASK: &[u8] = br#"{"tasks":[]}"#;

#[derive(Debug, Error)]
pub enum MaaCliError {
    #[error("maa-cli version probe failed: {0}")]
    VersionCommand(#[source] ProcessError),
    #[error("maa-cli version output did not contain a maa-cli version")]
    InvalidVersionOutput,
    #[error("maa-cli config directory probe failed: {0}")]
    ConfigDirectoryCommand(#[source] ProcessError),
    #[error("maa-cli config directory output must contain exactly one absolute path")]
    InvalidConfigDirectoryOutput,
    #[error("maa-cli config directory is not a directory: {0}")]
    InvalidConfigDirectory(PathBuf),
    #[error("adapter state directory must be absolute: {0}")]
    RelativeStateDirectory(PathBuf),
    #[error("adapter state directory must not overlap the maa-cli config directory")]
    StateDirectoryOverlapsMaaConfig,
    #[error("failed to access {path}: {source}")]
    FileSystem {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub(super) async fn inspect_environment(
    adapter: &MaaCliAdapter,
    state_directory: &Path,
) -> Result<MaaEnvironment, MaaCliError> {
    let version_output = run_probe(adapter, &["version"])
        .await
        .map_err(MaaCliError::VersionCommand)?;
    let (cli_version, core_version) = parse_versions(&version_output.stdout)?;

    let config_output = run_probe(adapter, &["dir", "config"])
        .await
        .map_err(MaaCliError::ConfigDirectoryCommand)?;
    let config_dir = parse_config_directory(&config_output.stdout).await?;
    let profiles = discover_profiles(&config_dir).await?;

    let mut diagnostics = Vec::new();
    let run_help = optional_probe(adapter, &["run", "--help"], "run help", &mut diagnostics).await;
    let global_help = optional_probe(adapter, &["--help"], "global help", &mut diagnostics).await;
    let mut capabilities = capabilities::from_help(&run_help, &global_help);

    if capabilities.batch && capabilities.profile && capabilities.dry_run && capabilities.log_file {
        if let Some(profile) = preferred_profile(&profiles) {
            capabilities.absolute_task_path = probe_absolute_task_path(
                adapter,
                state_directory,
                &config_dir,
                profile,
                &mut diagnostics,
            )
            .await?;
        } else {
            diagnostics.push("no supported maa-cli profile was found".to_owned());
        }
    } else {
        diagnostics.push("required maa-cli command options could not be confirmed".to_owned());
    }

    Ok(MaaEnvironment {
        executable: adapter.executable.clone(),
        cli_version,
        core_version,
        config_dir,
        profiles,
        capabilities,
        diagnostics,
    })
}

async fn run_probe(
    adapter: &MaaCliAdapter,
    args: &[&str],
) -> Result<process::ProcessOutput, ProcessError> {
    let args = args.iter().map(OsString::from).collect::<Vec<_>>();
    process::run(&adapter.executable, &args, adapter.timeout).await
}

async fn optional_probe(
    adapter: &MaaCliAdapter,
    args: &[&str],
    label: &str,
    diagnostics: &mut Vec<String>,
) -> String {
    match run_probe(adapter, args).await {
        Ok(output) => output.stdout,
        Err(_) => {
            diagnostics.push(format!("{label} probe failed"));
            String::new()
        }
    }
}

pub(crate) fn parse_versions(output: &str) -> Result<(String, Option<String>), MaaCliError> {
    let cli_version = output.lines().find_map(|line| {
        line.trim()
            .strip_prefix("maa-cli v")
            .filter(|version| !version.is_empty())
            .map(ToOwned::to_owned)
    });
    let core_version = output.lines().find_map(|line| {
        line.trim()
            .strip_prefix("MaaCore ")
            .filter(|version| !version.is_empty())
            .map(ToOwned::to_owned)
    });

    cli_version
        .map(|cli_version| (cli_version, core_version))
        .ok_or(MaaCliError::InvalidVersionOutput)
}

pub(crate) async fn parse_config_directory(output: &str) -> Result<PathBuf, MaaCliError> {
    let lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() != 1 {
        return Err(MaaCliError::InvalidConfigDirectoryOutput);
    }

    let directory = PathBuf::from(lines[0]);
    if !directory.is_absolute() {
        return Err(MaaCliError::InvalidConfigDirectoryOutput);
    }
    let canonical = tokio::fs::canonicalize(&directory)
        .await
        .map_err(|source| MaaCliError::FileSystem {
            path: directory.clone(),
            source,
        })?;
    let metadata =
        tokio::fs::metadata(&canonical)
            .await
            .map_err(|source| MaaCliError::FileSystem {
                path: canonical.clone(),
                source,
            })?;
    if !metadata.is_dir() {
        return Err(MaaCliError::InvalidConfigDirectory(canonical));
    }

    Ok(canonical)
}

async fn discover_profiles(config_directory: &Path) -> Result<Vec<String>, MaaCliError> {
    let profiles_directory = config_directory.join("profiles");
    let mut directory = match tokio::fs::read_dir(&profiles_directory).await {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(MaaCliError::FileSystem {
                path: profiles_directory,
                source,
            });
        }
    };
    let mut profiles = BTreeSet::new();

    while let Some(entry) =
        directory
            .next_entry()
            .await
            .map_err(|source| MaaCliError::FileSystem {
                path: profiles_directory.clone(),
                source,
            })?
    {
        let file_type = entry
            .file_type()
            .await
            .map_err(|source| MaaCliError::FileSystem {
                path: entry.path(),
                source,
            })?;
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        let supported_extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "toml" | "json" | "yaml" | "yml"
                )
            });
        if !supported_extension {
            continue;
        }
        if let Some(profile) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| is_valid_profile_name(stem))
        {
            profiles.insert(profile.to_owned());
        }
    }

    Ok(profiles.into_iter().collect())
}

fn is_valid_profile_name(profile: &str) -> bool {
    !profile.is_empty()
        && profile.trim() == profile
        && profile != "."
        && profile != ".."
        && !profile.chars().any(char::is_control)
}

fn preferred_profile(profiles: &[String]) -> Option<&str> {
    profiles
        .iter()
        .find(|profile| profile.as_str() == "default")
        .or_else(|| profiles.first())
        .map(String::as_str)
}

async fn probe_absolute_task_path(
    adapter: &MaaCliAdapter,
    state_directory: &Path,
    config_directory: &Path,
    profile: &str,
    diagnostics: &mut Vec<String>,
) -> Result<bool, MaaCliError> {
    if !state_directory.is_absolute() {
        return Err(MaaCliError::RelativeStateDirectory(
            state_directory.to_path_buf(),
        ));
    }
    tokio::fs::create_dir_all(state_directory)
        .await
        .map_err(|source| MaaCliError::FileSystem {
            path: state_directory.to_path_buf(),
            source,
        })?;
    let canonical_state = tokio::fs::canonicalize(state_directory)
        .await
        .map_err(|source| MaaCliError::FileSystem {
            path: state_directory.to_path_buf(),
            source,
        })?;
    if canonical_state.starts_with(config_directory)
        || config_directory.starts_with(&canonical_state)
    {
        return Err(MaaCliError::StateDirectoryOverlapsMaaConfig);
    }

    let probe_path = state_directory.join(format!("capability-probe-{}.json", Uuid::new_v4()));
    let mut probe_file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe_path)
        .await
        .map_err(|source| MaaCliError::FileSystem {
            path: probe_path.clone(),
            source,
        })?;
    probe_file
        .write_all(PROBE_TASK)
        .await
        .map_err(|source| MaaCliError::FileSystem {
            path: probe_path.clone(),
            source,
        })?;
    probe_file
        .flush()
        .await
        .map_err(|source| MaaCliError::FileSystem {
            path: probe_path.clone(),
            source,
        })?;
    drop(probe_file);

    let args = [
        OsString::from("--batch"),
        OsString::from("run"),
        probe_path.as_os_str().to_owned(),
        OsString::from("--dry-run"),
        OsString::from("--profile"),
        OsString::from(profile),
    ];
    let result = process::run(&adapter.executable, &args, adapter.timeout).await;
    let cleanup_result = tokio::fs::remove_file(&probe_path).await;
    if let Err(source) = cleanup_result {
        return Err(MaaCliError::FileSystem {
            path: probe_path,
            source,
        });
    }

    match result {
        Ok(_) => Ok(true),
        Err(_) => {
            diagnostics.push("absolute task path dry-run failed".to_owned());
            Ok(false)
        }
    }
}
