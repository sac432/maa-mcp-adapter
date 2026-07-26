use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rmcp::service::ElicitationError;
use rmcp::{Peer, RoleServer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{AdapterConfig, ConfirmationMode};
use crate::storage::resolve_state_directory;
use crate::tools::environment::discover_gui_settings;

use super::{SharedRuntime, ToolError, ToolResult, path_search_value};

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupEnvironmentInput {
    #[serde(default)]
    #[schemars(
        description = "MAA 图形版 gui.json 的绝对路径；缺省使用 config.toml 的 maa_gui_config。"
    )]
    pub maa_gui_config: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "要创建的 maa-cli 配置档名，默认 mcp-daily；仅允许字母数字、- 和 _。"
    )]
    pub profile_name: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupEnvironmentOutput {
    pub outcome: String,
    pub profile_name: String,
    pub profile_written: bool,
    pub profile_note: String,
    pub adapter_config_updated: bool,
    pub emulator_configured: bool,
    pub notes: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetupConfirmation {
    #[schemars(description = "同意按上述内容写入时选择 true，否则选择 false")]
    confirmed: bool,
}

rmcp::elicit_safe!(SetupConfirmation);

pub async fn setup_environment(
    runtime: &SharedRuntime,
    input: SetupEnvironmentInput,
    peer: Peer<RoleServer>,
) -> ToolResult<SetupEnvironmentOutput> {
    let config = runtime
        .config()
        .map_err(ToolError::from_runtime)
        .map_err(rmcp::Json)?;

    let profile_name = input.profile_name.unwrap_or_else(|| "mcp-daily".to_owned());
    if !is_safe_profile_name(&profile_name) {
        return Err(rmcp::Json(ToolError::new(
            "INVALID_PROFILE_NAME",
            "配置档名只允许字母、数字、- 和 _。",
            Vec::new(),
            vec!["换一个不含路径分隔符的名称。".to_owned()],
        )));
    }

    let gui_path = input
        .maa_gui_config
        .map(PathBuf::from)
        .or_else(|| config.maa_gui_config.clone())
        .ok_or_else(|| {
            rmcp::Json(ToolError::new(
                "GUI_CONFIG_REQUIRED",
                "需要 MAA 图形版 gui.json 的路径才能发现启动信息。",
                Vec::new(),
                vec![
                    "传入 maa_gui_config 参数，或先在 config.toml 配置 maa_gui_config。".to_owned(),
                ],
            ))
        })?;
    if !gui_path.is_absolute() || !gui_path.is_file() {
        return Err(rmcp::Json(ToolError::new(
            "GUI_CONFIG_INVALID",
            "maa_gui_config 必须指向存在的 gui.json 绝对路径。",
            Vec::new(),
            vec!["检查 MAA 图形版安装目录下的 config\\gui.json。".to_owned()],
        )));
    }

    let discovery = discover_gui_settings(&gui_path).map_err(|message| {
        rmcp::Json(ToolError::new(
            "GUI_CONFIG_UNREADABLE",
            message,
            Vec::new(),
            vec!["确认该文件是 MAA 图形版的 gui.json 后重试。".to_owned()],
        ))
    })?;
    let Some(address) = discovery.address.clone() else {
        return Err(rmcp::Json(ToolError::new(
            "SETUP_DISCOVERY_INCOMPLETE",
            "MAA 配置里没有已保存的连接地址。",
            Vec::new(),
            vec!["先在 MAA 图形版完成一次成功连接（保存 adb 地址）后重试。".to_owned()],
        )));
    };

    let resolved = config
        .resolve_maa_executable(path_search_value().as_deref())
        .map_err(|_| {
            rmcp::Json(ToolError::new(
                "MAA_CLI_UNAVAILABLE",
                "未找到 maa-cli，可执行文件是初始化的前置条件。",
                Vec::new(),
                vec![
                    "安装 maa-cli 并确保在 PATH 中，或在 config.toml 配置 maa_executable。"
                        .to_owned(),
                ],
            ))
        })?;
    let profiles_directory = maa_profiles_directory(&resolved.path).await.map_err(|_| {
        rmcp::Json(ToolError::new(
            "MAA_CONFIG_DIR_UNAVAILABLE",
            "无法从 maa-cli 获取配置目录。",
            Vec::new(),
            vec!["在终端运行 maa dir config 检查 maa-cli 是否可用。".to_owned()],
        ))
    })?;
    let profile_path = profiles_directory.join(format!("{profile_name}.toml"));
    let profile_exists = profile_path.exists();
    let profile_content = render_profile(&discovery.adb_path, &address);

    let adapter_config_path = adapter_config_path().map_err(rmcp::Json)?;
    let current_config_text = std::fs::read_to_string(&adapter_config_path).unwrap_or_default();
    let disk_config = AdapterConfig::from_toml(&current_config_text).map_err(|_| {
        rmcp::Json(ToolError::new(
            "CONFIG_INVALID",
            "磁盘上的 config.toml 当前无法解析，拒绝在其上追加。",
            Vec::new(),
            vec!["先修复 config.toml 后重试。".to_owned()],
        ))
    })?;
    let emulator = if disk_config.emulator.is_none() {
        derive_emulator_section(discovery.adb_path.as_deref(), &address)
    } else {
        None
    };
    let config_additions = render_config_additions(
        disk_config
            .maa_gui_config
            .is_none()
            .then_some(gui_path.as_path()),
        emulator.as_deref(),
    );

    let mut plan = String::new();
    let _ = writeln!(plan, "将进行以下写入：");
    if profile_exists {
        let _ = writeln!(
            plan,
            "1. maa-cli 配置档 {profile_name}.toml 已存在，跳过（绝不覆盖）。"
        );
    } else {
        let _ = writeln!(plan, "1. 新建 maa-cli 配置档 {profile_name}.toml：");
        let _ = writeln!(plan, "{profile_content}");
    }
    match &config_additions {
        Some(additions) => {
            let _ = writeln!(plan, "2. 在适配器 config.toml 追加（原文件先备份）：");
            let _ = writeln!(plan, "{additions}");
        }
        None => {
            let _ = writeln!(plan, "2. 适配器 config.toml 无需变更。");
        }
    }
    let _ = write!(plan, "\n是否确认按以上内容写入？");

    if config.confirmation == ConfirmationMode::Elicitation {
        let confirmation = match peer.elicit::<SetupConfirmation>(plan.clone()).await {
            Ok(Some(confirmation)) => confirmation,
            Ok(None) | Err(ElicitationError::UserCancelled) => {
                return Ok(rmcp::Json(not_applied("cancelled", &profile_name)));
            }
            Err(ElicitationError::UserDeclined) => {
                return Ok(rmcp::Json(not_applied("declined", &profile_name)));
            }
            Err(ElicitationError::CapabilityNotSupported) => {
                return Err(rmcp::Json(ToolError::new(
                    "CLIENT_ELICITATION_UNSUPPORTED",
                    "当前 MCP 客户端未声明表单 elicitation 能力。",
                    Vec::new(),
                    vec!["使用支持表单的客户端执行初始化。".to_owned()],
                )));
            }
            Err(_) => {
                return Err(rmcp::Json(ToolError::new(
                    "ELICITATION_FAILED",
                    "无法取得可靠的初始化确认。",
                    Vec::new(),
                    vec!["重新发起 setup_environment。".to_owned()],
                )));
            }
        };
        if !confirmation.confirmed {
            return Ok(rmcp::Json(not_applied("declined", &profile_name)));
        }
    }

    let mut notes = Vec::new();
    let profile_written = if profile_exists {
        false
    } else {
        write_new_file(&profile_path, &profile_content).map_err(|error| {
            rmcp::Json(ToolError::new(
                "SETUP_WRITE_FAILED",
                "写入 maa-cli 配置档失败。",
                vec![error],
                vec!["检查 maa-cli 配置目录的写权限后重试。".to_owned()],
            ))
        })?
    };
    if discovery.adb_path.is_none() {
        notes.push(
            "MAA 配置中没有 adb 路径，配置档未包含 adb_path，将使用 maa-cli 默认。".to_owned(),
        );
    }

    let adapter_config_updated = match &config_additions {
        Some(additions) => {
            apply_config_additions(&adapter_config_path, &current_config_text, additions).map_err(
                |error| {
                    rmcp::Json(ToolError::new(
                        "SETUP_WRITE_FAILED",
                        "更新适配器 config.toml 失败，原文件未受影响。",
                        vec![error],
                        vec!["检查状态目录写权限后重试。".to_owned()],
                    ))
                },
            )?;
            true
        }
        None => false,
    };
    if emulator.is_none() && disk_config.emulator.is_none() {
        notes.push(
            "未能从 adb 路径定位受支持的模拟器启动器，[emulator] 未写入；可参照文档手动配置。"
                .to_owned(),
        );
    }

    Ok(rmcp::Json(SetupEnvironmentOutput {
        outcome: "applied".to_owned(),
        profile_name,
        profile_written,
        profile_note: if profile_exists {
            "已存在，未改动".to_owned()
        } else {
            "已新建".to_owned()
        },
        adapter_config_updated,
        emulator_configured: emulator.is_some(),
        notes,
        next_steps: vec![
            "重启 MCP 客户端中的 maa Server 以加载新配置。".to_owned(),
            "调用 check_environment，确认 can_execute = true。".to_owned(),
        ],
    }))
}

fn not_applied(outcome: &str, profile_name: &str) -> SetupEnvironmentOutput {
    SetupEnvironmentOutput {
        outcome: outcome.to_owned(),
        profile_name: profile_name.to_owned(),
        profile_written: false,
        profile_note: "未写入".to_owned(),
        adapter_config_updated: false,
        emulator_configured: false,
        notes: Vec::new(),
        next_steps: Vec::new(),
    }
}

fn is_safe_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

async fn maa_profiles_directory(executable: &Path) -> Result<PathBuf, ()> {
    let args = [OsString::from("dir"), OsString::from("config")];
    let output = crate::maa_cli::process::run(executable, &args, PROBE_TIMEOUT)
        .await
        .map_err(|_| ())?;
    let line = output.stdout.trim();
    if line.is_empty() {
        return Err(());
    }
    let directory = PathBuf::from(line);
    if !directory.is_absolute() || !directory.is_dir() {
        return Err(());
    }
    let profiles = directory.join("profiles");
    std::fs::create_dir_all(&profiles).map_err(|_| ())?;
    Ok(profiles)
}

fn render_profile(adb_path: &Option<String>, address: &str) -> String {
    let mut content = String::from("[connection]\npreset = \"ADB\"\n");
    if let Some(adb) = adb_path {
        let _ = writeln!(content, "adb_path = '{adb}'");
    }
    let _ = writeln!(content, "address = \"{address}\"");
    content
}

/// 从 adb 路径反推安装根目录并定位受支持的启动器；找不到则返回 None。
fn derive_emulator_section(adb_path: Option<&str>, address: &str) -> Option<String> {
    let adb = PathBuf::from(adb_path?);
    let root = adb.parent()?.parent()?;
    let candidates: [(&str, &[&str], &[&str]); 3] = [
        (
            "nx_main/MuMuManager.exe",
            &["control", "-v", "0", "launch"],
            &["control", "-v", "0", "shutdown"],
        ),
        (
            "ldconsole.exe",
            &["launch", "--index", "0"],
            &["quit", "--index", "0"],
        ),
        (
            "dnconsole.exe",
            &["launch", "--index", "0"],
            &["quit", "--index", "0"],
        ),
    ];
    for (relative, launch, stop) in candidates {
        let path = root.join(relative);
        if path.is_file() {
            let mut section = String::from("[emulator]\n");
            let _ = writeln!(section, "path = '{}'", path.display());
            let _ = writeln!(section, "args = {}", toml_string_array(launch));
            let _ = writeln!(section, "address = \"{address}\"");
            let _ = writeln!(section, "adb_path = '{}'", adb.display());
            let _ = writeln!(section, "startup_wait_seconds = 180");
            let _ = writeln!(section, "boot_settle_seconds = 15");
            let _ = writeln!(section, "stop_after_run = false");
            let _ = writeln!(section, "stop_args = {}", toml_string_array(stop));
            return Some(section);
        }
    }
    None
}

fn toml_string_array(values: &[&str]) -> String {
    let quoted: Vec<String> = values.iter().map(|value| format!("\"{value}\"")).collect();
    format!("[{}]", quoted.join(", "))
}

fn render_config_additions(
    gui_path_to_add: Option<&Path>,
    emulator_section: Option<&str>,
) -> Option<String> {
    if gui_path_to_add.is_none() && emulator_section.is_none() {
        return None;
    }
    let mut additions = String::new();
    if let Some(path) = gui_path_to_add {
        let _ = writeln!(additions, "maa_gui_config = '{}'", path.display());
    }
    if let Some(section) = emulator_section {
        if !additions.is_empty() {
            let _ = writeln!(additions);
        }
        additions.push_str(section);
    }
    Some(additions)
}

fn adapter_config_path() -> Result<PathBuf, ToolError> {
    let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        ToolError::new(
            "LOCAL_APP_DATA_UNAVAILABLE",
            "无法确定 Windows 应用数据目录。",
            Vec::new(),
            vec!["设置 LOCALAPPDATA 后重试。".to_owned()],
        )
    })?;
    let default_state = resolve_state_directory(None, Some(&local_app_data)).map_err(|_| {
        ToolError::new(
            "STATE_DIRECTORY_INVALID",
            "适配器默认状态目录无效。",
            Vec::new(),
            vec!["检查 LOCALAPPDATA 指向后重试。".to_owned()],
        )
    })?;
    std::fs::create_dir_all(&default_state).map_err(|_| {
        ToolError::new(
            "STORAGE_UNAVAILABLE",
            "无法创建适配器状态目录。",
            Vec::new(),
            vec!["检查目录权限后重试。".to_owned()],
        )
    })?;
    Ok(default_state.join("config.toml"))
}

fn write_new_file(path: &Path, content: &str) -> Result<bool, String> {
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())?;
    Ok(true)
}

/// 顶层键必须写在任何表之前：追加内容整体放在原文件之前拼接，
/// 先验证合并结果可被 AdapterConfig 解析，再备份原文件并原子替换。
fn apply_config_additions(
    config_path: &Path,
    current_text: &str,
    additions: &str,
) -> Result<(), String> {
    let merged = merge_config_text(current_text, additions);
    AdapterConfig::from_toml(&merged).map_err(|error| format!("合并后的配置无法解析：{error}"))?;
    let mode = if config_path.exists() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let backup = config_path.with_extension(format!("toml.bak-{stamp}"));
        std::fs::copy(config_path, &backup).map_err(|error| error.to_string())?;
        crate::storage::atomic::WriteMode::Replace
    } else {
        crate::storage::atomic::WriteMode::Create
    };
    crate::storage::atomic::write_atomically(config_path, merged.as_bytes(), mode)
        .map_err(|error| error.to_string())
}

/// 追加块的顶层键（如 maa_gui_config）放在原内容前，表段放在原内容后，
/// 避免顶层键落入原文件末尾的表内。
fn merge_config_text(current_text: &str, additions: &str) -> String {
    let (top_level, tables) = match additions.find("[") {
        Some(index) if additions[..index].trim().is_empty() => ("", additions),
        Some(index) => additions.split_at(index),
        None => (additions, ""),
    };
    let mut merged = String::new();
    if !top_level.trim().is_empty() {
        merged.push_str(top_level.trim_end());
        merged.push('\n');
    }
    merged.push_str(current_text.trim_end());
    if !merged.is_empty() {
        merged.push('\n');
    }
    if !tables.trim().is_empty() {
        merged.push('\n');
        merged.push_str(tables.trim_end());
        merged.push('\n');
    }
    merged
}

#[cfg(test)]
mod tests {
    use crate::config::AdapterConfig;

    use super::{derive_emulator_section, is_safe_profile_name, merge_config_text, render_profile};

    #[test]
    fn profile_names_reject_path_tricks() {
        assert!(is_safe_profile_name("mcp-daily"));
        assert!(is_safe_profile_name("user_2"));
        assert!(!is_safe_profile_name(""));
        assert!(!is_safe_profile_name("../evil"));
        assert!(!is_safe_profile_name("a b"));
        assert!(!is_safe_profile_name("名"));
    }

    #[test]
    fn rendered_profile_contains_connection_essentials() {
        let content = render_profile(
            &Some("D:\\MuMu\\shell\\adb.exe".to_owned()),
            "127.0.0.1:16384",
        );
        assert!(content.contains("preset = \"ADB\""));
        assert!(content.contains("adb_path = 'D:\\MuMu\\shell\\adb.exe'"));
        assert!(content.contains("address = \"127.0.0.1:16384\""));
    }

    #[test]
    fn emulator_derivation_finds_mumu_manager_from_adb_path() {
        let temp = tempfile::tempdir().expect("temporary root");
        let root = temp.path();
        std::fs::create_dir_all(root.join("shell")).expect("shell dir");
        std::fs::create_dir_all(root.join("nx_main")).expect("nx_main dir");
        let adb = root.join("shell").join("adb.exe");
        std::fs::write(&adb, "fake").expect("fake adb");
        std::fs::write(root.join("nx_main").join("MuMuManager.exe"), "fake").expect("fake manager");

        let section = derive_emulator_section(Some(&adb.display().to_string()), "127.0.0.1:16384")
            .expect("derived emulator section");

        assert!(section.contains("MuMuManager.exe"));
        assert!(section.contains("args = [\"control\", \"-v\", \"0\", \"launch\"]"));
        assert!(section.contains("stop_args = [\"control\", \"-v\", \"0\", \"shutdown\"]"));
        assert!(section.contains("stop_after_run = false"));

        assert!(derive_emulator_section(Some("Z:/nowhere/shell/adb.exe"), "127.0.0.1:1").is_none());
    }

    #[test]
    fn merged_config_keeps_top_level_keys_before_tables_and_parses() {
        let current = "maa_executable = 'C:/maa/maa.exe'\nretention_days = 7\n";
        let additions =
            "maa_gui_config = 'C:/MAA/config/gui.json'\n\n[emulator]\npath = 'C:/emu/m.exe'\n";

        let merged = merge_config_text(current, additions);

        let parsed = AdapterConfig::from_toml(&merged).expect("merged config parses");
        assert!(parsed.maa_gui_config.is_some());
        assert!(parsed.emulator.is_some());
        assert!(parsed.maa_executable.is_some());
        assert!(
            merged.find("maa_gui_config").expect("gui key")
                < merged.find("[emulator]").expect("table")
        );
    }
}
