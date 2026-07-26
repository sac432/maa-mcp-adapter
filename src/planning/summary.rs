use crate::domain::{DailyPlan, FightCategory};

use super::DomainError;
use super::normalize::{CanonicalTask, normalize_plan};

pub fn summarize_plan(plan: &DailyPlan) -> Result<String, DomainError> {
    let plan = normalize_plan(plan)?;

    let mut total_medicine: u32 = 0;
    let mut recruit_permits: u8 = 0;
    let mut expedited_permits: u8 = 0;
    for task in &plan.tasks {
        match task {
            CanonicalTask::Fight { medicine, .. } => {
                total_medicine += u32::from(*medicine);
            }
            CanonicalTask::Recruit {
                max_recruits,
                confirm_recruitment,
                use_expedited_plans,
            } => {
                if *confirm_recruitment {
                    recruit_permits = recruit_permits.saturating_add(*max_recruits);
                }
                if *use_expedited_plans {
                    expedited_permits = expedited_permits.saturating_add(*max_recruits);
                }
            }
            _ => {}
        }
    }

    let mut lines = vec![
        format!("配置档：{}", plan.profile),
        format!(
            "消耗上限：理智药 {total_medicine}；招募许可 {recruit_permits}；加急许可 {expedited_permits}；源石 0（固定）"
        ),
        "账号切换：禁止（固定）".to_owned(),
    ];

    let mut has_fight = false;
    let mut has_recruit = false;
    let mut has_mall = false;
    let mut closes_game = false;

    for task in plan.tasks {
        match task {
            CanonicalTask::StartUp { start_game } => {
                lines.push(format!("启动游戏：{}", yes_no(start_game)));
            }
            CanonicalTask::Fight {
                category,
                stage,
                times,
                medicine,
                stone,
            } => {
                has_fight = true;
                lines.push(format!(
                    "战斗：类别：{}；关卡：{stage}；次数：{times}；理智药：{medicine}；源石：{stone}（固定）",
                    fight_category_name(category)
                ));
            }
            CanonicalTask::Recruit {
                max_recruits,
                confirm_recruitment,
                use_expedited_plans,
            } => {
                has_recruit = true;
                lines.push(format!(
                    "公招消耗：招募许可最多 {max_recruits}；确认招募：{}；加急许可：{}",
                    yes_no(confirm_recruitment),
                    yes_no(use_expedited_plans)
                ));
            }
            CanonicalTask::Infrast {} => {
                lines.push("基建换班：使用现有配置档".to_owned());
            }
            CanonicalTask::Mall {
                visit_friends,
                shopping,
                force_shopping_if_credit_full,
                preserve_blacklist,
            } => {
                has_mall = true;
                lines.push(format!(
                    "信用商店：访问好友：{}；信用购物：{}；信用满时强制购物：{}（固定）；保留现有黑名单：{}（固定）",
                    yes_no(visit_friends),
                    yes_no(shopping),
                    yes_no(force_shopping_if_credit_full),
                    yes_no(preserve_blacklist)
                ));
            }
            CanonicalTask::Award {
                daily,
                weekly,
                mail,
                event,
            } => {
                lines.push(format!(
                    "奖励：日常与周常任务奖励：{}（MAA 合并领取，不可单独选择）；邮件：{}；活动：{}",
                    yes_no(daily || weekly),
                    yes_no(mail),
                    yes_no(event)
                ));
            }
            CanonicalTask::CloseDown {} => closes_game = true,
        }
    }

    if !has_fight {
        lines.push("战斗：关卡：无；次数：0；理智药：0；源石：0（固定）".to_owned());
    }
    if !has_recruit {
        lines.push("公招消耗：招募许可最多 0；确认招募：否；加急许可：否".to_owned());
    }
    if !has_mall {
        lines.push(
            "信用商店：访问好友：否；信用购物：否；信用满时强制购物：否（固定）；保留现有黑名单：是（固定）"
                .to_owned(),
        );
    }
    lines.push(format!("关闭游戏：{}", yes_no(closes_game)));
    lines.push("付费购买：禁止（固定）".to_owned());

    Ok(lines.join("\n"))
}

fn yes_no(value: bool) -> &'static str {
    if value { "是" } else { "否" }
}

fn fight_category_name(category: FightCategory) -> &'static str {
    match category {
        FightCategory::Main => "主线",
        FightCategory::Resource => "资源关",
        FightCategory::Event => "活动",
        FightCategory::Annihilation => "剿灭",
    }
}
