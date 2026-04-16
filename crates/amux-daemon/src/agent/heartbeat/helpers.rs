#![allow(dead_code)]

use super::*;
use std::collections::HashMap;

pub(super) fn check_quiet_window(
    hour: u32,
    start: Option<u32>,
    end: Option<u32>,
    dnd: bool,
) -> bool {
    if dnd {
        return true;
    }
    let (s, e) = match (start, end) {
        (Some(s), Some(e)) => (s, e),
        _ => return false,
    };
    if s <= e {
        hour >= s && hour < e
    } else {
        hour >= s || hour < e
    }
}

pub(in crate::agent) fn resolve_cron_from_config(config: &AgentConfig) -> String {
    config
        .heartbeat_cron
        .clone()
        .unwrap_or_else(|| interval_mins_to_cron(config.heartbeat_interval_mins))
}

pub(super) fn should_broadcast(actionable: bool, items: &[HeartbeatDigestItem]) -> bool {
    actionable || !items.is_empty()
}

pub(super) fn heartbeat_persistence_status(synthesis_json: Option<&str>) -> &'static str {
    if synthesis_json.is_some() {
        "completed"
    } else {
        "synthesis_failed"
    }
}

pub(super) fn is_custom_item_due(
    now: u64,
    last_run_at: Option<u64>,
    item_interval_minutes: u64,
    global_interval_mins: u64,
) -> bool {
    let interval_ms = if item_interval_minutes > 0 {
        item_interval_minutes * 60 * 1000
    } else {
        global_interval_mins * 60 * 1000
    };
    match last_run_at {
        Some(last) => now.saturating_sub(last) >= interval_ms,
        None => true,
    }
}

pub(super) fn parse_digest_items(response: &str) -> Vec<HeartbeatDigestItem> {
    response
        .lines()
        .filter(|l| l.trim_start().starts_with("- PRIORITY:"))
        .filter_map(|line| {
            let priority = line
                .split("PRIORITY:")
                .nth(1)?
                .split_whitespace()
                .next()?
                .parse::<u8>()
                .ok()?;
            let type_str = line.split("TYPE:").nth(1)?.split_whitespace().next()?;
            let check_type = match type_str.to_lowercase().as_str() {
                "staletodos" | "stale_todos" => HeartbeatCheckType::StaleTodos,
                "stuckgoalruns" | "stuck_goal_runs" => HeartbeatCheckType::StuckGoalRuns,
                "unrepliedgatewaymessages" | "unreplied_gateway_messages" => {
                    HeartbeatCheckType::UnrepliedGatewayMessages
                }
                "repochanges" | "repo_changes" => HeartbeatCheckType::RepoChanges,
                "pluginauth" | "plugin_auth" => HeartbeatCheckType::PluginAuth,
                _ => HeartbeatCheckType::StaleTodos,
            };
            let title = line
                .split("TITLE:")
                .nth(1)?
                .split("SUGGESTION:")
                .next()?
                .trim()
                .to_string();
            let suggestion = line.split("SUGGESTION:").nth(1)?.trim().to_string();
            Some(HeartbeatDigestItem {
                priority,
                check_type,
                title,
                suggestion,
            })
        })
        .collect()
}

pub(super) fn format_anticipatory_items_for_heartbeat(items: &[AnticipatoryItem]) -> String {
    items
        .iter()
        .map(|item| {
            let priority_hint = if item.kind == "hydration" {
                "LOW-PRIORITY INFORMATIONAL"
            } else if item.kind == "system_outcome_foresight" {
                "OPERATOR-VISIBLE FORESIGHT"
            } else {
                "ACTIONABLE"
            };
            let trigger_suffix = item
                .bullets
                .iter()
                .find_map(|bullet| bullet.strip_prefix("prediction_type="))
                .map(|value| format!(" trigger={}", value.trim()))
                .unwrap_or_default();
            let bullets_text = if !item.bullets.is_empty() {
                format!("\n    Bullets: {}", item.bullets.join(", "))
            } else {
                String::new()
            };
            format!(
                "- [{}] ({}) {}{} (confidence: {:.2}): {}{}",
                item.kind,
                priority_hint,
                item.title,
                trigger_suffix,
                item.confidence,
                item.summary,
                bullets_text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn format_consolidation_forge_summary(result: &ConsolidationResult) -> Option<String> {
    if !result.forge_ran {
        return None;
    }

    let logged_only_suffix = if result.forge_hints_logged_only > 0 {
        format!(", {} logged-only", result.forge_hints_logged_only)
    } else {
        String::new()
    };

    Some(format!(
        "forge learned from {} traces: {} pattern(s), {} hint(s) generated, {} auto-applied{}.",
        result.forge_traces_analyzed,
        result.forge_patterns_detected,
        result.forge_hints_generated,
        result.forge_hints_auto_applied,
        logged_only_suffix,
    ))
}

pub(super) fn format_consolidation_dream_summary(result: &ConsolidationResult) -> Option<String> {
    let learned_something = result.distillation_ran
        || result.forge_ran
        || result.facts_refined > 0
        || result.skills_promoted > 0;
    if !learned_something {
        return None;
    }

    let review_queue_suffix = if result.distillation_queued_for_review > 0 {
        format!(
            ", {} queued for review",
            result.distillation_queued_for_review
        )
    } else {
        String::new()
    };

    Some(format!(
        "Dream state: what the system considered while idle — {} thread(s), {} memory update(s){}, {} recurring pattern(s), {} refined fact(s), {} promoted skill(s).",
        result.distillation_threads_analyzed,
        result.distillation_auto_applied,
        review_queue_suffix,
        result.forge_patterns_detected,
        result.facts_refined,
        result.skills_promoted,
    ))
}

pub(super) fn check_type_to_action_type(check_type: &HeartbeatCheckType) -> &'static str {
    match check_type {
        HeartbeatCheckType::StaleTodos => "stale_todo",
        HeartbeatCheckType::StuckGoalRuns => "stuck_goal",
        HeartbeatCheckType::UnrepliedGatewayMessages => "unreplied_message",
        HeartbeatCheckType::RepoChanges => "repo_change",
        HeartbeatCheckType::PluginAuth => "plugin_auth",
        HeartbeatCheckType::SkillLifecycle => "skill_lifecycle",
    }
}

pub(crate) fn enabled_checks(config: &HeartbeatChecksConfig) -> Vec<HeartbeatCheckType> {
    let mut checks = Vec::new();
    if config.stale_todos_enabled {
        checks.push(HeartbeatCheckType::StaleTodos);
    }
    if config.stuck_goals_enabled {
        checks.push(HeartbeatCheckType::StuckGoalRuns);
    }
    if config.unreplied_messages_enabled {
        checks.push(HeartbeatCheckType::UnrepliedGatewayMessages);
    }
    if config.repo_changes_enabled {
        checks.push(HeartbeatCheckType::RepoChanges);
    }
    if config.plugin_auth_enabled {
        checks.push(HeartbeatCheckType::PluginAuth);
    }
    checks
}

pub(in crate::agent) fn is_peak_activity_hour(
    current_hour_utc: u8,
    peak_hours: &[u8],
    smoothed_histogram: &HashMap<u8, f64>,
    ema_threshold: f64,
) -> bool {
    peak_hours.contains(&current_hour_utc)
        || smoothed_histogram
            .get(&current_hour_utc)
            .map(|&count| count >= ema_threshold)
            .unwrap_or(false)
}

pub(super) fn should_run_check(weight: f64, cycle_count: u64) -> bool {
    if weight >= 1.0 {
        return true;
    }
    if weight <= 0.0 {
        return false;
    }
    let skip_factor = (1.0 / weight).round() as u64;
    if skip_factor == 0 {
        return true;
    }
    cycle_count % skip_factor == 0
}

pub(crate) fn compute_check_priority(
    dismiss_count: u64,
    inaction_count: u64,
    total_shown: u64,
    recovery_count: u64,
    decay_rate: f64,
    recovery_rate: f64,
) -> f64 {
    let dismiss_penalty = (dismiss_count as f64 * decay_rate).min(0.6);
    let inaction_penalty = if total_shown > 0 {
        let inaction_rate = inaction_count as f64 / total_shown as f64;
        (inaction_rate * 0.4).min(0.3)
    } else {
        0.0
    };
    let recovery_bonus = (recovery_count as f64 * recovery_rate).min(0.5);
    let raw = 1.0 - dismiss_penalty - inaction_penalty + recovery_bonus;
    raw.clamp(0.1, 1.0)
}
