use std::ffi::OsStr;
use std::path::PathBuf;

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterConfig {
    pub maa_executable: Option<PathBuf>,
    pub state_directory: Option<PathBuf>,
    pub retention_days: Option<u16>,
    #[serde(default)]
    pub redaction_patterns: Vec<String>,
}

impl AdapterConfig {
    pub fn retention_days(&self) -> u16 {
        self.retention_days.unwrap_or(7)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverySource {
    Explicit,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExecutable {
    pub path: PathBuf,
    pub source: DiscoverySource,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid adapter config: {0}")]
    InvalidToml(#[from] toml::de::Error),
    #[error("configured maa executable must be an absolute path: {0}")]
    RelativeExecutable(PathBuf),
    #[error("configured maa executable does not exist: {0}")]
    ExecutableNotFound(PathBuf),
    #[error("configured maa executable is not a file: {0}")]
    ExecutableNotFile(PathBuf),
    #[error("could not find maa.exe or maa-cli.exe on PATH")]
    MaaCliNotFound,
}

impl AdapterConfig {
    pub fn from_toml(input: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(input)?)
    }

    pub fn resolve_maa_executable(
        &self,
        search_path: Option<&OsStr>,
    ) -> Result<ResolvedExecutable, ConfigError> {
        if let Some(path) = self.maa_executable.clone() {
            if !path.is_absolute() {
                return Err(ConfigError::RelativeExecutable(path));
            }
            if !path.exists() {
                return Err(ConfigError::ExecutableNotFound(path));
            }
            if !path.is_file() {
                return Err(ConfigError::ExecutableNotFile(path));
            }

            return Ok(ResolvedExecutable {
                path,
                source: DiscoverySource::Explicit,
            });
        }

        let inherited_path;
        let search_path = match search_path {
            Some(path) => Some(path),
            None => {
                inherited_path = std::env::var_os("PATH");
                inherited_path.as_deref()
            }
        };

        if let Some(search_path) = search_path {
            let directories: Vec<_> = std::env::split_paths(search_path).collect();
            for executable_name in ["maa.exe", "maa-cli.exe"] {
                for directory in &directories {
                    let candidate = directory.join(executable_name);
                    if candidate.is_file() {
                        return Ok(ResolvedExecutable {
                            path: candidate,
                            source: DiscoverySource::Path,
                        });
                    }
                }
            }
        }

        Err(ConfigError::MaaCliNotFound)
    }
}
