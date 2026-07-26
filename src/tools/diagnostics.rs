use std::ffi::OsString;
use std::fmt::Write as _;
use std::str::FromStr;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::{RunId, RunRecord};
use crate::runs::{LogStream, RunControlService};
use crate::storage::{FileRunRepository, RunRepository, active_run_id};

use super::{SharedRuntime, ToolResult, parse_run_id};

pub const ISSUE_URL: &str = "https://github.com/sac432/maa-mcp-adapter/issues/new";
const MAX_RUNS: usize = 3;
const MAX_LOG_LINES: usize = 12;
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectDiagnosticsInput {
    #[serde(default)]
    #[schemars(description = "可选。聚焦某个 run_id；缺省汇总最近的运行。")]
    pub run_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CollectDiagnosticsOutput {
    pub issue_url: String,
    #[schemars(description = "可直接粘贴到 GitHub Issue 的脱敏诊断报告。")]
    pub report_markdown: String,
}

pub async fn collect_diagnostics(
    runtime: &SharedRuntime,
    input: CollectDiagnosticsInput,
) -> ToolResult<CollectDiagnosticsOutput> {
    let focus = match input.run_id.as_deref() {
        Some(value) => Some(parse_run_id(value)?),
        None => None,
    };

    let mut report = String::new();
    let _ = writeln!(report, "## maa-mcp-adapter 诊断报告");
    let _ = writeln!(report);
    let _ = writeln!(report, "- 适配器版本：{}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        report,
        "- 系统：{} / {}（具体 Windows 版本请补充，例如 Win11 24H2）",
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    match runtime.config() {
        Ok(config) => {
            append_maa_versions(&mut report, &config).await;
            append_config_shape(&mut report, &config);
        }
        Err(issue) => {
            let _ = writeln!(report, "- 适配器启动问题：{}", issue.code);
        }
    }

    match runtime.state_directory() {
        Ok(state_directory) => {
            let lock = active_run_id(&state_directory).ok().flatten();
            let _ = writeln!(
                report,
                "- 活动锁：{}",
                match lock {
                    Some(run_id) => format!("存在（run {}）", short_id(&run_id)),
                    None => "无".to_owned(),
                }
            );
            let patterns = runtime
                .config()
                .map(|config| config.redaction_patterns.clone())
                .unwrap_or_default();
            append_recent_runs(&mut report, &state_directory, patterns, focus);
        }
        Err(issue) => {
            let _ = writeln!(report, "- 状态目录不可用：{}", issue.code);
        }
    }

    let _ = writeln!(report);
    let _ = writeln!(report, "### 复现步骤（请补充）");
    let _ = writeln!(report);
    let _ = writeln!(report, "1. ");
    let _ = writeln!(report);
    let _ = writeln!(report, "### 期望行为与实际行为（请补充）");
    let _ = writeln!(report);
    let _ = writeln!(
        report,
        "---\n以上内容已自动脱敏（配置的 redaction_patterns 与用户目录）。\
         提交前请再次确认不含账号、设备或其他隐私信息，然后发到：{ISSUE_URL}"
    );

    Ok(rmcp::Json(CollectDiagnosticsOutput {
        issue_url: ISSUE_URL.to_owned(),
        report_markdown: sanitize_user_paths(&report),
    }))
}

async fn append_maa_versions(report: &mut String, config: &crate::config::AdapterConfig) {
    let resolved = config.resolve_maa_executable(super::path_search_value().as_deref());
    match resolved {
        Ok(resolved) => {
            let args = [OsString::from("version")];
            match crate::maa_cli::process::run(&resolved.path, &args, VERSION_PROBE_TIMEOUT).await {
                Ok(output) => match crate::maa_cli::parse_versions(&output.stdout) {
                    Ok((cli, core)) => {
                        let _ = writeln!(
                            report,
                            "- maa-cli：v{cli}；MaaCore：{}",
                            core.unwrap_or_else(|| "未安装".to_owned())
                        );
                    }
                    Err(_) => {
                        let _ = writeln!(report, "- maa-cli：版本输出无法解析");
                    }
                },
                Err(_) => {
                    let _ = writeln!(report, "- maa-cli：版本探测失败");
                }
            }
        }
        Err(_) => {
            let _ = writeln!(report, "- maa-cli：未找到可执行文件");
        }
    }
}

fn append_config_shape(report: &mut String, config: &crate::config::AdapterConfig) {
    let _ = writeln!(report);
    let _ = writeln!(report, "### 配置形状（不含敏感值）");
    let _ = writeln!(report);
    let _ = writeln!(
        report,
        "- confirmation：{}",
        match config.confirmation {
            crate::config::ConfirmationMode::Elicitation => "elicitation",
            crate::config::ConfirmationMode::None => "none",
        }
    );
    let _ = writeln!(
        report,
        "- maa_executable：{}；maa_gui_config：{}；state_directory：{}",
        set_or_default(config.maa_executable.is_some()),
        set_or_default(config.maa_gui_config.is_some()),
        if config.state_directory.is_some() {
            "自定义"
        } else {
            "默认"
        }
    );
    let _ = writeln!(
        report,
        "- retention_days：{}；redaction_patterns：{} 项",
        config.retention_days(),
        config.redaction_patterns.len()
    );
    match &config.emulator {
        Some(emulator) => {
            let _ = writeln!(
                report,
                "- [emulator]：已配置（args：{}；adb_path：{}；stop_args：{}；startup_wait：{}s；boot_settle：{}s；stop_after_run：{}）",
                yes_no(!emulator.args.is_empty()),
                yes_no(emulator.adb_path.is_some()),
                yes_no(emulator.stop_args.is_some()),
                emulator.startup_wait_seconds,
                emulator.boot_settle_seconds,
                yes_no(emulator.stop_after_run)
            );
        }
        None => {
            let _ = writeln!(report, "- [emulator]：未配置");
        }
    }
}

fn append_recent_runs(
    report: &mut String,
    state_directory: &std::path::Path,
    patterns: Vec<String>,
    focus: Option<RunId>,
) {
    let Ok(repository) = FileRunRepository::new(state_directory.to_path_buf()) else {
        let _ = writeln!(report, "- 运行记录：仓库不可用");
        return;
    };
    let mut records: Vec<RunRecord> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(state_directory.join("runs")) {
        for entry in entries.flatten() {
            if let Some(run_id) = entry
                .file_name()
                .to_str()
                .and_then(|name| RunId::from_str(name).ok())
                && let Ok(record) = repository.load(&run_id)
            {
                records.push(record);
            }
        }
    }
    records.sort_by_key(|record| std::cmp::Reverse(record.started_at_unix_ms));
    let selected: Vec<&RunRecord> = match focus {
        Some(run_id) => records
            .iter()
            .filter(|record| record.run_id == run_id)
            .collect(),
        None => records.iter().take(MAX_RUNS).collect(),
    };

    let _ = writeln!(report);
    let _ = writeln!(report, "### 最近运行");
    if selected.is_empty() {
        let _ = writeln!(report);
        let _ = writeln!(report, "（没有可读的运行记录）");
        return;
    }
    let controls = RunControlService::new(state_directory.to_path_buf(), patterns);
    for record in selected {
        let _ = writeln!(report);
        let _ = writeln!(
            report,
            "#### 运行 {}：{}{}",
            short_id(&record.run_id),
            format!("{:?}", record.status).to_ascii_lowercase(),
            match record.exit_code {
                Some(code) => format!("，exit {code}（0x{:08X}）", code as u32),
                None => String::new(),
            }
        );
        let _ = writeln!(
            report,
            "- 任务：{}；时长：{}；停止方式：{}",
            task_types(record),
            match record.finished_at_unix_ms {
                Some(finished) => format!(
                    "{} 秒",
                    finished.saturating_sub(record.started_at_unix_ms) / 1000
                ),
                None => "进行中/未知".to_owned(),
            },
            record
                .stop_method
                .map(|method| format!("{method:?}").to_ascii_lowercase())
                .unwrap_or_else(|| "无".to_owned())
        );
        match controls.get_log(&record.run_id, LogStream::Stderr) {
            Ok(tail) => {
                let lines: Vec<&str> = tail
                    .lines
                    .iter()
                    .rev()
                    .take(MAX_LOG_LINES)
                    .map(String::as_str)
                    .collect();
                let _ = writeln!(report, "- stderr 尾部（已脱敏）：");
                let _ = writeln!(report);
                let _ = writeln!(report, "```text");
                for line in lines.iter().rev() {
                    let _ = writeln!(report, "{line}");
                }
                let _ = writeln!(report, "```");
            }
            Err(_) => {
                let _ = writeln!(report, "- stderr 尾部：不可读");
            }
        }
    }
}

fn task_types(record: &RunRecord) -> String {
    let Ok(contents) = std::fs::read_to_string(&record.task_path) else {
        return "未知".to_owned();
    };
    let Ok(value) = toml::Value::from_str(&contents) else {
        return "未知".to_owned();
    };
    let Some(tasks) = value.get("tasks").and_then(toml::Value::as_array) else {
        return "未知".to_owned();
    };
    let names: Vec<&str> = tasks
        .iter()
        .filter_map(|task| task.get("type").and_then(toml::Value::as_str))
        .collect();
    if names.is_empty() {
        "未知".to_owned()
    } else {
        names.join("→")
    }
}

fn short_id(run_id: &RunId) -> String {
    let full = run_id.to_string();
    full.chars().take(8).collect()
}

fn set_or_default(set: bool) -> &'static str {
    if set { "已配置" } else { "默认/未配置" }
}

fn yes_no(value: bool) -> &'static str {
    if value { "是" } else { "否" }
}

/// 把报告中出现的用户主目录替换为 %USERPROFILE%，避免公开 Issue 泄漏
/// Windows 用户名。大小写不敏感，兼容正反斜杠写法。
fn sanitize_user_paths(text: &str) -> String {
    let mut result = text.to_owned();
    if let Some(profile) = std::env::var_os("USERPROFILE").and_then(|value| {
        let value = value.to_string_lossy().into_owned();
        if value.len() > 3 { Some(value) } else { None }
    }) {
        for variant in [profile.clone(), profile.replace('\\', "/")] {
            result = replace_ignore_ascii_case(&result, &variant, "%USERPROFILE%");
        }
    }
    result
}

fn replace_ignore_ascii_case(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return haystack.to_owned();
    }
    let lower_haystack = haystack.to_ascii_lowercase();
    let lower_needle = needle.to_ascii_lowercase();
    let mut result = String::with_capacity(haystack.len());
    let mut position = 0;
    while let Some(found) = lower_haystack[position..].find(&lower_needle) {
        let start = position + found;
        result.push_str(&haystack[position..start]);
        result.push_str(replacement);
        position = start + needle.len();
    }
    result.push_str(&haystack[position..]);
    result
}

#[cfg(test)]
mod tests {
    use super::{replace_ignore_ascii_case, sanitize_user_paths};

    #[test]
    fn user_profile_paths_are_masked_case_insensitively() {
        let Some(profile) = std::env::var("USERPROFILE").ok() else {
            return;
        };
        let text = format!(
            "log at {}\\AppData\\x and {} and {}",
            profile,
            profile.to_ascii_uppercase(),
            profile.replace('\\', "/")
        );

        let masked = sanitize_user_paths(&text);

        assert!(
            !masked
                .to_ascii_lowercase()
                .contains(&profile.to_ascii_lowercase())
        );
        assert!(masked.contains("%USERPROFILE%\\AppData\\x"));
    }

    #[test]
    fn case_insensitive_replace_covers_every_occurrence() {
        assert_eq!(
            replace_ignore_ascii_case("AbcABCabc", "abc", "-"),
            "---".to_owned()
        );
        assert_eq!(replace_ignore_ascii_case("xyz", "abc", "-"), "xyz");
    }
}
