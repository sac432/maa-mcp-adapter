use std::ffi::OsStr;
use std::path::Path;

use maa_mcp_adapter::domain::{
    AwardPlan, CloseDownPlan, DailyPlan, DailyTask, FightCategory, FightPlan, InfrastPlan,
    MallPlan, RecruitPlan, StartUpPlan,
};
use maa_mcp_adapter::maa_cli::MaaCliAdapter;
use maa_mcp_adapter::maa_cli::task_file::{
    RunId, TaskFileError, TaskFilePolicy, TaskFileStore, render_task_file,
    render_task_file_with_policy,
};
use tempfile::tempdir;

mod support;

#[test]
fn every_whitelisted_task_matches_its_golden_toml() {
    let cases = [
        (
            DailyTask::StartUp(StartUpPlan {
                start_game: Some(true),
            }),
            include_str!("golden/start_up.toml"),
        ),
        (
            DailyTask::Fight(FightPlan {
                category: Some(FightCategory::Event),
                stage: Some("XX-8".to_owned()),
                times: Some(5),
                medicine: Some(2),
            }),
            include_str!("golden/fight.toml"),
        ),
        (
            DailyTask::Recruit(RecruitPlan {
                max_recruits: Some(4),
                confirm_recruitment: Some(true),
                use_expedited_plans: Some(false),
            }),
            include_str!("golden/recruit.toml"),
        ),
        (
            DailyTask::Infrast(InfrastPlan {}),
            include_str!("golden/infrast.toml"),
        ),
        (
            DailyTask::Mall(MallPlan {
                visit_friends: Some(true),
                shopping: Some(true),
            }),
            include_str!("golden/mall.toml"),
        ),
        (
            DailyTask::Award(AwardPlan {
                daily: Some(true),
                weekly: Some(true),
                mail: Some(true),
                event: Some(true),
            }),
            include_str!("golden/award.toml"),
        ),
        (
            DailyTask::CloseDown(CloseDownPlan {}),
            include_str!("golden/close_down.toml"),
        ),
    ];

    for (task, expected) in cases {
        let actual = render_task_file(&DailyPlan::new("default", vec![task]))
            .expect("render safe task file");
        assert_eq!(
            toml::from_str::<toml::Value>(&actual).expect("parse generated TOML"),
            toml::from_str::<toml::Value>(expected).expect("parse golden TOML")
        );
    }
}

#[test]
fn generated_fight_and_mall_safety_fields_are_non_overridable() {
    let plan = DailyPlan::new(
        "default",
        vec![
            DailyTask::Fight(FightPlan {
                category: Some(FightCategory::Main),
                stage: Some("1-7".to_owned()),
                times: Some(100),
                medicine: Some(10),
            }),
            DailyTask::Mall(MallPlan {
                visit_friends: Some(false),
                shopping: Some(true),
            }),
        ],
    );

    let rendered = render_task_file(&plan).expect("render safe task file");

    assert!(rendered.contains("stone = 0"));
    assert!(rendered.contains("force_shopping_if_credit_full = false"));
    assert!(!rendered.contains("serde_json"));
}

#[test]
fn configured_mall_blacklist_is_preserved_without_weakening_fixed_protection() {
    let plan = DailyPlan::new(
        "default",
        vec![DailyTask::Mall(MallPlan {
            visit_friends: Some(true),
            shopping: Some(true),
        })],
    );
    let policy = TaskFilePolicy::new(vec!["家具零件".to_owned()]);
    let rendered =
        render_task_file_with_policy(&plan, &policy).expect("render configured blacklist");

    assert!(rendered.contains("\"家具零件\""));
    assert!(rendered.contains("\"加急许可\""));
    assert!(rendered.contains("force_shopping_if_credit_full = false"));
}

#[test]
fn generated_tasks_have_canonical_order_and_run_ids_cannot_overwrite() {
    let mut plan = safe_fight_plan();
    plan.tasks.insert(0, DailyTask::CloseDown(CloseDownPlan {}));
    plan.tasks.push(DailyTask::StartUp(StartUpPlan {
        start_game: Some(true),
    }));
    let rendered = render_task_file(&plan).expect("render canonically ordered task file");
    let value: toml::Value = toml::from_str(&rendered).expect("parse generated task file");
    let task_types = value["tasks"]
        .as_array()
        .expect("task array")
        .iter()
        .map(|task| task["type"].as_str().expect("task type"))
        .collect::<Vec<_>>();
    assert_eq!(task_types, ["StartUp", "Fight", "CloseDown"]);

    let temp = tempdir().expect("create test directory");
    let store = TaskFileStore::new(temp.path().join("state")).expect("create task file store");
    let run_id = RunId::new();
    let first = store
        .generate(run_id, &plan)
        .expect("generate first task file");
    let original = std::fs::read(first.path()).expect("read first task file");
    assert!(matches!(
        store.generate(run_id, &safe_fight_plan()),
        Err(TaskFileError::RunAlreadyExists)
    ));
    assert_eq!(
        std::fs::read(first.path()).expect("read protected task file"),
        original
    );
}

#[tokio::test]
async fn generated_task_uses_a_contained_absolute_path_and_argument_array() {
    let temp = tempdir().expect("create test directory");
    let state_directory = temp
        .path()
        .join("中文 状态目录")
        .join("long-segment-012345678901234567890123456789");
    let store = TaskFileStore::new(state_directory.clone()).expect("create task file store");
    let artifact = store
        .generate(RunId::new(), &safe_fight_plan())
        .expect("generate run task file");
    let canonical_state = state_directory
        .canonicalize()
        .expect("canonicalize state directory");

    assert!(artifact.path().is_absolute());
    assert!(artifact.path().starts_with(&canonical_state));
    assert_eq!(artifact.path().file_name(), Some(OsStr::new("task.toml")));

    let fake = support::fake_maa::FakeMaa::build(temp.path());
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());
    adapter
        .dry_run_generated_task(
            &artifact,
            "default",
            Some("emulator-5554 & still-one-argument"),
        )
        .await
        .expect("dry-run generated task");

    let args = fake.recorded_args();
    assert_eq!(args[0..2], ["--batch", "run"]);
    assert_eq!(Path::new(&args[2]), artifact.path());
    assert_eq!(&args[3..6], ["--dry-run", "--profile", "default"]);
    assert_eq!(&args[6..], ["--addr", "emulator-5554 & still-one-argument"]);
}

#[tokio::test]
async fn dry_run_rejects_a_task_file_changed_by_the_checked_process() {
    let temp = tempdir().expect("create test directory");
    let store = TaskFileStore::new(temp.path().join("state")).expect("create task file store");
    let artifact = store
        .generate(RunId::new(), &safe_fight_plan())
        .expect("generate run task file");
    let fake = support::fake_maa::FakeMaa::build(temp.path());
    fake.set_mode("tamper-task");
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());

    let result = adapter
        .dry_run_generated_task(&artifact, "default", None)
        .await;

    assert!(matches!(result, Err(TaskFileError::IntegrityMismatch)));
}

#[tokio::test]
async fn generated_artifact_rejects_a_different_profile_before_spawning() {
    let temp = tempdir().expect("create test directory");
    let store = TaskFileStore::new(temp.path().join("state")).expect("create task file store");
    let artifact = store
        .generate(RunId::new(), &safe_fight_plan())
        .expect("generate run task file");
    let fake = support::fake_maa::FakeMaa::build(temp.path());
    let adapter = MaaCliAdapter::new(fake.executable().to_path_buf());

    let result = adapter
        .dry_run_generated_task(&artifact, "other-profile", None)
        .await;

    assert!(matches!(result, Err(TaskFileError::ProfileMismatch)));
    assert!(!fake.executable().with_extension("args").exists());
}

fn safe_fight_plan() -> DailyPlan {
    DailyPlan::new(
        "default",
        vec![DailyTask::Fight(FightPlan {
            category: Some(FightCategory::Event),
            stage: Some("XX-8".to_owned()),
            times: Some(5),
            medicine: Some(0),
        })],
    )
}
