use super::*;
use tempfile::tempdir;

#[test]
fn cognitive_style_prefers_terse_for_short_messages() {
    assert_eq!(
        verbosity_preference_for_length(6.0),
        VerbosityPreference::Terse
    );
    assert_eq!(reading_depth_for_length(6.0), ReadingDepth::Skim);
}

#[test]
fn risk_metrics_compute_category_rates_and_tolerance() {
    let mut risk = RiskFingerprint {
        approvals: 4,
        denials: 1,
        category_requests: HashMap::from([
            ("git".to_string(), 2),
            ("network_request".to_string(), 3),
        ]),
        category_approvals: HashMap::from([
            ("git".to_string(), 2),
            ("network_request".to_string(), 2),
        ]),
        ..RiskFingerprint::default()
    };

    refresh_risk_metrics(&mut risk);

    assert_eq!(risk.risk_tolerance, RiskTolerance::Aggressive);
    assert_eq!(
        risk.approval_rate_by_category.get("git").copied(),
        Some(1.0)
    );
    assert_eq!(
        risk.approval_rate_by_category
            .get("network_request")
            .copied(),
        Some(2.0 / 3.0)
    );
}

#[test]
fn classify_command_category_uses_command_heuristics_first() {
    assert_eq!(
        classify_command_category("rm -rf target", "highest"),
        "destructive_delete"
    );
    assert_eq!(
        classify_command_category("curl https://example.com", "moderate"),
        "network_request"
    );
}

#[test]
fn normalize_attention_surface_keeps_supported_separators() {
    assert_eq!(
        normalize_attention_surface(" modal:settings:SubAgents "),
        "modal:settings:subagents"
    );
}

#[test]
fn top_keys_orders_by_count_then_name() {
    let mut histogram = HashMap::new();
    histogram.insert("conversation:chat".to_string(), 4);
    histogram.insert("conversation:input".to_string(), 4);
    histogram.insert("modal:settings:provider".to_string(), 1);

    assert_eq!(
        top_keys(&histogram, 2),
        vec![
            "conversation:chat".to_string(),
            "conversation:input".to_string()
        ]
    );
}

#[test]
fn detect_revision_signal_finds_corrections() {
    assert_eq!(
        detect_revision_signal("Actually, use ripgrep instead."),
        RevisionSignal::Correction
    );
    assert_eq!(
        detect_revision_signal("Next time prefer the shorter answer."),
        RevisionSignal::Revision
    );
    assert_eq!(
        detect_revision_signal("Please run tests."),
        RevisionSignal::None
    );
}

#[test]
fn detect_reading_signal_finds_summary_and_reasoning_preferences() {
    assert_eq!(
        detect_reading_signal("Give me the tl;dr first."),
        ReadingSignal::SummaryRequest
    );
    assert_eq!(
        detect_reading_signal("Just the answer, no reasoning."),
        ReadingSignal::SkipReasoning
    );
    assert_eq!(
        detect_reading_signal("Show me the full trace and walk through it step by step."),
        ReadingSignal::DeepDetailRequest
    );
    assert_eq!(
        detect_reading_signal("Please run tests."),
        ReadingSignal::None
    );
}

#[test]
fn reading_depth_uses_behavioral_signals_not_just_message_length() {
    assert_eq!(reading_depth_for_profile(18.0, 3, 0, 2), ReadingDepth::Skim);
    assert_eq!(
        reading_depth_for_profile(8.0, 0, 3, 0),
        ReadingDepth::Standard
    );
}

#[test]
fn ema_update_basic_calculation() {
    let result = ema_update(10.0, 20.0, 0.3);
    assert!((result - 13.0).abs() < f64::EPSILON);
}

#[test]
fn ema_update_converges_toward_sample() {
    let mut value = 0.0;
    for _ in 0..50 {
        value = ema_update(value, 100.0, 0.3);
    }
    assert!((value - 100.0).abs() < 0.01);
}

#[test]
fn smooth_activity_histogram_applies_ema_to_all_24_hours() {
    let current: HashMap<u8, f64> = HashMap::new();
    let mut observation: HashMap<u8, u64> = HashMap::new();
    observation.insert(9, 5);
    observation.insert(14, 3);

    let smoothed = smooth_activity_histogram(&current, &observation, 0.3);
    assert_eq!(smoothed.len(), 24);
    assert!((smoothed[&9] - 1.5).abs() < f64::EPSILON);
    assert!((smoothed[&14] - 0.9).abs() < f64::EPSILON);
    assert!((smoothed[&0]).abs() < f64::EPSILON);
}

#[test]
fn smooth_activity_histogram_decays_unobserved_hours() {
    let mut current: HashMap<u8, f64> = HashMap::new();
    current.insert(9, 10.0);
    let observation: HashMap<u8, u64> = HashMap::new();

    let smoothed = smooth_activity_histogram(&current, &observation, 0.3);
    assert!((smoothed[&9] - 7.0).abs() < f64::EPSILON);
}

#[test]
fn record_attention_event_tracks_dwell_and_rapid_switches() {
    let mut model = OperatorModel::default();
    record_attention_event(&mut model, "conversation:chat", 1_000);
    record_attention_event(&mut model, "modal:settings", 6_000);
    record_attention_event(&mut model, "conversation:chat", 10_000);
    record_attention_event(&mut model, "conversation:chat", 50_000);

    assert_eq!(model.attention_topology.focus_event_count, 4);
    assert_eq!(model.attention_topology.rapid_switch_count, 2);
    assert_eq!(
        model.attention_topology.deep_focus_surface.as_deref(),
        Some("conversation:chat")
    );
    assert!(model.attention_topology.avg_surface_dwell_secs > 0.0);
}

#[test]
fn risk_metrics_learn_auto_approve_and_auto_deny_categories() {
    let mut risk = RiskFingerprint {
        approvals: 6,
        denials: 4,
        category_requests: HashMap::from([
            ("git".to_string(), 4),
            ("destructive_delete".to_string(), 4),
            ("network_request".to_string(), 2),
        ]),
        category_approvals: HashMap::from([
            ("git".to_string(), 4),
            ("destructive_delete".to_string(), 0),
            ("network_request".to_string(), 2),
        ]),
        ..RiskFingerprint::default()
    };

    refresh_risk_metrics(&mut risk);

    assert_eq!(
        risk.auto_approve_categories,
        vec!["git".to_string()],
        "high-volume, always-approved categories should become shortcuts"
    );
    assert_eq!(
        risk.auto_deny_categories,
        vec!["destructive_delete".to_string()],
        "high-volume, never-approved categories should become auto-deny heuristics"
    );
}

#[test]
fn risk_metrics_learn_auto_deny_from_repeated_fast_denials() {
    let mut risk = RiskFingerprint {
        approvals: 1,
        denials: 3,
        category_requests: HashMap::from([("network_request".to_string(), 4)]),
        category_approvals: HashMap::from([("network_request".to_string(), 1)]),
        fast_denials_by_category: HashMap::from([("network_request".to_string(), 3)]),
        ..RiskFingerprint::default()
    };

    refresh_risk_metrics(&mut risk);

    assert_eq!(
        risk.auto_deny_categories,
        vec!["network_request".to_string()],
        "three fast denials in one category should learn an approval bypass even before the long-run approval rate drops to zero"
    );
}

#[tokio::test]
async fn reset_operator_model_clears_learned_shortcuts() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_approval_learning = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    {
        let mut model = engine.operator_model.write().await;
        model.risk_fingerprint.auto_approve_categories = vec!["git".to_string()];
        model.risk_fingerprint.auto_deny_categories = vec!["destructive_delete".to_string()];
        persist_operator_model(&engine.data_dir, &model).expect("persist learned model");
    }

    engine
        .reset_operator_model()
        .await
        .expect("reset operator model");

    let json = engine
        .operator_model_json()
        .await
        .expect("reload operator model json");
    let parsed: OperatorModel = serde_json::from_str(&json).expect("parse operator model json");
    assert!(parsed.risk_fingerprint.auto_approve_categories.is_empty());
    assert!(parsed.risk_fingerprint.auto_deny_categories.is_empty());
}

#[tokio::test]
async fn repeated_fast_denials_enable_learned_auto_deny() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_approval_learning = true;
    config.operator_model.allow_implicit_feedback = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let command = "curl https://example.com/status";
    let risk_level = "moderate";

    let approved = ToolPendingApproval {
        approval_id: "approval-approve".to_string(),
        execution_id: "exec-approve".to_string(),
        command: command.to_string(),
        rationale: "Fetch service status".to_string(),
        risk_level: risk_level.to_string(),
        blast_radius: "single endpoint".to_string(),
        reasons: vec!["network access".to_string()],
        session_id: None,
    };
    engine
        .record_operator_approval_requested(&approved)
        .await
        .expect("record approval request");
    engine
        .record_operator_approval_resolution(&approved.approval_id, ApprovalDecision::ApproveOnce)
        .await
        .expect("record approval resolution");

    for idx in 0..3 {
        let denial = ToolPendingApproval {
            approval_id: format!("approval-deny-{idx}"),
            execution_id: format!("exec-deny-{idx}"),
            command: command.to_string(),
            rationale: "Fetch service status".to_string(),
            risk_level: risk_level.to_string(),
            blast_radius: "single endpoint".to_string(),
            reasons: vec!["network access".to_string()],
            session_id: None,
        };
        engine
            .record_operator_approval_requested(&denial)
            .await
            .expect("record denial request");
        engine
            .record_operator_approval_resolution(&denial.approval_id, ApprovalDecision::Deny)
            .await
            .expect("record denial resolution");
    }

    assert!(matches!(
        engine.learned_approval_decision(command, risk_level).await,
        Some(ApprovalDecision::Deny)
    ));
}

#[tokio::test]
async fn high_approval_latency_suppresses_duplicate_low_value_approval_bundles() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_approval_learning = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.risk_fingerprint.approval_requests = 4;
        model.risk_fingerprint.approvals = 2;
        model.risk_fingerprint.denials = 2;
        model.risk_fingerprint.avg_response_time_secs = 45.0;
        refresh_risk_metrics(&mut model.risk_fingerprint);
    }

    let existing = ToolPendingApproval {
        approval_id: "approval-existing".to_string(),
        execution_id: "exec-existing".to_string(),
        command: "git status".to_string(),
        rationale: "Inspect repo status".to_string(),
        risk_level: "lowest".to_string(),
        blast_radius: "repo metadata".to_string(),
        reasons: vec!["reads git metadata".to_string()],
        session_id: None,
    };
    engine
        .record_operator_approval_requested(&existing)
        .await
        .expect("record existing approval request");

    let duplicate = ToolPendingApproval {
        approval_id: "approval-duplicate".to_string(),
        execution_id: "exec-duplicate".to_string(),
        command: "git diff --stat".to_string(),
        rationale: "Inspect repo delta".to_string(),
        risk_level: "lowest".to_string(),
        blast_radius: "repo metadata".to_string(),
        reasons: vec!["reads git metadata".to_string()],
        session_id: None,
    };

    assert!(
        engine
            .should_suppress_duplicate_low_value_approval_bundle(&duplicate)
            .await
    );
}

#[tokio::test]
async fn high_approval_latency_keeps_high_value_approvals_visible() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_approval_learning = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.risk_fingerprint.approval_requests = 4;
        model.risk_fingerprint.approvals = 2;
        model.risk_fingerprint.denials = 2;
        model.risk_fingerprint.avg_response_time_secs = 45.0;
        refresh_risk_metrics(&mut model.risk_fingerprint);
    }

    let existing = ToolPendingApproval {
        approval_id: "approval-existing".to_string(),
        execution_id: "exec-existing".to_string(),
        command: "git status".to_string(),
        rationale: "Inspect repo status".to_string(),
        risk_level: "lowest".to_string(),
        blast_radius: "repo metadata".to_string(),
        reasons: vec!["reads git metadata".to_string()],
        session_id: None,
    };
    engine
        .record_operator_approval_requested(&existing)
        .await
        .expect("record existing approval request");

    let high_value = ToolPendingApproval {
        approval_id: "approval-high-value".to_string(),
        execution_id: "exec-high-value".to_string(),
        command: "curl https://example.com/status".to_string(),
        rationale: "Fetch service status".to_string(),
        risk_level: "moderate".to_string(),
        blast_radius: "single endpoint".to_string(),
        reasons: vec!["network access".to_string()],
        session_id: None,
    };

    assert!(
        !engine
            .should_suppress_duplicate_low_value_approval_bundle(&high_value)
            .await
    );
}

#[tokio::test]
async fn high_confirmation_seeking_suppresses_learned_auto_approve_shortcuts() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_approval_learning = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.cognitive_style.confirmation_seeking = 0.92;
        model.risk_fingerprint.auto_approve_categories = vec!["git".to_string()];
    }

    assert!(
        engine
            .learned_approval_decision("git status", "lowest")
            .await
            .is_none(),
        "high confirmation-seeking should suppress learned auto-approval and require explicit approval"
    );
}

#[tokio::test]
async fn operator_messages_learn_summary_first_reasoning_on_demand_preferences() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_message_statistics = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let messages = [
        "Give me the tl;dr first.",
        "Use the short version again.",
        "Just the answer, no reasoning.",
        "Skip the explanation and summarize it.",
    ];
    for (index, message) in messages.iter().enumerate() {
        engine
            .record_operator_message("thread-reading", message, index == 0)
            .await
            .expect("record operator message");
    }

    let json = engine
        .operator_model_json()
        .await
        .expect("read operator model json");
    let parsed: OperatorModel = serde_json::from_str(&json).expect("parse operator model json");
    assert!(parsed.cognitive_style.prefers_summaries);
    assert!(parsed.cognitive_style.skips_reasoning);
    assert_eq!(parsed.cognitive_style.reading_depth, ReadingDepth::Skim);

    let summary = engine
        .build_operator_model_prompt_summary()
        .await
        .expect("operator model prompt summary");
    assert!(summary.contains("summary-first"));
    assert!(summary.contains("reasoning on demand"));
}

#[test]
fn operator_satisfaction_uses_signal_gates_and_friction() {
    let mut model = OperatorModel::default();
    refresh_operator_satisfaction(&mut model);
    assert_eq!(model.operator_satisfaction.label, "unknown");
    assert!((model.operator_satisfaction.score - 0.8).abs() < f64::EPSILON);
    assert!(model.diagnostic_summary().contains("strong >=0.80"));

    model.cognitive_style.message_count = 1;
    refresh_operator_satisfaction(&mut model);
    assert_eq!(model.operator_satisfaction.label, "strong");
    assert!((model.operator_satisfaction.score - 0.8).abs() < f64::EPSILON);
    assert!(model.diagnostic_summary().contains("signal present"));

    model.implicit_feedback.tool_hesitation_count = 1;
    model.implicit_feedback.revision_message_count = 1;
    model.implicit_feedback.correction_message_count = 1;
    model.implicit_feedback.fast_denial_count = 1;
    model.attention_topology.rapid_switch_count = 2;
    refresh_operator_satisfaction(&mut model);

    assert_eq!(model.operator_satisfaction.label, "strained");
    assert!((model.operator_satisfaction.score - 0.18).abs() < 1e-9);
}

#[test]
fn operator_model_diagnostic_summary_exposes_thresholds_and_friction() {
    let mut model = OperatorModel::default();
    model.cognitive_style.message_count = 1;
    model.implicit_feedback.tool_hesitation_count = 2;
    model.implicit_feedback.correction_message_count = 1;
    model.attention_topology.rapid_switch_count = 3;
    refresh_operator_satisfaction(&mut model);

    let summary = model.diagnostic_summary();
    assert!(summary.contains("satisfaction="));
    assert!(summary.contains("strained <0.35, fragile <0.55, healthy <0.80, strong >=0.80"));
    assert!(summary.contains("signal present"));
    assert!(summary.contains("corrections 1"));
    assert!(summary.contains("tool fallbacks 2"));
    assert!(summary.contains("rapid switches 3"));
    assert!(summary.contains("rapid reverts 0"));
}

#[tokio::test]
async fn tool_hesitation_refreshes_persisted_operator_satisfaction_and_summary() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_implicit_feedback = true;
    config.operator_model.allow_message_statistics = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_message("thread-satisfaction", "Please run tests.", true)
        .await
        .expect("record operator message");
    engine
        .record_tool_hesitation("read_file", "search_files", true, false)
        .await
        .expect("record tool hesitation");

    let json = engine
        .operator_model_json()
        .await
        .expect("read operator model json");
    let parsed: OperatorModel = serde_json::from_str(&json).expect("parse operator model json");
    assert_eq!(parsed.cognitive_style.message_count, 1);
    assert_eq!(parsed.implicit_feedback.tool_hesitation_count, 1);
    assert_eq!(parsed.operator_satisfaction.label, "healthy");
    assert!((parsed.operator_satisfaction.score - 0.68).abs() < 1e-9);

    let summary = engine
        .build_operator_model_prompt_summary()
        .await
        .expect("operator model prompt summary");
    assert!(summary.contains("Implicit feedback: 1 tool fallback(s), 0 revision-style operator message(s), 0 fast denial(s); common fallback read_file -> search_files"));
    assert!(summary.contains("Satisfaction signal: healthy (0.68); friction markers revisions 0, corrections 0, tool fallbacks 1, fast denials 0"));
    assert!(summary.contains("Adaptive response mode: keep a normal proactive cadence"));
    assert!(summary.contains("prefer the later successful fallback earlier"));
}

#[tokio::test]
async fn strong_operator_satisfaction_adds_proactive_guidance_without_friction() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_message_statistics = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_message("thread-strong", "Please run tests.", true)
        .await
        .expect("record operator message");

    let summary = engine
        .build_operator_model_prompt_summary()
        .await
        .expect("operator model prompt summary");
    assert!(summary.contains("Satisfaction signal: strong (0.80); friction markers revisions 0, corrections 0, tool fallbacks 0, fast denials 0"));
    assert!(summary
        .contains("Adaptive response mode: trust is high, so stay proactive and exploratory"));
    assert!(summary.contains("Adaptive delivery rule: start with the conclusion"));
}

#[tokio::test]
async fn fast_aggressive_approvals_add_proactive_approval_guidance() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_message_statistics = true;
    config.operator_model.allow_approval_learning = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.risk_fingerprint.approval_requests = 4;
        model.risk_fingerprint.approvals = 4;
        model.risk_fingerprint.denials = 0;
        model.risk_fingerprint.avg_response_time_secs = 3.0;
        model.risk_fingerprint.risk_tolerance = RiskTolerance::Aggressive;
        refresh_operator_satisfaction(&mut model);
    }

    let summary = engine
        .build_operator_model_prompt_summary()
        .await
        .expect("operator model prompt summary");
    assert!(summary.contains(
        "Risk tolerance: aggressive (4 approvals across 4 approval requests, avg response 3.0s)"
    ));
    assert!(summary.contains(
        "Adaptive approval rule: approvals resolve quickly and usually favor proceeding"
    ));
    assert!(summary.contains("avoid redundant confirmation loops for low-risk progress"));
}

#[tokio::test]
async fn slow_approval_latency_adds_deliberate_approval_guidance() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_message_statistics = true;
    config.operator_model.allow_approval_learning = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.risk_fingerprint.approval_requests = 4;
        model.risk_fingerprint.approvals = 2;
        model.risk_fingerprint.denials = 2;
        model.risk_fingerprint.avg_response_time_secs = 45.0;
        model.risk_fingerprint.risk_tolerance = RiskTolerance::Moderate;
        refresh_operator_satisfaction(&mut model);
    }

    let summary = engine
        .build_operator_model_prompt_summary()
        .await
        .expect("operator model prompt summary");
    assert!(summary.contains(
        "Risk tolerance: moderate (2 approvals across 4 approval requests, avg response 45.0s)"
    ));
    assert!(summary.contains("Adaptive approval rule: approval responses are deliberate"));
    assert!(summary.contains("keep only one pending approval live at a time"));
}

#[tokio::test]
async fn strained_operator_satisfaction_adds_recovery_guidance() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_message_statistics = true;
    config.operator_model.allow_implicit_feedback = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_message("thread-strained", "Please run tests.", true)
        .await
        .expect("record operator message");
    engine
        .record_tool_hesitation("read_file", "search_files", true, false)
        .await
        .expect("record tool hesitation");

    {
        let mut model = engine.operator_model.write().await;
        model.implicit_feedback.revision_message_count = 1;
        model.implicit_feedback.correction_message_count = 1;
        model.implicit_feedback.fast_denial_count = 1;
        model.attention_topology.rapid_switch_count = 2;
        refresh_operator_satisfaction(&mut model);
    }

    let summary = engine
        .build_operator_model_prompt_summary()
        .await
        .expect("operator model prompt summary");
    assert!(summary.contains("Satisfaction signal: strained (0.18); friction markers revisions 1, corrections 1, tool fallbacks 1, fast denials 1"));
    assert!(summary.contains("Adaptive response mode: reduce friction aggressively"));
    assert!(summary.contains("Adaptive delivery rule: keep the answer compact"));
    assert!(summary.contains("Adaptive clarification rule: when intent is underspecified, ask one targeted question before guessing broadly"));
    assert!(summary.contains("prefer the later successful fallback earlier"));
}

#[tokio::test]
async fn fragile_operator_satisfaction_adds_compact_delivery_and_clarification_guidance() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_message_statistics = true;
    config.operator_model.allow_implicit_feedback = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    {
        let mut model = engine.operator_model.write().await;
        model.cognitive_style.message_count = 1;
        model.operator_satisfaction.label = "fragile".to_string();
        model.operator_satisfaction.score = 0.54;
        model.implicit_feedback.correction_message_count = 1;
    }

    let summary = engine
        .build_operator_model_prompt_summary()
        .await
        .expect("operator model prompt summary");
    assert!(summary.contains("Satisfaction signal: fragile (0.54); friction markers revisions 0, corrections 1, tool fallbacks 0, fast denials 0"));
    assert!(summary.contains("Adaptive response mode: tighten the loop"));
    assert!(summary.contains("Adaptive delivery rule: keep the answer compact"));
    assert!(summary.contains("Adaptive clarification rule: when intent is underspecified, ask one targeted question before guessing broadly"));
}

#[tokio::test]
async fn status_diagnostics_snapshot_includes_operator_satisfaction_summary() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_implicit_feedback = true;
    config.operator_model.allow_message_statistics = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_message("thread-diagnostics", "Please run tests.", true)
        .await
        .expect("record operator message");

    let snapshot = engine.status_diagnostics_snapshot().await;
    let satisfaction = &snapshot["operator_satisfaction"];
    assert_eq!(satisfaction["label"], "strong");
    assert_eq!(satisfaction["message_count"], 1);
    let summary = satisfaction["summary"]
        .as_str()
        .expect("operator satisfaction summary string");
    assert!(summary.contains("strong >=0.80"));
    assert!(summary.contains("signal present"));
}

#[test]
fn preferred_tool_fallback_targets_deduplicates_and_skips_invalid_pairs() {
    let preferred = preferred_tool_fallback_targets(
        &[
            "read_file -> search_files".to_string(),
            "READ_FILE -> Search_Files".to_string(),
            "search_files -> read_file".to_string(),
            "invalid-pair".to_string(),
            "tool_a ->   ".to_string(),
        ],
        3,
    );

    assert_eq!(
        preferred,
        vec!["search_files".to_string(), "read_file".to_string()]
    );
}

#[tokio::test]
async fn implicit_feedback_persistence_records_signal_rows_and_score_history() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_message_statistics = true;
    config.operator_model.allow_implicit_feedback = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_message("thread-persisted-satisfaction", "Please run tests.", true)
        .await
        .expect("record operator message");
    engine
        .record_tool_hesitation("read_file", "search_files", true, false)
        .await
        .expect("record tool hesitation");

    let signals = engine
        .history
        .list_implicit_signals("global", 10)
        .await
        .expect("load implicit signals");
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].signal_type, "tool_fallback");
    assert!((signals[0].weight + 0.12).abs() < f64::EPSILON);
    assert!(signals[0]
        .context_snapshot_json
        .as_deref()
        .is_some_and(|json| json.contains("search_files")));

    let scores = engine
        .history
        .list_satisfaction_scores("global", 10)
        .await
        .expect("load satisfaction scores");
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0].label, "healthy");
    assert_eq!(scores[0].signal_count, 1);
    assert!((scores[0].score - 0.68).abs() < 1e-9);
}

#[tokio::test]
async fn operator_correction_persists_thread_scoped_signal_and_score_snapshot() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_message_statistics = true;
    config.operator_model.allow_implicit_feedback = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_message(
            "thread-correction-persist",
            "Actually, use ripgrep instead.",
            true,
        )
        .await
        .expect("record correction message");

    let signals = engine
        .history
        .list_implicit_signals("thread-correction-persist", 10)
        .await
        .expect("load correction signals");
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].signal_type, "operator_correction");
    assert!((signals[0].weight + 0.16).abs() < f64::EPSILON);

    let scores = engine
        .history
        .list_satisfaction_scores("thread-correction-persist", 10)
        .await
        .expect("load correction satisfaction scores");
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0].label, "fragile");
    assert_eq!(scores[0].signal_count, 2);
    assert!((scores[0].score - 0.54).abs() < 1e-9);
}

#[tokio::test]
async fn rapid_revert_persists_thread_scoped_signal_when_agent_file_edit_is_quickly_reverted() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_implicit_feedback = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    engine.set_aline_startup_test_availability(false);

    let git_dir = root.path();
    let git_init = std::process::Command::new("git")
        .args(["init"])
        .current_dir(git_dir)
        .output()
        .expect("git init should spawn");
    assert!(git_init.status.success(), "git init should succeed");

    let git_user_name = std::process::Command::new("git")
        .args(["config", "user.name", "tamux tests"])
        .current_dir(git_dir)
        .output()
        .expect("git config user.name should spawn");
    assert!(
        git_user_name.status.success(),
        "git config user.name should succeed"
    );

    let git_user_email = std::process::Command::new("git")
        .args(["config", "user.email", "tamux@example.com"])
        .current_dir(git_dir)
        .output()
        .expect("git config user.email should spawn");
    assert!(
        git_user_email.status.success(),
        "git config user.email should succeed"
    );

    let src_dir = root.path().join("src");
    std::fs::create_dir_all(&src_dir).expect("create src dir");
    let file_path = src_dir.join("lib.rs");
    let baseline = "pub fn answer() -> u32 {\n    41\n}\n";
    std::fs::write(&file_path, baseline).expect("write baseline file");

    let git_add = std::process::Command::new("git")
        .args(["add", "src/lib.rs"])
        .current_dir(git_dir)
        .output()
        .expect("git add should spawn");
    assert!(git_add.status.success(), "git add should succeed");

    let git_commit = std::process::Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(git_dir)
        .output()
        .expect("git commit should spawn");
    assert!(git_commit.status.success(), "git commit should succeed");

    let agent_version = "pub fn answer() -> u32 {\n    42\n}\n";
    std::fs::write(&file_path, agent_version).expect("write agent version");
    engine
        .record_file_work_context(
            "thread-rapid-revert",
            None,
            "write_file",
            file_path.to_str().expect("utf-8 file path"),
        )
        .await;

    std::fs::write(&file_path, baseline).expect("revert file back to baseline");
    engine
        .refresh_thread_repo_context("thread-rapid-revert")
        .await;

    let signals = engine
        .history
        .list_implicit_signals("thread-rapid-revert", 10)
        .await
        .expect("load rapid revert signals");
    assert_eq!(
        signals.len(),
        1,
        "rapid revert should persist exactly one implicit feedback signal"
    );
    assert_eq!(signals[0].signal_type, "rapid_revert");
    assert!(
        signals[0].weight < -0.16,
        "rapid revert should be a stronger negative signal than an operator correction"
    );
    assert!(signals[0]
        .context_snapshot_json
        .as_deref()
        .is_some_and(|json| json.contains("src/lib.rs") && json.contains("write_file")));
}

#[tokio::test]
async fn very_short_attention_dwell_persists_short_dwell_signal_with_duration() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_implicit_feedback = true;
    config.operator_model.allow_attention_tracking = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_attention_surface("conversation:chat")
        .await
        .expect("record first attention surface");
    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    engine
        .record_attention_surface("modal:command_palette")
        .await
        .expect("record second attention surface");

    let signals = engine
        .history
        .list_implicit_signals("global", 10)
        .await
        .expect("load implicit signals");
    let short_dwell = signals
        .iter()
        .find(|signal| signal.signal_type == "short_dwell")
        .expect("short dwell signal should be persisted");
    assert!(short_dwell.weight < 0.0);
    assert!(short_dwell
        .context_snapshot_json
        .as_deref()
        .is_some_and(|json| json.contains("dwell_secs") && json.contains("conversation:chat")));
}

#[tokio::test]
async fn deleting_thread_right_after_assistant_reply_persists_session_abandon_signal() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_implicit_feedback = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let now = now_millis();
    let mut assistant = AgentMessage::user("Here is the answer.", now);
    assistant.role = MessageRole::Assistant;

    engine.threads.write().await.insert(
        "thread-session-abandon".to_string(),
        AgentThread {
            id: "thread-session-abandon".to_string(),
            agent_name: Some(crate::agent::agent_identity::MAIN_AGENT_NAME.to_string()),
            title: "Abandonable thread".to_string(),
            messages: vec![
                AgentMessage::user("Help me.", now.saturating_sub(1_000)),
                assistant,
            ],
            pinned: false,
            upstream_thread_id: None,
            upstream_transport: None,
            upstream_provider: None,
            upstream_model: None,
            upstream_assistant_id: None,
            created_at: now.saturating_sub(1_000),
            updated_at: now,
            total_input_tokens: 0,
            total_output_tokens: 0,
        },
    );

    assert!(engine.delete_thread("thread-session-abandon").await);

    let signals = engine
        .history
        .list_implicit_signals("thread-session-abandon", 10)
        .await
        .expect("load session abandonment signals");
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].signal_type, "session_abandon");
    assert!(signals[0].weight < 0.0);
    assert!(signals[0]
        .context_snapshot_json
        .as_deref()
        .is_some_and(
            |json| json.contains("thread-session-abandon") && json.contains("Here is the answer.")
        ));

    let scores = engine
        .history
        .list_satisfaction_scores("thread-session-abandon", 10)
        .await
        .expect("load abandonment satisfaction scores");
    assert_eq!(scores.len(), 1);
    assert!(matches!(
        scores[0].label.as_str(),
        "fragile" | "strained" | "healthy"
    ));
    assert_eq!(scores[0].signal_count, 1);
}

#[test]
fn persisted_satisfaction_decay_uses_recent_signal_history() {
    let mut model = OperatorModel::default();
    model.cognitive_style.message_count = 1;

    let now = 1_717_400_000_000u64;
    let applied = apply_persisted_satisfaction_decay(
        &mut model,
        &[
            PersistedSatisfactionSignalSample {
                weight: -0.12,
                timestamp_ms: now - 1_000,
            },
            PersistedSatisfactionSignalSample {
                weight: -0.16,
                timestamp_ms: now - 2_000,
            },
            PersistedSatisfactionSignalSample {
                weight: -0.18,
                timestamp_ms: now - 3_000,
            },
        ],
        now,
    );

    assert!(applied);
    assert_eq!(model.operator_satisfaction.label, "strained");
    assert!((model.operator_satisfaction.score - 0.34).abs() < 0.02);
}

#[test]
fn persisted_satisfaction_decay_requires_enough_history() {
    let mut model = OperatorModel::default();
    model.cognitive_style.message_count = 1;
    model.operator_satisfaction.score = 0.8;
    model.operator_satisfaction.label = "strong".to_string();

    let applied = apply_persisted_satisfaction_decay(
        &mut model,
        &[
            PersistedSatisfactionSignalSample {
                weight: -0.12,
                timestamp_ms: 10,
            },
            PersistedSatisfactionSignalSample {
                weight: -0.10,
                timestamp_ms: 20,
            },
        ],
        1_000,
    );

    assert!(!applied);
    assert_eq!(model.operator_satisfaction.label, "strong");
    assert!((model.operator_satisfaction.score - 0.8).abs() < f64::EPSILON);
}

#[tokio::test]
async fn status_diagnostics_snapshot_includes_persisted_implicit_feedback_history() {
    let root = tempdir().expect("tempdir");
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.operator_model.enabled = true;
    config.operator_model.allow_implicit_feedback = true;
    config.operator_model.allow_message_statistics = true;
    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    engine
        .record_operator_message("thread-diagnostics-persisted", "Please run tests.", true)
        .await
        .expect("record operator message");
    engine
        .record_tool_hesitation("read_file", "search_files", true, false)
        .await
        .expect("record tool hesitation");

    let snapshot = engine.status_diagnostics_snapshot().await;
    let satisfaction = &snapshot["operator_satisfaction"];
    let signals = satisfaction["recent_implicit_signals"]
        .as_array()
        .expect("recent implicit signals array");
    let scores = satisfaction["recent_satisfaction_scores"]
        .as_array()
        .expect("recent satisfaction scores array");

    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0]["signal_type"], "tool_fallback");
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0]["label"], "healthy");
}
