use std::path::Path;

use serde_json::Value;

use crate::domain::{AwardPlan, DailyTask, InfrastPlan, MallPlan, RecruitPlan, StartUpPlan};

/// 从 MAA 图形版保存的任务队列（`gui.new.json` 的
/// `Configurations.<Current>.TaskQueue`）确定性地派生本适配器的默认
/// 日常计划。只读；只映射白名单内的任务；数值一律向安全方向钳制。
///
/// 刻意不派生 `Fight`：MAA 的刷理智是"刷到资源耗尽"模型，与本项目
/// "有限显式次数"的安全模型不兼容，刷图必须由用户显式给出关卡与次数。
/// `use_expedited_plans` 恒为 `false`（消耗品的启用必须来自用户显式请求）。
pub fn derive_default_tasks(gui_config_path: &Path) -> Option<Vec<DailyTask>> {
    let queue_path = task_queue_path(gui_config_path)?;
    let contents = std::fs::read_to_string(queue_path).ok()?;
    let value: Value = serde_json::from_str(&contents).ok()?;
    let current = value
        .get("Current")
        .and_then(Value::as_str)
        .unwrap_or("Default");
    let queue = value
        .get("Configurations")?
        .get(current)?
        .get("TaskQueue")?
        .as_array()?;

    let mut tasks = Vec::new();
    for entry in queue {
        if entry.get("IsEnable").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        match entry.get("TaskType").and_then(Value::as_str) {
            Some("StartUp") => tasks.push(DailyTask::StartUp(StartUpPlan {
                start_game: Some(true),
            })),
            Some("Recruit") => {
                let max_times = entry
                    .get("MaxTimes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(4) as u8;
                let confirm = ["Level3Choose", "Level4Choose", "Level5Choose"]
                    .iter()
                    .any(|key| entry.get(*key).and_then(Value::as_bool) == Some(true));
                tasks.push(DailyTask::Recruit(RecruitPlan {
                    max_recruits: Some(max_times),
                    confirm_recruitment: Some(confirm),
                    use_expedited_plans: Some(false),
                }));
            }
            Some("Infrast") => tasks.push(DailyTask::Infrast(InfrastPlan {})),
            Some("Mall") => tasks.push(DailyTask::Mall(MallPlan {
                visit_friends: Some(
                    entry.get("VisitFriends").and_then(Value::as_bool) == Some(true),
                ),
                shopping: Some(entry.get("Shopping").and_then(Value::as_bool) == Some(true)),
            })),
            Some("Award") => {
                let award = entry.get("Award").and_then(Value::as_bool) == Some(true);
                let event = ["FreeGacha", "Orundum", "Mining", "SpecialAccess"]
                    .iter()
                    .any(|key| entry.get(*key).and_then(Value::as_bool) == Some(true));
                tasks.push(DailyTask::Award(AwardPlan {
                    daily: Some(award),
                    weekly: Some(award),
                    mail: Some(entry.get("Mail").and_then(Value::as_bool) == Some(true)),
                    event: Some(event),
                }));
            }
            _ => {}
        }
    }

    if tasks.is_empty() { None } else { Some(tasks) }
}

/// `maa_gui_config` 指向 `gui.json`；任务队列存放在同目录的
/// `gui.new.json`（新版 MAA）。若配置直接指向含 TaskQueue 的文件也接受。
fn task_queue_path(gui_config_path: &Path) -> Option<std::path::PathBuf> {
    let sibling = gui_config_path.parent()?.join("gui.new.json");
    if sibling.is_file() {
        return Some(sibling);
    }
    if gui_config_path.is_file() {
        return Some(gui_config_path.to_path_buf());
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::domain::DailyTask;

    use super::derive_default_tasks;

    const FIXTURE: &str = r#"{
        "Configurations": {
            "Default": {
                "TaskQueue": [
                    {"TaskType": "StartUp", "IsEnable": true},
                    {"TaskType": "Recruit", "IsEnable": true, "MaxTimes": 100,
                     "Level3Choose": true, "Level4Choose": true, "Level5Choose": true},
                    {"TaskType": "Infrast", "IsEnable": true},
                    {"TaskType": "Fight", "IsEnable": true, "TimesLimit": 5},
                    {"TaskType": "Mall", "IsEnable": true, "Shopping": true, "VisitFriends": true},
                    {"TaskType": "Award", "IsEnable": true, "Award": true, "Mail": false,
                     "FreeGacha": false, "Orundum": false, "Mining": false, "SpecialAccess": false},
                    {"TaskType": "Roguelike", "IsEnable": true},
                    {"TaskType": "Reclamation", "IsEnable": false}
                ]
            }
        },
        "Current": "Default"
    }"#;

    #[test]
    fn enabled_whitelisted_tasks_map_with_safe_clamps() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let gui = temp.path().join("gui.json");
        std::fs::write(temp.path().join("gui.new.json"), FIXTURE).expect("write fixture");
        std::fs::write(&gui, "{}").expect("write gui placeholder");

        let tasks = derive_default_tasks(&gui).expect("derived tasks");

        let kinds: Vec<&str> = tasks
            .iter()
            .map(|task| match task {
                DailyTask::StartUp(_) => "start_up",
                DailyTask::Fight(_) => "fight",
                DailyTask::Recruit(_) => "recruit",
                DailyTask::Infrast(_) => "infrast",
                DailyTask::Mall(_) => "mall",
                DailyTask::Award(_) => "award",
                DailyTask::CloseDown(_) => "close_down",
            })
            .collect();
        assert_eq!(kinds, ["start_up", "recruit", "infrast", "mall", "award"]);

        let DailyTask::Recruit(recruit) = &tasks[1] else {
            panic!("expected recruit");
        };
        assert_eq!(recruit.max_recruits, Some(4), "MaxTimes 100 clamps to 4");
        assert_eq!(recruit.confirm_recruitment, Some(true));
        assert_eq!(
            recruit.use_expedited_plans,
            Some(false),
            "expedited plans stay off no matter what"
        );

        let DailyTask::Award(award) = &tasks[4] else {
            panic!("expected award");
        };
        assert_eq!(award.daily, Some(true));
        assert_eq!(award.weekly, Some(true));
        assert_eq!(award.mail, Some(false));
        assert_eq!(award.event, Some(false));
    }

    #[test]
    fn disabled_queue_or_missing_file_yields_none() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let gui = temp.path().join("gui.json");
        assert!(derive_default_tasks(&gui).is_none());

        std::fs::write(
            temp.path().join("gui.new.json"),
            r#"{"Configurations":{"Default":{"TaskQueue":[
                {"TaskType":"StartUp","IsEnable":false}]}},"Current":"Default"}"#,
        )
        .expect("write disabled fixture");
        assert!(derive_default_tasks(&gui).is_none());
    }
}
