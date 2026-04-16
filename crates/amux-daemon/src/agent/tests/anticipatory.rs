use super::*;
use crate::history::IntentPredictionRow;
use crate::session_manager::SessionManager;
use tempfile::tempdir;
use tokio::time::{timeout, Duration};

fn sample_task(id: &str, thread_id: Option<&str>, goal_run_id: Option<&str>) -> AgentTask {
    AgentTask {
        id: id.to_string(),
        title: id.to_string(),
        description: String::new(),
        status: TaskStatus::Queued,
        priority: TaskPriority::Normal,
        progress: 0,
        created_at: 0,
        started_at: None,
        completed_at: None,
        error: None,
        result: None,
        thread_id: thread_id.map(str::to_string),
        source: "user".to_string(),
        notify_on_complete: false,
        notify_channels: Vec::new(),
        dependencies: Vec::new(),
        command: None,
        session_id: None,
        goal_run_id: goal_run_id.map(str::to_string),
        goal_run_title: None,
        goal_step_id: None,
        goal_step_title: None,
        parent_task_id: None,
        parent_thread_id: None,
        runtime: "daemon".to_string(),
        retry_count: 0,
        max_retries: 3,
        next_retry_at: None,
        scheduled_at: None,
        blocked_reason: None,
        awaiting_approval_id: None,
        policy_fingerprint: None,
        approval_expires_at: None,
        containment_scope: None,
        compensation_status: None,
        compensation_summary: None,
        lane_id: None,
        last_error: None,
        logs: Vec::new(),
        tool_whitelist: None,
        tool_blacklist: None,
        context_budget_tokens: None,
        context_overflow_action: None,
        termination_conditions: None,
        success_criteria: None,
        max_duration_secs: None,
        supervisor_config: None,
        override_provider: None,
        override_model: None,
        override_system_prompt: None,
        sub_agent_def_id: None,
    }
}

fn sample_goal_run(id: &str, thread_id: Option<&str>) -> GoalRun {
    GoalRun {
        id: id.to_string(),
        title: id.to_string(),
        goal: String::new(),
        client_request_id: None,
        status: GoalRunStatus::Queued,
        priority: TaskPriority::Normal,
        created_at: 0,
        updated_at: 0,
        started_at: None,
        completed_at: None,
        thread_id: thread_id.map(str::to_string),
        session_id: None,
        current_step_index: 0,
        current_step_title: None,
        current_step_kind: None,
        replan_count: 0,
        max_replans: 3,
        plan_summary: None,
        reflection_summary: None,
        memory_updates: Vec::new(),
        generated_skill_path: None,
        last_error: None,
        failure_cause: None,
        child_task_ids: Vec::new(),
        child_task_count: 0,
        approval_count: 0,
        awaiting_approval_id: None,
        policy_fingerprint: None,
        approval_expires_at: None,
        containment_scope: None,
        compensation_status: None,
        compensation_summary: None,
        active_task_id: None,
        duration_ms: None,
        steps: Vec::new(),
        events: Vec::new(),
        total_prompt_tokens: 0,
        total_completion_tokens: 0,
        estimated_cost_usd: None,
        autonomy_level: Default::default(),
        authorship_tag: None,
    }
}

#[test]
fn circular_hour_distance_wraps_across_midnight() {
    assert_eq!(circular_hour_distance(23, 1), 2);
    assert_eq!(circular_hour_distance(5, 5), 0);
}

#[test]
fn truncate_hint_shortens_long_strings() {
    let value = "x".repeat(160);
    let shortened = truncate_hint(&value);
    assert!(shortened.len() < value.len());
    assert!(shortened.ends_with('…'));
}

#[test]
fn attention_surface_suppresses_briefs_in_settings() {
    assert!(!should_surface_anticipatory_kind(
        "morning_brief",
        Some("modal:settings:provider")
    ));
    assert!(!should_surface_anticipatory_kind(
        "stuck_hint",
        Some("modal:approval")
    ));
}

#[test]
fn attention_surface_allows_task_and_conversation_contexts() {
    assert!(should_surface_anticipatory_kind(
        "morning_brief",
        Some("conversation:chat")
    ));
    assert!(should_surface_anticipatory_kind(
        "stuck_hint",
        Some("task:detail")
    ));
}

#[test]
fn task_attention_priority_prefers_goal_then_thread() {
    let attention = AttentionFocus {
        thread_id: Some("thread_1".to_string()),
        goal_run_id: Some("goal_1".to_string()),
    };
    let goal_match = sample_task("task_goal", Some("thread_9"), Some("goal_1"));
    let thread_match = sample_task("task_thread", Some("thread_1"), Some("goal_9"));

    assert_eq!(task_attention_priority(&goal_match, &attention), 2);
    assert_eq!(task_attention_priority(&thread_match, &attention), 1);
}

#[test]
fn goal_attention_priority_prefers_exact_goal_match() {
    let attention = AttentionFocus {
        thread_id: Some("thread_1".to_string()),
        goal_run_id: Some("goal_1".to_string()),
    };
    let exact = sample_goal_run("goal_1", Some("thread_2"));
    let thread_only = sample_goal_run("goal_2", Some("thread_1"));

    assert_eq!(goal_attention_priority(&exact, &attention), 2);
    assert_eq!(goal_attention_priority(&thread_only, &attention), 1);
}

#[tokio::test]
async fn anticipatory_tick_ignores_weles_owned_stuck_tasks() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.stuck_detection = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let mut weles_task = sample_task("task-weles", Some("thread-weles"), None);
    weles_task.title = "WELES".to_string();
    weles_task.status = TaskStatus::Blocked;
    weles_task.sub_agent_def_id = Some("weles_builtin".to_string());
    weles_task.blocked_reason = Some("waiting for lane availability: daemon-main".to_string());
    weles_task.started_at = Some(1);

    engine.tasks.lock().await.push_back(weles_task);
    engine.run_anticipatory_tick().await;

    assert!(
        engine.anticipatory.read().await.items.is_empty(),
        "WELES-owned blocked tasks should not surface as anticipatory hints"
    );
}

#[tokio::test]
async fn session_start_prewarm_hydrates_active_attention_thread() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.morning_brief = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_attention("conversation:chat", Some("thread-prewarm"), None)
        .await
        .unwrap();
    engine.mark_operator_present("test").await;
    engine.run_anticipatory_tick().await;

    assert!(
        engine
            .anticipatory
            .read()
            .await
            .hydration_by_thread
            .contains_key("thread-prewarm"),
        "session-start prewarm should hydrate the active attention thread before surfacing items"
    );
}

#[tokio::test]
async fn anticipatory_tick_routes_stuck_hint_to_thread_surface_with_idle_signal() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.stuck_detection = true;
    config.anticipatory.stuck_detection_delay_seconds = 1;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let mut events = engine.subscribe();

    let now = now_millis();
    let mut stale_task = sample_task("task-stale", Some("thread-surface"), None);
    stale_task.title = "Review failing command".to_string();
    stale_task.status = TaskStatus::InProgress;
    stale_task.started_at = Some(now.saturating_sub(30_000));
    stale_task.last_error = Some("command timed out while waiting for output".to_string());
    engine.tasks.lock().await.push_back(stale_task);

    engine
        .set_thread_client_surface("thread-surface", amux_protocol::ClientSurface::Electron)
        .await;
    engine
        .record_operator_attention("task:detail", Some("thread-surface"), None)
        .await
        .unwrap();
    {
        let mut runtime = engine.anticipatory.write().await;
        runtime.last_presence_at = Some(now.saturating_sub(60_000));
        runtime.active_attention_updated_at = Some(now.saturating_sub(60_000));
    }

    engine.run_anticipatory_tick().await;

    let items = engine.anticipatory.read().await.items.clone();
    let item = items
        .into_iter()
        .find(|candidate| candidate.kind == "stuck_hint")
        .expect("expected a stuck hint for the stale task");
    assert_eq!(item.preferred_client_surface.as_deref(), Some("electron"));
    assert_eq!(
        item.preferred_attention_surface.as_deref(),
        Some("task:detail")
    );
    assert!(
        item.bullets
            .iter()
            .any(|bullet| bullet.contains("Operator attention has been idle")),
        "idle-aware heuristics should be surfaced in the stuck hint bullets"
    );

    let notice = timeout(Duration::from_millis(250), async {
        loop {
            match events.recv().await {
                Ok(AgentEvent::WorkflowNotice {
                    kind,
                    thread_id,
                    message,
                    details,
                }) => {
                    break (kind, thread_id, message, details);
                }
                Ok(_) => continue,
                Err(error) => panic!("expected workflow notice, got event error: {error}"),
            }
        }
    })
    .await
    .expect("thread-targeted anticipatory notice should arrive");

    assert_eq!(notice.0, "anticipatory");
    assert_eq!(notice.1, "thread-surface");
    assert!(notice.2.contains("Task May Be Stuck"));
    assert!(notice
        .3
        .as_deref()
        .is_some_and(|details| details.contains("Operator attention has been idle")));
}

#[tokio::test]
async fn morning_brief_inherits_route_from_top_goal_surface() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.morning_brief = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let now = now_millis();
    let mut goal = sample_goal_run("goal-route", Some("thread-route"));
    goal.title = "Resume release work".to_string();
    goal.status = GoalRunStatus::Running;
    goal.updated_at = now;
    goal.current_step_title = Some("publish the package".to_string());
    engine.goal_runs.lock().await.push_back(goal);
    engine
        .set_goal_run_client_surface("goal-route", amux_protocol::ClientSurface::Tui)
        .await;
    engine.mark_operator_present("test").await;

    engine.run_anticipatory_tick().await;

    let items = engine.anticipatory.read().await.items.clone();
    let item = items
        .into_iter()
        .find(|candidate| candidate.kind == "morning_brief")
        .expect("expected a morning brief");
    assert_eq!(item.preferred_client_surface.as_deref(), Some("tui"));
}

#[tokio::test]
async fn strained_satisfaction_suppresses_optional_morning_brief() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.morning_brief = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let now = now_millis();
    let mut goal = sample_goal_run("goal-strained", Some("thread-strained"));
    goal.title = "Resume degraded flow".to_string();
    goal.status = GoalRunStatus::Running;
    goal.updated_at = now;
    goal.current_step_title = Some("retry the command".to_string());
    engine.goal_runs.lock().await.push_back(goal);
    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.operator_satisfaction.score = 0.22;
        model.operator_satisfaction.label = "strained".to_string();
    }
    engine.mark_operator_present("test").await;

    engine.run_anticipatory_tick().await;

    assert!(
        engine
            .anticipatory
            .read()
            .await
            .items
            .iter()
            .all(|item| item.kind != "morning_brief"),
        "strained satisfaction should suppress optional morning briefs to reduce proactive noise"
    );
}

#[tokio::test]
async fn tool_hesitation_suppresses_optional_morning_brief() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.morning_brief = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let now = now_millis();
    let mut goal = sample_goal_run("goal-hesitation-brief", Some("thread-hesitation-brief"));
    goal.title = "Resume degraded flow".to_string();
    goal.status = GoalRunStatus::Running;
    goal.updated_at = now;
    goal.current_step_title = Some("retry the command".to_string());
    engine.goal_runs.lock().await.push_back(goal);
    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.implicit_feedback.tool_hesitation_count = 1;
        refresh_operator_satisfaction(&mut model);
    }
    engine.mark_operator_present("test").await;

    engine.run_anticipatory_tick().await;

    assert!(
        engine
            .anticipatory
            .read()
            .await
            .items
            .iter()
            .all(|item| item.kind != "morning_brief"),
        "tool hesitation should suppress optional morning briefs to reduce proactive noise"
    );
}

#[tokio::test]
async fn slow_approval_latency_suppresses_optional_intent_prediction() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.stuck_detection = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let mut task = sample_task("task-latency-intent", Some("thread-latency-intent"), None);
    task.title = "Need approval".to_string();
    task.status = TaskStatus::AwaitingApproval;
    engine.tasks.lock().await.push_back(task);
    engine
        .record_operator_attention("conversation:chat", Some("thread-latency-intent"), None)
        .await
        .unwrap();
    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.risk_fingerprint.approval_requests = 4;
        model.risk_fingerprint.approvals = 2;
        model.risk_fingerprint.denials = 2;
        model.risk_fingerprint.avg_response_time_secs = 45.0;
        refresh_risk_metrics(&mut model.risk_fingerprint);
        refresh_operator_satisfaction(&mut model);
    }

    engine.run_anticipatory_tick().await;

    assert!(
        engine
            .anticipatory
            .read()
            .await
            .items
            .iter()
            .all(|item| item.kind != "intent_prediction"),
        "slow approval latency should suppress optional intent prediction to reduce proactive noise"
    );
}

#[tokio::test]
async fn strained_satisfaction_skips_predictive_hydration() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.predictive_hydration = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let now = now_millis();
    let mut goal = sample_goal_run("goal-hydration", Some("thread-hydration"));
    goal.status = GoalRunStatus::Running;
    goal.updated_at = now;
    engine.goal_runs.lock().await.push_back(goal);
    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.operator_satisfaction.score = 0.24;
        model.operator_satisfaction.label = "strained".to_string();
    }

    engine.run_anticipatory_tick().await;

    assert!(
        !engine
            .anticipatory
            .read()
            .await
            .hydration_by_thread
            .contains_key("thread-hydration"),
        "strained satisfaction should skip predictive hydration so the daemon reduces background churn"
    );
}

#[tokio::test]
async fn anticipatory_tick_surfaces_intent_prediction_for_pending_approval() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.stuck_detection = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let mut task = sample_task("task-approval", Some("thread-intent"), None);
    task.title = "Need approval".to_string();
    task.status = TaskStatus::AwaitingApproval;
    engine.tasks.lock().await.push_back(task);
    engine
        .record_operator_attention("conversation:chat", Some("thread-intent"), None)
        .await
        .unwrap();

    engine.run_anticipatory_tick().await;

    let items = engine.anticipatory.read().await.items.clone();
    let item = items
        .into_iter()
        .find(|candidate| candidate.kind == "intent_prediction")
        .expect("expected an intent prediction item");
    let payload = item
        .intent_prediction
        .as_ref()
        .expect("intent prediction payload should be present");
    assert_eq!(payload.primary_action, "review pending approval");
    assert_eq!(
        payload.ranked_actions.len(),
        3,
        "intent prediction should surface ranked next actions"
    );
    assert_eq!(payload.ranked_actions[0].rank, 1);
    assert_eq!(payload.ranked_actions[0].action, "review pending approval");
    assert!(payload.ranked_actions[0].confidence >= 0.86);
    assert_eq!(item.thread_id.as_deref(), Some("thread-intent"));
    assert!(item.summary.contains("review pending approval"));
    assert!(item.confidence >= 0.86);
}

#[tokio::test]
async fn anticipatory_tick_surfaces_intent_prediction_for_repo_change_context() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_attention("conversation:chat", Some("thread-repo-intent"), None)
        .await
        .unwrap();
    engine.thread_work_contexts.write().await.insert(
        "thread-repo-intent".to_string(),
        ThreadWorkContext {
            thread_id: "thread-repo-intent".to_string(),
            entries: vec![WorkContextEntry {
                path: "src/main.rs".to_string(),
                previous_path: None,
                kind: WorkContextEntryKind::RepoChange,
                source: "repo_scan".to_string(),
                change_kind: Some("modified".to_string()),
                repo_root: Some("/tmp/repo".to_string()),
                goal_run_id: None,
                step_index: None,
                session_id: None,
                is_text: true,
                updated_at: now_millis(),
            }],
        },
    );

    engine.run_anticipatory_tick().await;

    let items = engine.anticipatory.read().await.items.clone();
    let item = items
        .into_iter()
        .find(|candidate| candidate.kind == "intent_prediction")
        .expect("expected an intent prediction item");
    let payload = item
        .intent_prediction
        .as_ref()
        .expect("intent prediction payload should be present");
    assert_eq!(
        payload.primary_action,
        "inspect or test recent repo changes"
    );
    assert_eq!(
        payload.ranked_actions.len(),
        3,
        "intent prediction should surface ranked next actions"
    );
    assert_eq!(payload.ranked_actions[0].rank, 1);
    assert_eq!(
        payload.ranked_actions[0].action,
        "inspect or test recent repo changes"
    );
    assert_eq!(item.thread_id.as_deref(), Some("thread-repo-intent"));
    assert!(item.summary.contains("inspect or test recent repo changes"));
    assert!(item
        .bullets
        .iter()
        .any(|bullet| bullet.contains("repo-linked")));
}

#[tokio::test]
async fn strained_satisfaction_suppresses_intent_prediction() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_attention("conversation:chat", Some("thread-strained-intent"), None)
        .await
        .unwrap();
    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.operator_satisfaction.score = 0.20;
        model.operator_satisfaction.label = "strained".to_string();
    }

    engine.run_anticipatory_tick().await;

    assert!(engine
        .anticipatory
        .read()
        .await
        .items
        .iter()
        .all(|item| item.kind != "intent_prediction"));
}

#[tokio::test]
async fn tool_hesitation_tightens_predictive_hydration_to_active_attention_thread() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.predictive_hydration = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_attention("conversation:chat", Some("thread-focus"), None)
        .await
        .unwrap();

    let now = now_millis();
    let mut focused_goal = sample_goal_run("goal-focus", Some("thread-focus"));
    focused_goal.status = GoalRunStatus::Running;
    focused_goal.updated_at = now;
    engine.goal_runs.lock().await.push_back(focused_goal);

    let mut other_goal = sample_goal_run("goal-other", Some("thread-other"));
    other_goal.status = GoalRunStatus::Running;
    other_goal.updated_at = now.saturating_sub(1_000);
    engine.goal_runs.lock().await.push_back(other_goal);

    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.implicit_feedback.tool_hesitation_count = 1;
        refresh_operator_satisfaction(&mut model);
    }

    engine.run_anticipatory_tick().await;

    let hydration = engine.anticipatory.read().await.hydration_by_thread.clone();
    assert!(
        hydration.contains_key("thread-focus"),
        "active attention thread should still be hydrated"
    );
    assert!(
        !hydration.contains_key("thread-other"),
        "tool hesitation should tighten predictive hydration to the active attention thread"
    );
}

#[tokio::test]
async fn slow_approval_latency_tightens_predictive_hydration_to_active_attention_thread() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.predictive_hydration = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_attention("conversation:chat", Some("thread-focus"), None)
        .await
        .unwrap();

    let now = now_millis();
    let mut focused_goal = sample_goal_run("goal-focus", Some("thread-focus"));
    focused_goal.status = GoalRunStatus::Running;
    focused_goal.updated_at = now;
    engine.goal_runs.lock().await.push_back(focused_goal);

    let mut other_goal = sample_goal_run("goal-other", Some("thread-other"));
    other_goal.status = GoalRunStatus::Running;
    other_goal.updated_at = now.saturating_sub(1_000);
    engine.goal_runs.lock().await.push_back(other_goal);

    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.risk_fingerprint.approval_requests = 4;
        model.risk_fingerprint.approvals = 2;
        model.risk_fingerprint.denials = 2;
        model.risk_fingerprint.avg_response_time_secs = 45.0;
        refresh_risk_metrics(&mut model.risk_fingerprint);
        refresh_operator_satisfaction(&mut model);
    }

    engine.run_anticipatory_tick().await;

    let hydration = engine.anticipatory.read().await.hydration_by_thread.clone();
    assert!(
        hydration.contains_key("thread-focus"),
        "active attention thread should still be hydrated"
    );
    assert!(
        !hydration.contains_key("thread-other"),
        "slow approval latency should tighten predictive hydration to the active attention thread"
    );
}

#[tokio::test]
async fn predictive_hydration_populates_prewarm_cache_for_hydrated_thread() {
    let root = tempdir().unwrap();
    let repo_root = root.path().join("repo-predictive-cache");
    std::fs::create_dir_all(&repo_root).unwrap();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repo_root)
        .output()
        .expect("git init");
    std::fs::write(
        repo_root.join("Cargo.toml"),
        "[package]\nname='demo'\nversion='0.1.0'\n",
    )
    .unwrap();

    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.anticipatory.predictive_hydration = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let mut goal = sample_goal_run("goal-cache", Some("thread-cache"));
    goal.status = GoalRunStatus::Running;
    goal.updated_at = now_millis();
    engine.goal_runs.lock().await.push_back(goal);
    engine.thread_work_contexts.write().await.insert(
        "thread-cache".to_string(),
        ThreadWorkContext {
            thread_id: "thread-cache".to_string(),
            entries: vec![WorkContextEntry {
                path: "Cargo.toml".to_string(),
                previous_path: None,
                kind: WorkContextEntryKind::RepoChange,
                source: "repo_scan".to_string(),
                change_kind: Some("modified".to_string()),
                repo_root: Some(repo_root.to_string_lossy().to_string()),
                goal_run_id: None,
                step_index: None,
                session_id: None,
                is_text: true,
                updated_at: now_millis(),
            }],
        },
    );

    engine.run_anticipatory_tick().await;

    let runtime = engine.anticipatory.read().await;
    let snapshot = runtime
        .prewarm_cache_by_thread
        .get("thread-cache")
        .expect("prewarm cache snapshot for hydrated thread");
    assert!(snapshot.summary.contains("branch"));
    assert!(snapshot.summary.contains("context entries 1"));
}

#[tokio::test]
async fn resolved_intent_predictions_raise_future_prediction_confidence() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_attention("conversation:chat", Some("thread-confidence"), None)
        .await
        .unwrap();

    let mut task = sample_task("task-confidence", Some("thread-confidence"), None);
    task.title = "Need approval".to_string();
    task.status = TaskStatus::AwaitingApproval;
    engine.tasks.lock().await.push_back(task);

    for (id, was_correct, created_at_ms) in [
        ("intent-hist-1", true, 100),
        ("intent-hist-2", true, 200),
        ("intent-hist-3", false, 300),
    ] {
        engine
            .history
            .insert_intent_prediction(&IntentPredictionRow {
                id: id.to_string(),
                session_id: "thread-other".to_string(),
                context_state_hash: format!("ctx-{id}"),
                predicted_action: "review pending approval".to_string(),
                confidence: 0.80,
                actual_action: Some("review pending approval".to_string()),
                was_correct: Some(was_correct),
                created_at_ms,
            })
            .await
            .expect("seed resolved prediction history");
    }

    engine.run_anticipatory_tick().await;

    let item = engine
        .anticipatory
        .read()
        .await
        .items
        .clone()
        .into_iter()
        .find(|candidate| candidate.kind == "intent_prediction")
        .expect("expected intent prediction item");
    let payload = item
        .intent_prediction
        .as_ref()
        .expect("intent prediction payload should be present");

    assert!(
        payload.confidence > 0.86,
        "persisted success-rate priors should raise confidence above the raw heuristic baseline"
    );
    assert_eq!(payload.primary_action, "review pending approval");
}

#[tokio::test]
async fn intent_prediction_persists_and_resolves_when_operator_action_matches() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    config.operator_model.enabled = true;
    config.operator_model.allow_message_statistics = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let mut task = sample_task("task-approval-persist", Some("thread-intent-persist"), None);
    task.title = "Need approval".to_string();
    task.status = TaskStatus::AwaitingApproval;
    engine.tasks.lock().await.push_back(task);
    engine
        .record_operator_attention("conversation:chat", Some("thread-intent-persist"), None)
        .await
        .unwrap();

    engine.run_anticipatory_tick().await;

    let before = engine
        .history
        .list_intent_predictions("thread-intent-persist", 10)
        .await
        .expect("list persisted intent predictions before resolution");
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].predicted_action, "review pending approval");
    assert_eq!(before[0].was_correct, None);

    engine
        .record_operator_message(
            "thread-intent-persist",
            "please review the approval first",
            false,
        )
        .await
        .expect("record operator message");

    let after = engine
        .history
        .list_intent_predictions("thread-intent-persist", 10)
        .await
        .expect("list persisted intent predictions after resolution");
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].actual_action.as_deref(),
        Some("review pending approval")
    );
    assert_eq!(after[0].was_correct, Some(true));
}

#[tokio::test]
async fn system_outcome_foresight_persists_and_resolves_when_health_feedback_arrives() {
    let root = tempdir().unwrap();
    let repo_root = root.path().join("repo-foresight-persist");
    std::fs::create_dir_all(&repo_root).unwrap();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repo_root)
        .output()
        .expect("git init");
    std::fs::write(
        repo_root.join("Cargo.toml"),
        "[package]\nname='demo'\nversion='0.1.0'\n",
    )
    .unwrap();
    std::fs::create_dir_all(repo_root.join("src")).unwrap();
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn broken() {}\n").unwrap();

    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_attention("conversation:chat", Some("thread-foresight-persist"), None)
        .await
        .unwrap();
    engine.thread_work_contexts.write().await.insert(
        "thread-foresight-persist".to_string(),
        ThreadWorkContext {
            thread_id: "thread-foresight-persist".to_string(),
            entries: vec![WorkContextEntry {
                path: "src/lib.rs".to_string(),
                previous_path: None,
                kind: WorkContextEntryKind::RepoChange,
                source: "repo_scan".to_string(),
                change_kind: Some("modified".to_string()),
                repo_root: Some(repo_root.to_string_lossy().to_string()),
                goal_run_id: None,
                step_index: None,
                session_id: None,
                is_text: true,
                updated_at: now_millis(),
            }],
        },
    );
    engine
        .history
        .insert_health_log(
            "health-foresight-persist-degraded",
            "task",
            "cargo-test",
            "degraded",
            Some("{\"tool\":\"cargo test\",\"error\":\"Command failed\"}"),
            Some("recent cargo test failed in this repo"),
            now_millis(),
        )
        .await
        .expect("save degraded health log");

    engine.run_anticipatory_tick().await;

    let before = engine
        .history
        .list_system_outcome_predictions("thread-foresight-persist", 10)
        .await
        .expect("list persisted system outcome predictions before resolution");
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].prediction_type, "build_test_risk");
    assert_eq!(before[0].predicted_outcome, "build/test failure");
    assert!(before[0].confidence >= 0.7);
    assert_eq!(before[0].was_correct, None);

    engine
        .resolve_system_outcome_prediction_feedback("thread-foresight-persist", "healthy")
        .await;

    let after = engine
        .history
        .list_system_outcome_predictions("thread-foresight-persist", 10)
        .await
        .expect("list persisted system outcome predictions after resolution");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].actual_outcome.as_deref(), Some("healthy"));
    assert_eq!(after[0].was_correct, Some(false));
}

#[tokio::test]
async fn intent_prediction_includes_cached_prewarm_summary_when_available() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_attention("conversation:chat", Some("thread-cache-bullets"), None)
        .await
        .unwrap();
    engine.thread_work_contexts.write().await.insert(
        "thread-cache-bullets".to_string(),
        ThreadWorkContext {
            thread_id: "thread-cache-bullets".to_string(),
            entries: vec![WorkContextEntry {
                path: "src/main.rs".to_string(),
                previous_path: None,
                kind: WorkContextEntryKind::RepoChange,
                source: "repo_scan".to_string(),
                change_kind: Some("modified".to_string()),
                repo_root: Some("/tmp/repo".to_string()),
                goal_run_id: None,
                step_index: None,
                session_id: None,
                is_text: true,
                updated_at: now_millis(),
            }],
        },
    );
    engine.anticipatory.write().await.prewarm_cache_by_thread.insert(
        "thread-cache-bullets".to_string(),
        AnticipatoryPrewarmSnapshot {
            summary: "branch main; dirty=true; modified 1; staged 0; untracked 0; ahead 0; behind 0; context entries 1".to_string(),
        },
    );

    engine.run_anticipatory_tick().await;

    let items = engine.anticipatory.read().await.items.clone();
    let item = items
        .into_iter()
        .find(|candidate| candidate.kind == "intent_prediction")
        .expect("expected an intent prediction item");
    let payload = item
        .intent_prediction
        .as_ref()
        .expect("intent prediction payload should be present");
    assert!(payload
        .ranked_actions
        .iter()
        .any(|candidate| candidate.rationale.contains("Cached prewarm")));
    assert!(item
        .bullets
        .iter()
        .any(|bullet| bullet.contains("Cached prewarm:")));
}

#[tokio::test]
async fn anticipatory_tick_surfaces_persisted_system_outcome_foresight_for_build_risk() {
    let root = tempdir().unwrap();
    let repo_root = root.path().join("repo-build-risk");
    std::fs::create_dir_all(&repo_root).unwrap();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repo_root)
        .output()
        .expect("git init");
    std::fs::write(
        repo_root.join("Cargo.toml"),
        "[package]\nname='demo'\nversion='0.1.0'\n",
    )
    .unwrap();
    std::fs::create_dir_all(repo_root.join("src")).unwrap();

    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_attention("conversation:chat", Some("thread-build-risk"), None)
        .await
        .unwrap();
    engine.thread_work_contexts.write().await.insert(
        "thread-build-risk".to_string(),
        ThreadWorkContext {
            thread_id: "thread-build-risk".to_string(),
            entries: vec![WorkContextEntry {
                path: "src/lib.rs".to_string(),
                previous_path: None,
                kind: WorkContextEntryKind::RepoChange,
                source: "repo_scan".to_string(),
                change_kind: Some("modified".to_string()),
                repo_root: Some(repo_root.to_string_lossy().to_string()),
                goal_run_id: None,
                step_index: None,
                session_id: None,
                is_text: true,
                updated_at: now_millis(),
            }],
        },
    );
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn broken() {}\n").unwrap();
    engine
        .history
        .insert_health_log(
            "health-build-risk",
            "task",
            "cargo-test",
            "degraded",
            Some("{\"tool\":\"cargo test\",\"error\":\"Command failed\"}"),
            Some("recent cargo test failed in this repo"),
            now_millis(),
        )
        .await
        .expect("save health log");

    engine.run_anticipatory_tick().await;

    let items = engine.anticipatory.read().await.items.clone();
    let item = items
        .into_iter()
        .find(|candidate| candidate.kind == "system_outcome_foresight")
        .expect("expected a system-outcome foresight item");
    assert_eq!(item.thread_id.as_deref(), Some("thread-build-risk"));
    assert!(item.summary.contains("build/test failure risk"));
    assert!(item.confidence >= 0.7);
    assert!(item
        .bullets
        .iter()
        .any(|bullet| bullet.contains("prediction_type=build_test_risk")));
    assert!(item
        .bullets
        .iter()
        .any(|bullet| bullet.contains("recent cargo test failed")));
    assert!(item
        .bullets
        .iter()
        .any(|bullet| bullet.contains("dirty repo state")));
}

#[tokio::test]
async fn build_test_risk_confidence_increases_with_repeated_degraded_health_entries() {
    let root = tempdir().unwrap();
    let repo_root = root.path().join("repo-build-confidence");
    std::fs::create_dir_all(&repo_root).unwrap();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&repo_root)
        .output()
        .expect("git init");
    std::fs::write(
        repo_root.join("Cargo.toml"),
        "[package]\nname='demo'\nversion='0.1.0'\n",
    )
    .unwrap();
    std::fs::create_dir_all(repo_root.join("src")).unwrap();
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn broken() {}\n").unwrap();

    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_attention("conversation:chat", Some("thread-build-confidence"), None)
        .await
        .unwrap();
    engine.thread_work_contexts.write().await.insert(
        "thread-build-confidence".to_string(),
        ThreadWorkContext {
            thread_id: "thread-build-confidence".to_string(),
            entries: vec![WorkContextEntry {
                path: "src/lib.rs".to_string(),
                previous_path: None,
                kind: WorkContextEntryKind::RepoChange,
                source: "repo_scan".to_string(),
                change_kind: Some("modified".to_string()),
                repo_root: Some(repo_root.to_string_lossy().to_string()),
                goal_run_id: None,
                step_index: None,
                session_id: None,
                is_text: true,
                updated_at: now_millis(),
            }],
        },
    );

    engine
        .history
        .insert_health_log(
            "health-build-confidence-1",
            "task",
            "cargo-test",
            "degraded",
            Some("{\"tool\":\"cargo test\",\"error\":\"Command failed\"}"),
            Some("recent cargo test failed in this repo"),
            now_millis() - 2_000,
        )
        .await
        .expect("save first health log");
    let settings = engine.config.read().await.anticipatory.clone();
    let single_confidence = engine
        .compute_system_outcome_foresight(&settings)
        .await
        .map(|item| item.confidence)
        .expect("expected foresight item after one degraded health entry");

    engine
        .history
        .insert_health_log(
            "health-build-confidence-2",
            "task",
            "cargo-test",
            "degraded",
            Some("{\"tool\":\"cargo test\",\"error\":\"Command failed\"}"),
            Some("recent cargo test failed in this repo"),
            now_millis() - 1_000,
        )
        .await
        .expect("save second health log");
    let repeated_confidence = engine
        .compute_system_outcome_foresight(&settings)
        .await
        .map(|item| item.confidence)
        .expect("expected foresight item after repeated degraded health entries");

    assert!((0.0..=1.0).contains(&single_confidence));
    assert!((0.0..=1.0).contains(&repeated_confidence));
    assert!(
        repeated_confidence > single_confidence,
        "expected repeated degraded build/test health evidence to raise foresight confidence"
    );
}

#[tokio::test]
async fn anticipatory_tick_surfaces_stale_context_foresight_when_hydration_lags_session_rhythm() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.anticipatory.enabled = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_attention("conversation:chat", Some("thread-stale-context"), None)
        .await
        .unwrap();
    engine.thread_work_contexts.write().await.insert(
        "thread-stale-context".to_string(),
        ThreadWorkContext {
            thread_id: "thread-stale-context".to_string(),
            entries: vec![WorkContextEntry {
                path: "src/lib.rs".to_string(),
                previous_path: None,
                kind: WorkContextEntryKind::RepoChange,
                source: "repo_scan".to_string(),
                change_kind: Some("modified".to_string()),
                repo_root: Some("/tmp/repo".to_string()),
                goal_run_id: None,
                step_index: None,
                session_id: None,
                is_text: true,
                updated_at: now_millis(),
            }],
        },
    );
    {
        let mut runtime = engine.anticipatory.write().await;
        runtime.hydration_by_thread.insert(
            "thread-stale-context".to_string(),
            now_millis() - 16 * 60 * 1000,
        );
    }
    {
        let mut model = engine.operator_model.write().await;
        model.session_rhythm.session_count = 6;
        model.session_rhythm.session_duration_avg_minutes = 10.0;
        model.session_rhythm.typical_start_hour_utc = Some(9);
    }

    engine
        .history
        .create_thread(&amux_protocol::AgentDbThread {
            id: "thread-stale-context".to_string(),
            workspace_id: None,
            surface_id: None,
            pane_id: None,
            agent_name: Some(MAIN_AGENT_NAME.to_string()),
            title: "Stale context thread".to_string(),
            created_at: 1,
            updated_at: now_millis() as i64,
            message_count: 2,
            total_tokens: 0,
            last_preview: "off-topic drift".to_string(),
            metadata_json: None,
        })
        .await
        .expect("seed thread row");
    engine
        .history
        .add_message(&amux_protocol::AgentDbMessage {
            id: "stale-user-1".to_string(),
            thread_id: "thread-stale-context".to_string(),
            created_at: (now_millis() - 2_000) as i64,
            role: "user".to_string(),
            content: "Let's switch topics completely and talk about vacation photos.".to_string(),
            provider: None,
            model: None,
            input_tokens: Some(0),
            output_tokens: Some(0),
            total_tokens: Some(0),
            cost_usd: None,
            reasoning: None,
            tool_calls_json: None,
            metadata_json: None,
        })
        .await
        .expect("seed user message");
    engine
        .history
        .add_message(&amux_protocol::AgentDbMessage {
            id: "stale-assistant-1".to_string(),
            thread_id: "thread-stale-context".to_string(),
            created_at: (now_millis() - 1_000) as i64,
            role: "assistant".to_string(),
            content: "The recent repo context may be stale relative to the current conversation."
                .to_string(),
            provider: None,
            model: None,
            input_tokens: Some(0),
            output_tokens: Some(0),
            total_tokens: Some(0),
            cost_usd: None,
            reasoning: None,
            tool_calls_json: None,
            metadata_json: None,
        })
        .await
        .expect("seed assistant message");

    engine.run_anticipatory_tick().await;

    let items = engine.anticipatory.read().await.items.clone();
    let item = items
        .into_iter()
        .find(|candidate| candidate.kind == "system_outcome_foresight")
        .expect("expected a system-outcome foresight item");
    assert_eq!(item.thread_id.as_deref(), Some("thread-stale-context"));
    assert!(item.summary.contains("stale context"));
    assert!(item.confidence >= 0.7);
    assert!(item
        .bullets
        .iter()
        .any(|bullet| bullet.contains("prediction_type=stale_context")));
    assert!(item
        .bullets
        .iter()
        .any(|bullet| bullet.contains("hydration age")));
    assert!(item
        .bullets
        .iter()
        .any(|bullet| bullet.contains("semantic alignment degraded")));
}
