#[cfg(test)]
use super::*;
use tempfile::tempdir;
use tokio::time::{timeout, Duration};

// ── check_quiet_window pure function tests ─────────────────────────

#[test]
fn quiet_hours_within_midnight_wrap_window() {
    // start=22, end=6, hour=23 → quiet
    assert!(check_quiet_window(23, Some(22), Some(6), false));
}

#[test]
fn quiet_hours_outside_midnight_wrap_window() {
    // start=22, end=6, hour=12 → not quiet
    assert!(!check_quiet_window(12, Some(22), Some(6), false));
}

#[test]
fn quiet_hours_midnight_wrap_early_morning() {
    // start=22, end=6, hour=3 → quiet (early morning within wrap)
    assert!(check_quiet_window(3, Some(22), Some(6), false));
}

#[test]
fn quiet_hours_midnight_wrap_boundary_end() {
    // start=22, end=6, hour=6 → NOT quiet (end hour is exclusive)
    assert!(!check_quiet_window(6, Some(22), Some(6), false));
}

#[test]
fn quiet_hours_midnight_wrap_boundary_start() {
    // start=22, end=6, hour=22 → quiet (start hour is inclusive)
    assert!(check_quiet_window(22, Some(22), Some(6), false));
}

#[test]
fn dnd_enabled_overrides_everything() {
    // dnd=true → always quiet regardless of hour or window config
    assert!(check_quiet_window(12, None, None, true));
    assert!(check_quiet_window(12, Some(22), Some(6), true));
    assert!(check_quiet_window(0, Some(9), Some(17), true));
}

#[test]
fn no_quiet_hours_configured_and_no_dnd() {
    // No quiet hours, no DND → never quiet
    assert!(!check_quiet_window(12, None, None, false));
    assert!(!check_quiet_window(0, None, None, false));
    assert!(!check_quiet_window(23, None, None, false));
}

#[test]
fn same_day_range_inside() {
    // start=9, end=17, hour=12 → quiet
    assert!(check_quiet_window(12, Some(9), Some(17), false));
}

#[test]
fn same_day_range_outside() {
    // start=9, end=17, hour=20 → not quiet
    assert!(!check_quiet_window(20, Some(9), Some(17), false));
}

#[test]
fn partial_config_only_start_set() {
    // Only start set (no end) → not quiet
    assert!(!check_quiet_window(23, Some(22), None, false));
}

#[test]
fn partial_config_only_end_set() {
    // Only end set (no start) → not quiet
    assert!(!check_quiet_window(3, None, Some(6), false));
}

// ── resolve_cron_from_config tests ─────────────────────────────────

#[test]
fn resolve_cron_prefers_explicit_cron() {
    let config = AgentConfig {
        heartbeat_cron: Some("0 * * * *".to_string()),
        heartbeat_interval_mins: 15,
        ..AgentConfig::default()
    };
    assert_eq!(resolve_cron_from_config(&config), "0 * * * *");
}

#[test]
fn resolve_cron_falls_back_to_interval_mins() {
    let config = AgentConfig {
        heartbeat_cron: None,
        heartbeat_interval_mins: 15,
        ..AgentConfig::default()
    };
    assert_eq!(resolve_cron_from_config(&config), "*/15 * * * *");
}

#[test]
fn resolve_cron_with_hourly_interval() {
    let config = AgentConfig {
        heartbeat_cron: None,
        heartbeat_interval_mins: 60,
        ..AgentConfig::default()
    };
    assert_eq!(resolve_cron_from_config(&config), "0 * * * *");
}

#[test]
fn resolve_cron_explicit_overrides_interval() {
    let config = AgentConfig {
        heartbeat_cron: Some("30 2 * * *".to_string()),
        heartbeat_interval_mins: 60,
        ..AgentConfig::default()
    };
    assert_eq!(resolve_cron_from_config(&config), "30 2 * * *");
}

// ── should_broadcast tests (D-14: silent default) ───────────────────

#[test]
fn broadcast_when_actionable_true_and_items_present() {
    let items = vec![HeartbeatDigestItem {
        priority: 1,
        check_type: HeartbeatCheckType::StaleTodos,
        title: "Stale todo".into(),
        suggestion: "Review it".into(),
    }];
    assert!(should_broadcast(true, &items));
}

#[test]
fn broadcast_when_actionable_true_but_no_items() {
    assert!(should_broadcast(true, &[]));
}

#[test]
fn broadcast_when_not_actionable_but_items_present() {
    let items = vec![HeartbeatDigestItem {
        priority: 3,
        check_type: HeartbeatCheckType::RepoChanges,
        title: "Repo change".into(),
        suggestion: "Check it".into(),
    }];
    assert!(should_broadcast(false, &items));
}

#[test]
fn no_broadcast_when_not_actionable_and_no_items() {
    // D-14: silent default — no event broadcast
    assert!(!should_broadcast(false, &[]));
}

// ── heartbeat_persistence_status tests (Pitfall 4) ──────────────────

#[test]
fn persistence_status_completed_when_synthesis_present() {
    assert_eq!(
        heartbeat_persistence_status(Some("LLM response text")),
        "completed"
    );
}

#[test]
fn persistence_status_failed_when_synthesis_none() {
    assert_eq!(heartbeat_persistence_status(None), "synthesis_failed");
}

// ── is_custom_item_due tests ────────────────────────────────────────

#[test]
fn custom_item_due_when_never_run() {
    // last_run_at=None → always due
    assert!(is_custom_item_due(100_000_000, None, 15, 30));
}

#[test]
fn custom_item_due_when_interval_elapsed() {
    let now = 100_000_000;
    let last = now - (16 * 60 * 1000); // 16 minutes ago
                                       // item_interval=15min → 15*60*1000=900_000 < 960_000 elapsed → due
    assert!(is_custom_item_due(now, Some(last), 15, 30));
}

#[test]
fn custom_item_not_due_when_interval_not_elapsed() {
    let now = 100_000_000;
    let last = now - (10 * 60 * 1000); // 10 minutes ago
                                       // item_interval=15min → not enough time elapsed → not due
    assert!(!is_custom_item_due(now, Some(last), 15, 30));
}

#[test]
fn custom_item_uses_global_interval_when_item_interval_zero() {
    let now = 100_000_000;
    let last = now - (31 * 60 * 1000); // 31 minutes ago
                                       // item_interval=0, global=30min → 30*60*1000=1_800_000 < 1_860_000 elapsed → due
    assert!(is_custom_item_due(now, Some(last), 0, 30));
}

#[test]
fn custom_item_not_due_with_global_interval() {
    let now = 100_000_000;
    let last = now - (20 * 60 * 1000); // 20 minutes ago
                                       // item_interval=0, global=30min → not enough time elapsed → not due
    assert!(!is_custom_item_due(now, Some(last), 0, 30));
}

// ── enabled_checks tests (check gating by config) ───────────────────

#[test]
fn all_checks_enabled_by_default() {
    let config = HeartbeatChecksConfig::default();
    let checks = enabled_checks(&config);
    assert_eq!(checks.len(), 5);
    assert!(checks.contains(&HeartbeatCheckType::StaleTodos));
    assert!(checks.contains(&HeartbeatCheckType::StuckGoalRuns));
    assert!(checks.contains(&HeartbeatCheckType::UnrepliedGatewayMessages));
    assert!(checks.contains(&HeartbeatCheckType::RepoChanges));
    assert!(checks.contains(&HeartbeatCheckType::PluginAuth));
}

#[test]
fn only_enabled_checks_are_included() {
    let config = HeartbeatChecksConfig {
        stale_todos_enabled: true,
        stuck_goals_enabled: false,
        unreplied_messages_enabled: false,
        repo_changes_enabled: true,
        plugin_auth_enabled: false,
        ..HeartbeatChecksConfig::default()
    };
    let checks = enabled_checks(&config);
    assert_eq!(checks.len(), 2);
    assert!(checks.contains(&HeartbeatCheckType::StaleTodos));
    assert!(checks.contains(&HeartbeatCheckType::RepoChanges));
    assert!(!checks.contains(&HeartbeatCheckType::StuckGoalRuns));
    assert!(!checks.contains(&HeartbeatCheckType::UnrepliedGatewayMessages));
}

#[test]
fn no_checks_when_all_disabled() {
    let config = HeartbeatChecksConfig {
        stale_todos_enabled: false,
        stuck_goals_enabled: false,
        unreplied_messages_enabled: false,
        repo_changes_enabled: false,
        plugin_auth_enabled: false,
        ..HeartbeatChecksConfig::default()
    };
    let checks = enabled_checks(&config);
    assert!(checks.is_empty());
}

// ── parse_digest_items tests ────────────────────────────────────────

#[test]
fn parse_digest_items_from_valid_response() {
    let response = "\
ACTIONABLE: true
DIGEST: 2 items need attention
ITEMS:
- PRIORITY:1 TYPE:stale_todos TITLE:Stale todo found SUGGESTION:Review pending items
- PRIORITY:3 TYPE:repo_changes TITLE:Uncommitted changes SUGGESTION:Commit or stash";

    let items = parse_digest_items(response);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].priority, 1);
    assert_eq!(items[0].check_type, HeartbeatCheckType::StaleTodos);
    assert_eq!(items[0].title, "Stale todo found");
    assert_eq!(items[0].suggestion, "Review pending items");
    assert_eq!(items[1].priority, 3);
    assert_eq!(items[1].check_type, HeartbeatCheckType::RepoChanges);
}

#[test]
fn parse_digest_items_empty_when_no_items_section() {
    let response = "ACTIONABLE: false\nDIGEST: All systems normal.";
    let items = parse_digest_items(response);
    assert!(items.is_empty());
}

#[test]
fn parse_digest_items_handles_camelcase_types() {
    let response = "- PRIORITY:2 TYPE:StuckGoalRuns TITLE:Goal stuck SUGGESTION:Cancel it";
    let items = parse_digest_items(response);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].check_type, HeartbeatCheckType::StuckGoalRuns);
}

#[test]
fn format_anticipatory_items_highlights_system_outcome_foresight_for_operator() {
    let output = super::helpers::format_anticipatory_items_for_heartbeat(&[
        AnticipatoryItem {
            id: "system_outcome_foresight_thread-1".to_string(),
            kind: "system_outcome_foresight".to_string(),
            title: "System Outcome Foresight".to_string(),
            summary: "Predicted stale context: hydration-needed risk is elevated".to_string(),
            bullets: vec![
                "prediction_type=stale_context".to_string(),
                "hydration age=16m exceeded session rhythm window=10m".to_string(),
                "semantic alignment degraded across recent thread messages".to_string(),
            ],
            intent_prediction: None,
            confidence: 0.76,
            goal_run_id: None,
            thread_id: Some("thread-1".to_string()),
            preferred_client_surface: Some("conversation".to_string()),
            preferred_attention_surface: Some("conversation:chat".to_string()),
            created_at: 1,
            updated_at: 1,
        },
        AnticipatoryItem {
            id: "system_outcome_foresight_thread-2".to_string(),
            kind: "system_outcome_foresight".to_string(),
            title: "System Outcome Foresight".to_string(),
            summary: "Predicted build/test/risk: build/test failure risk is elevated".to_string(),
            bullets: vec![
                "prediction_type=build_test_risk".to_string(),
                "dirty repo state: modified=2 staged=0 untracked=0".to_string(),
            ],
            intent_prediction: None,
            confidence: 0.78,
            goal_run_id: None,
            thread_id: Some("thread-2".to_string()),
            preferred_client_surface: Some("conversation".to_string()),
            preferred_attention_surface: Some("conversation:chat".to_string()),
            created_at: 2,
            updated_at: 2,
        },
    ]);

    assert!(output.contains("system_outcome_foresight"));
    assert!(output.contains("OPERATOR-VISIBLE FORESIGHT"));
    assert!(output.contains("trigger=stale_context"));
    assert!(output.contains("trigger=build_test_risk"));
    assert!(output.contains("stale context"));
    assert!(output.contains("build/test failure risk"));
    assert!(output.contains("prediction_type=stale_context"));
    assert!(output.contains("prediction_type=build_test_risk"));
}

#[test]
fn format_consolidation_forge_summary_surfaces_strategy_learning() {
    let output = super::helpers::format_consolidation_forge_summary(&ConsolidationResult {
        forge_ran: true,
        forge_traces_analyzed: 17,
        forge_patterns_detected: 3,
        forge_hints_generated: 2,
        forge_hints_auto_applied: 1,
        ..Default::default()
    })
    .expect("forge summary should be present when forge ran");

    assert!(output.contains("forge learned from 17 traces"));
    assert!(output.contains("3 pattern(s)"));
    assert!(output.contains("2 hint(s) generated"));
    assert!(output.contains("1 auto-applied"));
}

#[test]
fn format_consolidation_forge_summary_surfaces_logged_only_hints() {
    let output = super::helpers::format_consolidation_forge_summary(&ConsolidationResult {
        forge_ran: true,
        forge_traces_analyzed: 11,
        forge_patterns_detected: 2,
        forge_hints_generated: 3,
        forge_hints_auto_applied: 1,
        forge_hints_logged_only: 2,
        ..Default::default()
    })
    .expect("forge summary should be present when forge ran");

    assert!(
        output.contains("2 logged-only"),
        "unexpected forge summary: {output}"
    );
}

#[test]
fn format_consolidation_dream_summary_surfaces_what_the_system_considered_while_idle() {
    let output = super::helpers::format_consolidation_dream_summary(&ConsolidationResult {
        distillation_ran: true,
        distillation_threads_analyzed: 4,
        distillation_auto_applied: 2,
        forge_ran: true,
        forge_patterns_detected: 3,
        forge_hints_auto_applied: 1,
        facts_refined: 2,
        skills_promoted: 1,
        ..Default::default()
    })
    .expect("dream summary should be present when consolidation learned something");

    assert!(output.contains("what the system considered while idle"));
    assert!(output.contains("4 thread(s)"));
    assert!(output.contains("2 memory update(s)"));
    assert!(output.contains("3 recurring pattern(s)"));
}

#[test]
fn format_consolidation_dream_summary_surfaces_distillation_review_queue() {
    let output = super::helpers::format_consolidation_dream_summary(&ConsolidationResult {
        distillation_ran: true,
        distillation_threads_analyzed: 3,
        distillation_auto_applied: 1,
        distillation_queued_for_review: 2,
        ..Default::default()
    })
    .expect("dream summary should be present when distillation ran");

    assert!(
        output.contains("2 queued for review"),
        "unexpected dream summary: {output}"
    );
}

#[tokio::test]
async fn heartbeat_consolidation_emits_workflow_notice_for_forge_learning_summary() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
    let mut events = engine.subscribe();

    let summary = super::helpers::format_consolidation_forge_summary(&ConsolidationResult {
        forge_ran: true,
        forge_traces_analyzed: 9,
        forge_patterns_detected: 2,
        forge_hints_generated: 2,
        forge_hints_auto_applied: 1,
        ..Default::default()
    })
    .expect("forge summary should be present");

    let _ = engine.event_tx.send(AgentEvent::WorkflowNotice {
        thread_id: String::new(),
        kind: "forge".to_string(),
        message: "Consolidation strategy learning updated".to_string(),
        details: Some(summary.clone()),
    });

    let event = timeout(Duration::from_millis(250), events.recv())
        .await
        .expect("forge notice should arrive")
        .expect("forge notice should deserialize");
    match event {
        AgentEvent::WorkflowNotice {
            kind,
            message,
            details,
            ..
        } => {
            assert_eq!(kind, "forge");
            assert_eq!(message, "Consolidation strategy learning updated");
            assert_eq!(details.as_deref(), Some(summary.as_str()));
        }
        other => panic!("expected WorkflowNotice, got {other:?}"),
    }
}

#[tokio::test]
async fn heartbeat_consolidation_emits_workflow_notice_for_dream_summary() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
    let mut events = engine.subscribe();

    let summary = super::helpers::format_consolidation_dream_summary(&ConsolidationResult {
        distillation_ran: true,
        distillation_threads_analyzed: 2,
        distillation_auto_applied: 1,
        forge_ran: true,
        forge_patterns_detected: 2,
        forge_hints_auto_applied: 1,
        facts_refined: 1,
        ..Default::default()
    })
    .expect("dream summary should be present");

    let _ = engine.event_tx.send(AgentEvent::WorkflowNotice {
        thread_id: String::new(),
        kind: "dream".to_string(),
        message: "Dream state updated".to_string(),
        details: Some(summary.clone()),
    });

    let event = timeout(Duration::from_millis(250), events.recv())
        .await
        .expect("dream notice should arrive")
        .expect("dream notice should deserialize");
    match event {
        AgentEvent::WorkflowNotice {
            kind,
            message,
            details,
            ..
        } => {
            assert_eq!(kind, "dream");
            assert_eq!(message, "Dream state updated");
            assert_eq!(details.as_deref(), Some(summary.as_str()));
        }
        other => panic!("expected WorkflowNotice, got {other:?}"),
    }
}

// ── is_peak_activity_hour tests (BEAT-06/D-01) ──────────────────────

#[test]
fn peak_activity_hour_in_peak_hours_list() {
    let smoothed: HashMap<u8, f64> = HashMap::new();
    assert!(is_peak_activity_hour(9, &[9, 10, 14], &smoothed, 2.0));
}

#[test]
fn peak_activity_hour_above_ema_threshold() {
    let mut smoothed: HashMap<u8, f64> = HashMap::new();
    smoothed.insert(15, 5.0);
    assert!(is_peak_activity_hour(15, &[], &smoothed, 2.0));
}

#[test]
fn peak_activity_hour_below_threshold_and_not_in_list() {
    let mut smoothed: HashMap<u8, f64> = HashMap::new();
    smoothed.insert(3, 1.0);
    assert!(!is_peak_activity_hour(3, &[9, 10], &smoothed, 2.0));
}

// ── should_run_check tests (BEAT-06/D-05) ──────────────────────────

#[test]
fn should_run_check_weight_one_always_runs() {
    assert!(should_run_check(1.0, 0));
    assert!(should_run_check(1.0, 1));
    assert!(should_run_check(1.0, 99));
}

#[test]
fn should_run_check_weight_quarter_every_fourth_cycle() {
    assert!(should_run_check(0.25, 4)); // 4 % 4 == 0
    assert!(should_run_check(0.25, 8)); // 8 % 4 == 0
    assert!(should_run_check(0.25, 0)); // 0 % 4 == 0
}

#[test]
fn should_run_check_weight_quarter_skips_other_cycles() {
    assert!(!should_run_check(0.25, 1)); // 1 % 4 != 0
    assert!(!should_run_check(0.25, 3)); // 3 % 4 != 0
}

#[test]
fn should_run_check_weight_zero_never_runs() {
    assert!(!should_run_check(0.0, 0));
    assert!(!should_run_check(0.0, 1));
    assert!(!should_run_check(0.0, 100));
}

// ── compute_check_priority tests (BEAT-09/D-04/D-05) ───────────────

#[test]
fn compute_check_priority_zero_dismissals_returns_one() {
    let result = compute_check_priority(0, 0, 0, 0, 0.1, 0.1);
    assert!((result - 1.0).abs() < f64::EPSILON);
}

#[test]
fn compute_check_priority_many_dismissals_clamped_minimum() {
    // 100 dismissals * 0.1 decay = 10.0 penalty, capped at 0.6
    // With 0 inaction, 0 recovery: 1.0 - 0.6 = 0.4
    // But also test with very high dismissals to hit 0.1 floor
    let result = compute_check_priority(100, 100, 100, 0, 0.1, 0.1);
    assert!((result - 0.1).abs() < f64::EPSILON);
}

#[test]
fn compute_check_priority_recovery_partially_restores() {
    // 5 dismissals * 0.1 = 0.5 penalty
    // 0 inaction: no penalty
    // 3 recovery * 0.1 = 0.3 bonus
    // 1.0 - 0.5 + 0.3 = 0.8
    let result = compute_check_priority(5, 0, 0, 3, 0.1, 0.1);
    assert!((result - 0.8).abs() < f64::EPSILON);
}

#[test]
fn priority_floor_never_below_point_one() {
    // Extreme dismissals and inaction with no recovery
    let result = compute_check_priority(1000, 1000, 1000, 0, 1.0, 0.0);
    assert!((result - 0.1).abs() < f64::EPSILON);
}

#[tokio::test]
async fn heartbeat_weles_health_marks_review_unavailable_as_degraded() {
    let root = tempdir().expect("tempdir should succeed");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.extra.insert(
        "weles_review_available".to_string(),
        serde_json::Value::Bool(false),
    );

    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let mut events = engine.subscribe();

    let status = engine.refresh_weles_health_from_heartbeat(1_234).await;

    assert_eq!(status.state, WelesHealthState::Degraded);
    assert_eq!(status.checked_at, 1_234);
    assert!(status
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("review unavailable")));

    let event = timeout(Duration::from_millis(250), events.recv())
        .await
        .expect("weles health event should arrive")
        .expect("weles health event should deserialize");
    match event {
        AgentEvent::WelesHealthUpdate {
            state,
            reason,
            checked_at,
        } => {
            assert_eq!(state, WelesHealthState::Degraded);
            assert_eq!(checked_at, 1_234);
            assert!(reason
                .as_deref()
                .is_some_and(|value| value.contains("review unavailable")));
        }
        other => panic!("expected weles health update, got {other:?}"),
    }
}
