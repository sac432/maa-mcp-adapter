use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::time::Duration;

use maa_mcp_adapter::config::{AdapterConfig, ConfigError, DiscoverySource};
use maa_mcp_adapter::maa_cli::{MaaCliAdapter, MaaCliError, process::run};
use tempfile::tempdir;

mod support;

#[test]
fn explicit_absolute_executable_takes_priority_over_path() {
    let temp = tempdir().expect("create test directory");
    let explicit = temp.path().join("configured maa.exe");
    let path_dir = temp.path().join("path-bin");
    fs::create_dir(&path_dir).expect("create PATH directory");
    fs::write(&explicit, b"fake executable").expect("create configured executable");
    fs::write(path_dir.join("maa.exe"), b"fake executable").expect("create PATH executable");

    let config = AdapterConfig {
        maa_executable: Some(explicit.clone()),
        state_directory: None,
    };
    let search_path = OsString::from(path_dir);

    let resolved = config
        .resolve_maa_executable(Some(&search_path))
        .expect("resolve configured executable");

    assert_eq!(resolved.path, explicit);
    assert_eq!(resolved.source, DiscoverySource::Explicit);
}

#[test]
fn path_discovery_prefers_maa_exe_over_maa_cli_exe() {
    let temp = tempdir().expect("create test directory");
    let first_dir = temp.path().join("first");
    let second_dir = temp.path().join("second");
    fs::create_dir_all(&first_dir).expect("create first PATH directory");
    fs::create_dir_all(&second_dir).expect("create second PATH directory");
    let maa_cli = first_dir.join("maa-cli.exe");
    let maa = second_dir.join("maa.exe");
    fs::write(&maa_cli, b"fake executable").expect("create maa-cli executable");
    fs::write(&maa, b"fake executable").expect("create maa executable");

    let resolved = AdapterConfig::default()
        .resolve_maa_executable(Some(&joined_path(&[&first_dir, &second_dir])))
        .expect("resolve executable from PATH");

    assert_eq!(resolved.path, maa);
    assert_eq!(resolved.source, DiscoverySource::Path);
}

#[test]
fn path_discovery_falls_back_to_maa_cli_exe() {
    let temp = tempdir().expect("create test directory");
    let path_dir = temp.path().join("bin");
    fs::create_dir(&path_dir).expect("create PATH directory");
    let maa_cli = path_dir.join("maa-cli.exe");
    fs::write(&maa_cli, b"fake executable").expect("create maa-cli executable");

    let resolved = AdapterConfig::default()
        .resolve_maa_executable(Some(&joined_path(&[&path_dir])))
        .expect("resolve maa-cli executable from PATH");

    assert_eq!(resolved.path, maa_cli);
    assert_eq!(resolved.source, DiscoverySource::Path);
}

#[test]
fn path_discovery_fails_closed_when_no_candidate_exists() {
    let temp = tempdir().expect("create test directory");

    let result =
        AdapterConfig::default().resolve_maa_executable(Some(&joined_path(&[temp.path()])));

    assert!(matches!(result, Err(ConfigError::MaaCliNotFound)));
}

#[test]
fn explicit_executable_must_be_absolute() {
    let config = AdapterConfig {
        maa_executable: Some("relative/maa.exe".into()),
        state_directory: None,
    };

    let result = config.resolve_maa_executable(Some(OsString::new().as_os_str()));

    assert!(matches!(
        result,
        Err(ConfigError::RelativeExecutable(path)) if path == Path::new("relative/maa.exe")
    ));
}

#[test]
fn explicit_executable_must_exist() {
    let temp = tempdir().expect("create test directory");
    let missing = temp.path().join("missing-maa.exe");
    let config = AdapterConfig {
        maa_executable: Some(missing.clone()),
        state_directory: None,
    };

    let result = config.resolve_maa_executable(None);

    assert!(matches!(
        result,
        Err(ConfigError::ExecutableNotFound(path)) if path == missing
    ));
}

#[test]
fn explicit_executable_must_be_a_file() {
    let temp = tempdir().expect("create test directory");
    let config = AdapterConfig {
        maa_executable: Some(temp.path().to_path_buf()),
        state_directory: None,
    };

    let result = config.resolve_maa_executable(None);

    assert!(matches!(
        result,
        Err(ConfigError::ExecutableNotFile(path)) if path == temp.path()
    ));
}

#[test]
fn config_toml_is_strict_and_preserves_paths() {
    let temp = tempdir().expect("create test directory");
    let executable = temp.path().join("玛恩纳 maa.exe");
    let state_directory = temp.path().join("adapter state");
    let input = format!(
        "maa_executable = '{}'\nstate_directory = '{}'\n",
        executable.display(),
        state_directory.display()
    );

    let config = AdapterConfig::from_toml(&input).expect("parse adapter config");

    assert_eq!(config.maa_executable.as_deref(), Some(executable.as_path()));
    assert_eq!(
        config.state_directory.as_deref(),
        Some(state_directory.as_path())
    );
    assert!(matches!(
        AdapterConfig::from_toml("unknown_setting = true"),
        Err(ConfigError::InvalidToml(_))
    ));
}

#[tokio::test]
async fn process_runner_preserves_each_argument_without_a_shell() {
    let temp = tempdir().expect("create test directory");
    let fake = support::fake_maa::FakeMaa::build(temp.path());
    let args = [
        OsString::from("run"),
        OsString::from(r"C:\任务 配置\日常 & 安全.json"),
        OsString::from("value|still-one-argument"),
        OsString::from(r#"quoted"value"#),
    ];

    let output = run(fake.executable(), &args, Duration::from_secs(5))
        .await
        .expect("run fake maa");

    assert_eq!(output.stdout.trim(), "fake maa");
    assert_eq!(
        fake.recorded_args(),
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn environment_inspection_proves_capabilities_with_an_absolute_dry_run() {
    let temp = tempdir().expect("create test directory");
    let config_directory = temp.path().join("maa config");
    let profiles_directory = config_directory.join("profiles");
    let user_tasks_directory = config_directory.join("tasks");
    let state_directory = temp.path().join("适配器 state");
    fs::create_dir_all(&profiles_directory).expect("create profiles directory");
    fs::create_dir_all(&user_tasks_directory).expect("create user tasks directory");
    fs::write(
        profiles_directory.join("default.toml"),
        "secret_account = 'must-not-leak'",
    )
    .expect("create default profile");
    fs::write(profiles_directory.join("夜班.json"), "{}").expect("create named profile");
    fs::write(profiles_directory.join("ignore.txt"), "not a profile")
        .expect("create unrelated file");
    fs::write(profiles_directory.join("..toml"), "{}").expect("create dot-stem profile");
    fs::write(profiles_directory.join(" .toml"), "{}").expect("create blank-stem profile");
    fs::write(user_tasks_directory.join("keep.txt"), "untouched")
        .expect("create user task sentinel");

    let fake = support::fake_maa::FakeMaa::build(temp.path());
    fake.set_config_directory(&config_directory);
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());

    let environment = adapter
        .inspect_environment(&state_directory)
        .await
        .expect("inspect fake maa environment");

    assert_eq!(environment.cli_version, "2.4.0");
    assert_eq!(environment.core_version.as_deref(), Some("v5.20.0"));
    assert_eq!(
        environment.config_dir,
        config_directory
            .canonicalize()
            .expect("canonical config path")
    );
    assert_eq!(environment.profiles, ["default", "夜班"]);
    assert!(environment.capabilities.batch);
    assert!(environment.capabilities.profile);
    assert!(environment.capabilities.dry_run);
    assert!(environment.capabilities.absolute_task_path);
    assert!(environment.capabilities.log_file);
    assert!(environment.can_execute());
    assert!(
        environment
            .profiles
            .iter()
            .all(|profile| !profile.contains("must-not-leak"))
    );

    let invocations = fake.recorded_invocations();
    assert!(invocations.contains(&vec!["version".to_owned()]));
    assert!(invocations.contains(&vec!["dir".to_owned(), "config".to_owned()]));
    assert!(invocations.contains(&vec!["run".to_owned(), "--help".to_owned()]));
    assert!(invocations.contains(&vec!["--help".to_owned()]));
    let dry_run = invocations
        .iter()
        .find(|args| args.first().is_some_and(|arg| arg == "--batch"))
        .expect("absolute task dry-run invocation");
    assert_eq!(dry_run[1], "run");
    let probe_path = Path::new(&dry_run[2]);
    assert!(probe_path.is_absolute());
    assert!(probe_path.starts_with(&state_directory));
    assert_eq!(&dry_run[3..], ["--dry-run", "--profile", "default"]);
    assert!(
        !probe_path.exists(),
        "successful probe file must be removed"
    );
    assert_eq!(
        fs::read_to_string(user_tasks_directory.join("keep.txt")).expect("read user task sentinel"),
        "untouched"
    );
    assert_eq!(
        fs::read_dir(&user_tasks_directory)
            .expect("read user tasks directory")
            .count(),
        1
    );
}

#[tokio::test]
async fn missing_help_capability_disables_execution_without_guessing() {
    let temp = tempdir().expect("create test directory");
    let (fake, state_directory) = configured_fake(&temp, "missing-dry-run");
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());

    let environment = adapter
        .inspect_environment(&state_directory)
        .await
        .expect("inspect limited fake maa environment");

    assert!(!environment.capabilities.dry_run);
    assert!(!environment.capabilities.absolute_task_path);
    assert!(!environment.can_execute());
    assert!(environment.can_plan());
    assert!(
        fake.recorded_invocations()
            .iter()
            .all(|args| args.first().is_none_or(|arg| arg != "--batch"))
    );
}

#[tokio::test]
async fn version_command_failure_is_a_hard_environment_error() {
    let temp = tempdir().expect("create test directory");
    let (fake, state_directory) = configured_fake(&temp, "version-fail");
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());

    let error = adapter
        .inspect_environment(&state_directory)
        .await
        .expect_err("version failure must reject the environment");

    assert!(matches!(error, MaaCliError::VersionCommand(_)));
    assert!(!error.to_string().contains("secret_account_token"));
}

#[tokio::test]
async fn relative_config_directory_output_is_rejected() {
    let temp = tempdir().expect("create test directory");
    let (fake, state_directory) = configured_fake(&temp, "relative-config");
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());

    let result = adapter.inspect_environment(&state_directory).await;

    assert!(matches!(
        result,
        Err(MaaCliError::InvalidConfigDirectoryOutput)
    ));
}

#[tokio::test]
async fn failed_absolute_path_probe_disables_execution_and_cleans_up() {
    let temp = tempdir().expect("create test directory");
    let (fake, state_directory) = configured_fake(&temp, "dry-run-fail");
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());

    let environment = adapter
        .inspect_environment(&state_directory)
        .await
        .expect("return inspected environment with disabled execution");

    assert!(!environment.capabilities.absolute_task_path);
    assert!(!environment.can_execute());
    assert_eq!(
        environment.diagnostics,
        ["absolute task path dry-run failed"]
    );
    assert_eq!(
        fs::read_dir(&state_directory)
            .expect("read adapter state directory")
            .count(),
        0
    );
}

#[tokio::test]
async fn adapter_state_directory_cannot_be_inside_maa_config() {
    let temp = tempdir().expect("create test directory");
    let (fake, _) = configured_fake(&temp, "success");
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());
    let unsafe_state_directory = temp.path().join("maa config/tasks");

    let result = adapter.inspect_environment(&unsafe_state_directory).await;

    assert!(matches!(
        result,
        Err(MaaCliError::StateDirectoryOverlapsMaaConfig)
    ));
    assert_eq!(
        fs::read_dir(&unsafe_state_directory)
            .expect("read user tasks directory")
            .count(),
        0
    );
}

#[tokio::test]
async fn adapter_state_directory_cannot_contain_maa_config() {
    let temp = tempdir().expect("create test directory");
    let (fake, _) = configured_fake(&temp, "success");
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());

    let result = adapter.inspect_environment(temp.path()).await;

    assert!(matches!(
        result,
        Err(MaaCliError::StateDirectoryOverlapsMaaConfig)
    ));
}

fn configured_fake(
    temp: &tempfile::TempDir,
    mode: &str,
) -> (support::fake_maa::FakeMaa, std::path::PathBuf) {
    let config_directory = temp.path().join("maa config");
    let profiles_directory = config_directory.join("profiles");
    fs::create_dir_all(&profiles_directory).expect("create profiles directory");
    fs::create_dir_all(config_directory.join("tasks")).expect("create user tasks directory");
    fs::write(
        profiles_directory.join("default.toml"),
        "account = 'secret'",
    )
    .expect("create profile");
    let fake = support::fake_maa::FakeMaa::build(temp.path());
    fake.set_config_directory(&config_directory);
    fake.set_mode(mode);
    (fake, temp.path().join("adapter state"))
}

fn joined_path(paths: &[&Path]) -> OsString {
    std::env::join_paths(paths).expect("join PATH entries")
}
