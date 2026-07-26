use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use maa_mcp_adapter::domain::{
    AwardPlan, CloseDownPlan, DailyPlan, DailyTask, FightCategory, FightPlan, InfrastPlan,
    MallPlan, PlanState, RecruitPlan, StartUpPlan,
};
use maa_mcp_adapter::planning::hash::hash_plan;
use maa_mcp_adapter::planning::lifecycle::revise_plan;
use maa_mcp_adapter::planning::normalize::normalize_plan;
use maa_mcp_adapter::planning::policy::{validate_plan, validate_transition};
use maa_mcp_adapter::planning::service::{PlanPatch, PlanService};
use maa_mcp_adapter::planning::summary::summarize_plan;
use maa_mcp_adapter::planning::{MissingField, TaskKind, ValidationError};
use maa_mcp_adapter::storage::{
    FilePlanRepository, PlanRepository, StorageError, resolve_state_directory,
};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn state_machine_allows_only_explicit_daily_plan_transitions() {
    let allowed = [
        (PlanState::Draft, PlanState::Ready),
        (PlanState::Ready, PlanState::Confirmed),
        (PlanState::Confirmed, PlanState::Ready),
        (PlanState::Confirmed, PlanState::Running),
        (PlanState::Running, PlanState::Succeeded),
        (PlanState::Running, PlanState::Failed),
        (PlanState::Running, PlanState::Stopped),
        (PlanState::Draft, PlanState::Cancelled),
        (PlanState::Ready, PlanState::Cancelled),
        (PlanState::Confirmed, PlanState::Cancelled),
    ];
    for (from, to) in allowed {
        validate_transition(from, to)
            .unwrap_or_else(|error| panic!("{from:?} -> {to:?} must be allowed: {error}"));
    }

    let forbidden = [
        (PlanState::Draft, PlanState::Confirmed),
        (PlanState::Ready, PlanState::Running),
        (PlanState::Confirmed, PlanState::Succeeded),
        (PlanState::Running, PlanState::Cancelled),
        (PlanState::Draft, PlanState::Draft),
    ];
    for (from, to) in forbidden {
        assert!(
            validate_transition(from, to).is_err(),
            "{from:?} -> {to:?} must be rejected"
        );
    }
}

#[test]
fn terminal_plan_states_have_no_outgoing_transitions() {
    let terminal = [
        PlanState::Succeeded,
        PlanState::Failed,
        PlanState::Stopped,
        PlanState::Cancelled,
    ];
    let all_states = [
        PlanState::Draft,
        PlanState::Ready,
        PlanState::Confirmed,
        PlanState::Running,
        PlanState::Succeeded,
        PlanState::Failed,
        PlanState::Stopped,
        PlanState::Cancelled,
    ];

    for from in terminal {
        for to in all_states {
            assert!(
                validate_transition(from, to).is_err(),
                "terminal transition {from:?} -> {to:?} must be rejected"
            );
        }
    }
}

#[test]
fn all_seven_whitelisted_daily_tasks_deserialize() {
    let valid_tasks = [
        json!({"type": "start_up", "params": {"start_game": true}}),
        json!({
            "type": "fight",
            "params": {
                "category": "event",
                "stage": "XX-8",
                "times": 5,
                "medicine": 0
            }
        }),
        json!({
            "type": "recruit",
            "params": {
                "max_recruits": 4,
                "confirm_recruitment": true,
                "use_expedited_plans": false
            }
        }),
        json!({"type": "infrast", "params": {}}),
        json!({
            "type": "mall",
            "params": {"visit_friends": true, "shopping": true}
        }),
        json!({
            "type": "award",
            "params": {
                "daily": true,
                "weekly": true,
                "mail": false,
                "event": false
            }
        }),
        json!({"type": "close_down", "params": {}}),
    ];

    for task in valid_tasks {
        serde_json::from_value::<DailyTask>(task).expect("deserialize whitelisted task");
    }
}

#[test]
fn daily_task_boundary_rejects_unknown_or_forbidden_fields() {
    let invalid_tasks = [
        json!({"type": "roguelike", "params": {}}),
        json!({
            "type": "fight",
            "params": {
                "category": "event",
                "stage": "XX-8",
                "times": 5,
                "medicine": 0,
                "stone": 1
            }
        }),
        json!({
            "type": "mall",
            "params": {
                "visit_friends": true,
                "shopping": true,
                "force_shopping_if_credit_full": true
            }
        }),
        json!({
            "type": "start_up",
            "params": {"start_game": true, "account": "another-account"}
        }),
        json!({
            "type": "award",
            "params": {
                "daily": true,
                "weekly": true,
                "mail": false,
                "event": false,
                "custom_task": "anything"
            }
        }),
        json!({
            "type": "infrast",
            "params": {},
            "arbitrary": {"type": "Fight"}
        }),
    ];

    for task in invalid_tasks {
        assert!(
            serde_json::from_value::<DailyTask>(task.clone()).is_err(),
            "must reject {task}"
        );
    }
}

#[test]
fn incomplete_plan_returns_stable_missing_fields_and_options() {
    let plan = DailyPlan::new(
        "",
        vec![
            DailyTask::StartUp(StartUpPlan::default()),
            DailyTask::Fight(FightPlan::default()),
            DailyTask::Recruit(RecruitPlan::default()),
        ],
    );

    let report = validate_plan(&plan);

    assert_eq!(
        report.missing_fields,
        [
            MissingField::Profile,
            MissingField::StartUpStartGame { task_index: 0 },
            MissingField::FightCategory { task_index: 1 },
            MissingField::FightStage { task_index: 1 },
            MissingField::FightTimes { task_index: 1 },
            MissingField::FightMedicine { task_index: 1 },
            MissingField::RecruitMaxRecruits { task_index: 2 },
            MissingField::RecruitConfirmRecruitment { task_index: 2 },
            MissingField::RecruitUseExpeditedPlans { task_index: 2 },
        ]
    );
    assert_eq!(
        MissingField::FightCategory { task_index: 1 }.valid_options(),
        ["main", "resource", "event", "annihilation"]
    );
    assert_eq!(
        MissingField::FightMedicine { task_index: 1 }.valid_options(),
        ["0..=10"]
    );
    assert!(!report.is_valid());
}

#[test]
fn policy_rejects_unsafe_ranges_stage_syntax_and_duplicates() {
    let unsafe_fight = FightPlan {
        category: Some(FightCategory::Event),
        stage: Some("../custom.json".to_owned()),
        times: Some(0),
        medicine: Some(11),
    };
    let plan = DailyPlan::new(
        "default",
        vec![
            DailyTask::StartUp(StartUpPlan {
                start_game: Some(true),
            }),
            DailyTask::StartUp(StartUpPlan {
                start_game: Some(false),
            }),
            DailyTask::Fight(unsafe_fight.clone()),
            DailyTask::Fight(unsafe_fight),
            DailyTask::Recruit(RecruitPlan {
                max_recruits: Some(5),
                confirm_recruitment: Some(true),
                use_expedited_plans: Some(true),
            }),
        ],
    );

    let report = validate_plan(&plan);

    assert!(report.errors.contains(&ValidationError::DuplicateTask {
        task: TaskKind::StartUp
    }));
    assert!(report.errors.contains(&ValidationError::DuplicateFight));
    assert!(
        report
            .errors
            .contains(&ValidationError::InvalidStage { task_index: 2 })
    );
    assert!(
        report
            .errors
            .contains(&ValidationError::FightTimesOutOfRange {
                task_index: 2,
                value: 0
            })
    );
    assert!(
        report
            .errors
            .contains(&ValidationError::MedicineOutOfRange {
                task_index: 2,
                value: 11
            })
    );
    assert!(
        report
            .errors
            .contains(&ValidationError::RecruitCountOutOfRange {
                task_index: 4,
                value: 5
            })
    );
    assert!(!report.is_valid());
}

#[test]
fn same_fight_target_is_duplicate_even_when_limits_differ() {
    let plan = DailyPlan::new(
        "default",
        vec![
            DailyTask::Fight(FightPlan {
                category: Some(FightCategory::Event),
                stage: Some("XX-8".to_owned()),
                times: Some(1),
                medicine: Some(0),
            }),
            DailyTask::Fight(FightPlan {
                category: Some(FightCategory::Event),
                stage: Some("XX-8".to_owned()),
                times: Some(2),
                medicine: Some(1),
            }),
        ],
    );

    assert!(
        validate_plan(&plan)
            .errors
            .contains(&ValidationError::DuplicateFight)
    );
}

#[test]
fn all_task_families_have_validation_success_and_failure_coverage() {
    assert!(validate_plan(&complete_plan()).is_valid());

    let invalid = DailyPlan::new(
        "default",
        vec![
            DailyTask::Infrast(InfrastPlan {}),
            DailyTask::Infrast(InfrastPlan {}),
            DailyTask::Mall(MallPlan::default()),
            DailyTask::Award(AwardPlan::default()),
            DailyTask::CloseDown(CloseDownPlan {}),
            DailyTask::CloseDown(CloseDownPlan {}),
        ],
    );
    let report = validate_plan(&invalid);

    assert!(report.errors.contains(&ValidationError::DuplicateTask {
        task: TaskKind::Infrast
    }));
    assert!(report.errors.contains(&ValidationError::DuplicateTask {
        task: TaskKind::CloseDown
    }));
    for missing in [
        MissingField::MallVisitFriends { task_index: 2 },
        MissingField::MallShopping { task_index: 2 },
        MissingField::AwardDaily { task_index: 3 },
        MissingField::AwardWeekly { task_index: 3 },
        MissingField::AwardMail { task_index: 3 },
        MissingField::AwardEvent { task_index: 3 },
    ] {
        assert!(report.missing_fields.contains(&missing));
    }
}

#[test]
fn canonical_hash_ignores_input_order_and_non_behavioral_plan_metadata() {
    let plan = complete_plan();
    let mut reordered = plan.clone();
    reordered.tasks.reverse();
    reordered.plan_id = complete_plan().plan_id;
    assert_ne!(plan.plan_id, reordered.plan_id);
    reordered.state = PlanState::Confirmed;
    reordered.revision = 42;
    reordered.confirmed_hash = Some(hash_plan(&plan).expect("hash complete plan"));

    assert_eq!(
        hash_plan(&plan).expect("hash original plan"),
        hash_plan(&reordered).expect("hash reordered plan")
    );
}

#[test]
fn plan_deserialization_rejects_operational_metadata_and_invalid_hashes() {
    let value = serde_json::to_value(complete_plan()).expect("serialize complete plan");

    for field in ["timestamp", "log_path"] {
        let mut with_metadata = value.clone();
        with_metadata
            .as_object_mut()
            .expect("plan object")
            .insert(field.to_owned(), json!("must-not-enter-domain"));
        assert!(
            serde_json::from_value::<DailyPlan>(with_metadata).is_err(),
            "plan must reject {field}"
        );
    }

    let mut with_invalid_hash = value;
    with_invalid_hash
        .as_object_mut()
        .expect("plan object")
        .insert("confirmed_hash".to_owned(), json!("not-a-sha256"));
    assert!(serde_json::from_value::<DailyPlan>(with_invalid_hash).is_err());
}

#[test]
fn profile_name_must_be_safe_or_reported_missing() {
    for profile in [" default", "default ", ".", "..", "bad\nprofile"] {
        let mut plan = complete_plan();
        plan.profile = profile.to_owned();
        assert!(
            validate_plan(&plan)
                .errors
                .contains(&ValidationError::InvalidProfile),
            "profile {profile:?} must be rejected"
        );
    }
}

#[test]
fn every_execution_change_invalidates_the_canonical_hash() {
    let plan = complete_plan();
    let original = hash_plan(&plan).expect("hash complete plan");
    let mut changed = Vec::new();

    let mut value = plan.clone();
    value.profile = "night".to_owned();
    changed.push(value);

    let mut value = plan.clone();
    let DailyTask::StartUp(task) = &mut value.tasks[0] else {
        panic!("expected StartUp");
    };
    task.start_game = Some(false);
    changed.push(value);

    for change_fight in [
        |fight: &mut FightPlan| fight.category = Some(FightCategory::Main),
        |fight: &mut FightPlan| fight.stage = Some("1-7".to_owned()),
        |fight: &mut FightPlan| fight.times = Some(6),
        |fight: &mut FightPlan| fight.medicine = Some(1),
    ] {
        let mut value = plan.clone();
        let DailyTask::Fight(task) = &mut value.tasks[1] else {
            panic!("expected Fight");
        };
        change_fight(task);
        changed.push(value);
    }

    for change_recruit in [
        |task: &mut RecruitPlan| task.max_recruits = Some(3),
        |task: &mut RecruitPlan| task.confirm_recruitment = Some(false),
        |task: &mut RecruitPlan| task.use_expedited_plans = Some(true),
    ] {
        let mut value = plan.clone();
        let DailyTask::Recruit(task) = &mut value.tasks[2] else {
            panic!("expected Recruit");
        };
        change_recruit(task);
        changed.push(value);
    }

    let mut value = plan.clone();
    value
        .tasks
        .retain(|task| !matches!(task, DailyTask::Infrast(_)));
    changed.push(value);

    for change_mall in [
        |task: &mut MallPlan| task.visit_friends = Some(false),
        |task: &mut MallPlan| task.shopping = Some(false),
    ] {
        let mut value = plan.clone();
        let DailyTask::Mall(task) = &mut value.tasks[4] else {
            panic!("expected Mall");
        };
        change_mall(task);
        changed.push(value);
    }

    for change_award in [
        |task: &mut AwardPlan| task.daily = Some(false),
        |task: &mut AwardPlan| task.weekly = Some(false),
        |task: &mut AwardPlan| task.mail = Some(true),
        |task: &mut AwardPlan| task.event = Some(true),
    ] {
        let mut value = plan.clone();
        let DailyTask::Award(task) = &mut value.tasks[5] else {
            panic!("expected Award");
        };
        change_award(task);
        changed.push(value);
    }

    let mut value = plan.clone();
    value
        .tasks
        .retain(|task| !matches!(task, DailyTask::CloseDown(_)));
    changed.push(value);

    for (index, changed_plan) in changed.iter().enumerate() {
        assert_ne!(
            hash_plan(changed_plan)
                .unwrap_or_else(|error| { panic!("hash execution change {index}: {error}") }),
            original,
            "execution change {index} must alter hash"
        );
    }
}

#[test]
fn chinese_summary_always_states_finite_resources_and_fixed_safety_values() {
    let summary = summarize_plan(&complete_plan()).expect("summarize complete plan");

    for required in [
        "类别：活动",
        "关卡：XX-8",
        "次数：5",
        "理智药：0",
        "源石：0（固定）",
        "公招消耗：招募许可最多 4",
        "加急许可：否",
        "信用购物：是",
        "保留现有黑名单：是（固定）",
        "账号切换：禁止（固定）",
        "关闭游戏：是",
    ] {
        assert!(summary.contains(required), "summary missing {required}");
    }
}

#[test]
fn summary_without_optional_task_families_still_states_all_safety_decisions() {
    let plan = DailyPlan::new("default", vec![DailyTask::Infrast(InfrastPlan {})]);
    let summary = summarize_plan(&plan).expect("summarize minimal safe plan");

    for required in [
        "关卡：无",
        "次数：0",
        "理智药：0",
        "源石：0（固定）",
        "公招消耗：招募许可最多 0",
        "信用购物：否",
        "账号切换：禁止（固定）",
        "关闭游戏：否",
    ] {
        assert!(summary.contains(required), "summary missing {required}");
    }
}

#[test]
fn summary_states_merged_award_collection_when_daily_and_weekly_differ() {
    let mut plan = complete_plan();
    let DailyTask::Award(award) = &mut plan.tasks[5] else {
        panic!("expected Award");
    };
    award.daily = Some(true);
    award.weekly = Some(false);

    let summary = summarize_plan(&plan).expect("summarize plan with split award choice");

    assert!(
        summary.contains("日常与周常任务奖励：是（MAA 合并领取，不可单独选择）"),
        "summary must state the merged award behavior: {summary}"
    );
    assert!(
        !summary.contains("周常：否"),
        "summary must not promise weekly rewards can be skipped separately: {summary}"
    );
}

#[test]
fn canonical_plan_contains_non_overridable_safety_constants() {
    let value = serde_json::to_value(normalize_plan(&complete_plan()).expect("normalize plan"))
        .expect("serialize normalized plan");
    let text = serde_json::to_string(&value).expect("serialize normalized plan text");

    assert!(text.contains(r#""stone":0"#));
    assert!(text.contains(r#""force_shopping_if_credit_full":false"#));
    assert!(text.contains(r#""preserve_blacklist":true"#));
    assert!(text.contains(r#""account_switching":false"#));
    assert!(text.contains(r#""paid_purchases":false"#));
}

#[test]
fn revising_a_confirmed_plan_clears_confirmation_and_increments_revision() {
    let mut plan = complete_plan();
    plan.state = PlanState::Confirmed;
    plan.confirmed_hash = Some(hash_plan(&plan).expect("hash confirmed plan"));
    let mut revised_tasks = plan.tasks.clone();
    let DailyTask::Fight(fight) = &mut revised_tasks[1] else {
        panic!("expected sample Fight");
    };
    fight.medicine = Some(2);

    revise_plan(&mut plan, "default".to_owned(), revised_tasks).expect("revise confirmed plan");

    assert_eq!(plan.state, PlanState::Ready);
    assert_eq!(plan.revision, 1);
    assert_eq!(plan.confirmed_hash, None);
}

#[test]
fn running_and_terminal_plans_reject_revisions_atomically() {
    for state in [
        PlanState::Running,
        PlanState::Succeeded,
        PlanState::Failed,
        PlanState::Stopped,
        PlanState::Cancelled,
    ] {
        let mut plan = complete_plan();
        plan.state = state;
        let before = plan.clone();

        assert!(
            revise_plan(&mut plan, "other".to_owned(), Vec::new()).is_err(),
            "{state:?} must reject revisions"
        );
        assert_eq!(plan, before, "{state:?} rejection must be atomic");
    }
}

fn complete_plan() -> DailyPlan {
    DailyPlan::new(
        "default",
        vec![
            DailyTask::StartUp(StartUpPlan {
                start_game: Some(true),
            }),
            DailyTask::Fight(FightPlan {
                category: Some(FightCategory::Event),
                stage: Some("XX-8".to_owned()),
                times: Some(5),
                medicine: Some(0),
            }),
            DailyTask::Recruit(RecruitPlan {
                max_recruits: Some(4),
                confirm_recruitment: Some(true),
                use_expedited_plans: Some(false),
            }),
            DailyTask::Infrast(InfrastPlan {}),
            DailyTask::Mall(MallPlan {
                visit_friends: Some(true),
                shopping: Some(true),
            }),
            DailyTask::Award(AwardPlan {
                daily: Some(true),
                weekly: Some(true),
                mail: Some(false),
                event: Some(false),
            }),
            DailyTask::CloseDown(CloseDownPlan {}),
        ],
    )
}

#[test]
fn state_directory_is_absolute_contained_and_has_a_windows_default() {
    let temp = tempdir().expect("create test directory");
    let explicit = temp.path().join("adapter-state");
    assert_eq!(
        resolve_state_directory(Some(&explicit), None).expect("resolve explicit state directory"),
        explicit
    );
    assert_eq!(
        resolve_state_directory(None, Some(temp.path().as_os_str()))
            .expect("resolve default state directory"),
        temp.path().join("maa-mcp-adapter")
    );
    assert!(resolve_state_directory(Some(Path::new("relative-state")), None).is_err());
    assert!(resolve_state_directory(Some(&temp.path().join("safe/../escape")), None).is_err());
}

#[test]
fn plan_repository_survives_restart_and_ignores_interrupted_temp_files() {
    let temp = tempdir().expect("create test directory");
    let repository =
        FilePlanRepository::new(temp.path().to_path_buf()).expect("create plan repository");
    let plan = complete_plan();
    repository.create(&plan).expect("create plan");

    fs::write(
        temp.path().join("plans/.interrupted-write.tmp"),
        br#"{"partial":"#,
    )
    .expect("simulate interrupted temporary write");
    drop(repository);

    let restarted =
        FilePlanRepository::new(temp.path().to_path_buf()).expect("restart plan repository");
    assert_eq!(
        restarted.load(&plan.plan_id).expect("load after restart"),
        plan
    );
}

#[test]
fn repository_revision_conflict_never_silently_overwrites() {
    let temp = tempdir().expect("create test directory");
    let repository =
        FilePlanRepository::new(temp.path().to_path_buf()).expect("create plan repository");
    let plan = complete_plan();
    repository.create(&plan).expect("create plan");

    let mut replacement = plan.clone();
    replacement.profile = "night".to_owned();
    replacement.revision = 1;
    repository
        .replace(0, &replacement)
        .expect("replace matching revision");

    let mut stale = plan.clone();
    stale.profile = "stale".to_owned();
    stale.revision = 1;
    let error = repository
        .replace(0, &stale)
        .expect_err("reject stale revision");
    assert!(matches!(
        error,
        StorageError::RevisionConflict {
            expected: 0,
            actual: 1
        }
    ));
    assert_eq!(
        repository
            .load(&plan.plan_id)
            .expect("load winning replacement"),
        replacement
    );
}

#[test]
fn plan_service_recovers_a_draft_then_previews_only_when_complete() {
    let temp = tempdir().expect("create test directory");
    let repository =
        FilePlanRepository::new(temp.path().to_path_buf()).expect("create plan repository");
    let service = PlanService::new(repository);
    let created = service
        .create("", vec![DailyTask::Fight(FightPlan::default())])
        .expect("create incomplete draft");
    let plan_id = created.plan.plan_id;
    assert_eq!(created.plan.state, PlanState::Draft);
    assert!(created.next_question.is_some());
    assert!(
        created
            .missing_fields
            .contains(&MissingField::FightStage { task_index: 0 })
    );
    drop(service);

    let restarted = PlanService::new(
        FilePlanRepository::new(temp.path().to_path_buf()).expect("restart plan repository"),
    );
    let with_profile = restarted
        .update(
            &plan_id,
            0,
            PlanPatch::SetProfile {
                profile: "default".to_owned(),
            },
        )
        .expect("set profile after restart");
    assert_eq!(with_profile.plan.revision, 1);
    assert_eq!(with_profile.plan.state, PlanState::Draft);

    let complete = restarted
        .update(
            &plan_id,
            1,
            PlanPatch::ReplaceTask {
                task_index: 0,
                task: DailyTask::Fight(FightPlan {
                    category: Some(FightCategory::Event),
                    stage: Some("XX-8".to_owned()),
                    times: Some(5),
                    medicine: Some(0),
                }),
            },
        )
        .expect("complete draft after restart");
    assert_eq!(complete.plan.revision, 2);
    assert_eq!(
        complete.plan.state,
        PlanState::Draft,
        "only preview may promote an ordinary draft"
    );

    let preview = restarted.preview(&plan_id).expect("preview complete draft");
    assert_eq!(preview.plan.state, PlanState::Ready);
    assert_eq!(preview.plan.revision, 3);
    assert!(preview.missing_fields.is_empty());
    assert!(
        preview
            .summary
            .as_deref()
            .is_some_and(|summary| summary.contains("关卡：XX-8"))
    );
}

#[test]
fn plan_service_cancels_unstarted_plan_and_rejects_later_updates() {
    let temp = tempdir().expect("create test directory");
    let service = PlanService::new(
        FilePlanRepository::new(temp.path().to_path_buf()).expect("create plan repository"),
    );
    let created = service
        .create("default", vec![DailyTask::Infrast(InfrastPlan {})])
        .expect("create complete draft");
    let ready = service
        .preview(&created.plan.plan_id)
        .expect("preview complete draft");
    let cancelled = service
        .cancel(&ready.plan.plan_id, ready.plan.revision)
        .expect("cancel ready plan");

    assert_eq!(cancelled.plan.state, PlanState::Cancelled);
    assert_eq!(cancelled.plan.confirmed_hash, None);
    assert!(
        service
            .update(
                &cancelled.plan.plan_id,
                cancelled.plan.revision,
                PlanPatch::SetProfile {
                    profile: "other".to_owned()
                }
            )
            .is_err()
    );
}

#[test]
fn plan_patch_deserialization_rejects_unknown_operations_and_fields() {
    assert!(
        serde_json::from_value::<PlanPatch>(json!({
            "operation": "shell",
            "command": "maa run anything"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<PlanPatch>(json!({
            "operation": "set_profile",
            "profile": "default",
            "unknown": true
        }))
        .is_err()
    );
}

#[test]
fn cleanup_removes_only_old_cancelled_plans_and_preserves_uncertain_records() {
    let temp = tempdir().expect("create test directory");
    let repository =
        FilePlanRepository::new(temp.path().to_path_buf()).expect("create plan repository");

    let mut cancelled = complete_plan();
    cancelled.state = PlanState::Cancelled;
    let mut ready = complete_plan();
    ready.state = PlanState::Ready;
    let mut confirmed = complete_plan();
    confirmed.state = PlanState::Confirmed;
    confirmed.confirmed_hash = Some(hash_plan(&confirmed).expect("hash confirmed plan"));
    for plan in [&cancelled, &ready, &confirmed] {
        repository.create(plan).expect("create cleanup fixture");
    }
    let corrupt_path = temp.path().join("plans/corrupt.json");
    let temporary_path = temp.path().join("plans/.unfinished.tmp");
    fs::write(&corrupt_path, "{not-json").expect("create corrupt record");
    fs::write(&temporary_path, "partial").expect("create temporary record");

    let removed = repository
        .cleanup_cancelled(SystemTime::now() + Duration::from_secs(7 * 24 * 60 * 60 + 1))
        .expect("clean old cancelled plans");

    assert_eq!(removed, 1);
    assert!(matches!(
        repository.load(&cancelled.plan_id),
        Err(StorageError::NotFound)
    ));
    assert_eq!(repository.load(&ready.plan_id).expect("keep Ready"), ready);
    assert_eq!(
        repository.load(&confirmed.plan_id).expect("keep Confirmed"),
        confirmed
    );
    assert!(corrupt_path.exists(), "corrupt records must fail closed");
    assert!(temporary_path.exists(), "temporary files are never plans");
    assert!("..".parse::<maa_mcp_adapter::domain::PlanId>().is_err());
    assert!(
        "not-a-uuid"
            .parse::<maa_mcp_adapter::domain::PlanId>()
            .is_err()
    );
}
