use super::*;
use crate::history::schema_helpers::table_has_column;

#[tokio::test]
async fn init_schema_adds_meta_cognition_tables() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .conn
        .call(|conn| {
            conn.execute_batch(
                "
                DROP TABLE IF EXISTS workflow_profiles;
                DROP TABLE IF EXISTS cognitive_biases;
                DROP TABLE IF EXISTS meta_cognition_model;
                ",
            )?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    store.init_schema().await?;

    let status = store
        .conn
        .call(|conn| {
            let model_has_agent_id = table_has_column(conn, "meta_cognition_model", "agent_id")?;
            let model_has_offset = table_has_column(conn, "meta_cognition_model", "calibration_offset")?;
            let bias_has_pattern = table_has_column(conn, "cognitive_biases", "trigger_pattern_json")?;
            let bias_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_cognitive_biases_model'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let workflow_has_tools = table_has_column(conn, "workflow_profiles", "typical_tools_json")?;
            let workflow_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_workflow_profiles_model'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            Ok((
                model_has_agent_id,
                model_has_offset,
                bias_has_pattern,
                bias_index,
                workflow_has_tools,
                workflow_index,
            ))
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    assert!(status.0);
    assert!(status.1);
    assert!(status.2);
    assert_eq!(status.3.as_deref(), Some("idx_cognitive_biases_model"));
    assert!(status.4);
    assert_eq!(status.5.as_deref(), Some("idx_workflow_profiles_model"));

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn meta_cognition_rows_round_trip() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .upsert_meta_cognition_model("svarog", -0.12, 1_717_200_000)
        .await?;

    let model = store
        .get_meta_cognition_model()
        .await?
        .expect("meta cognition model should exist");
    assert_eq!(model.agent_id, "svarog");
    assert!((model.calibration_offset + 0.12).abs() < f64::EPSILON);

    store
        .replace_cognitive_biases(
            model.id,
            &[CognitiveBiasRow {
                id: 0,
                model_id: model.id,
                name: "sunk_cost".to_string(),
                trigger_pattern_json: serde_json::json!({
                    "tool_sequence": ["bash_command"],
                    "max_revisions": 3,
                    "context_tags": ["retry_loop"]
                })
                .to_string(),
                mitigation_prompt: "re-evaluate approach".to_string(),
                severity: 0.8,
                occurrence_count: 4,
            }],
        )
        .await?;

    store
        .replace_workflow_profiles(
            model.id,
            &[WorkflowProfileRow {
                id: 0,
                model_id: model.id,
                name: "debug_loop".to_string(),
                avg_success_rate: 0.61,
                avg_steps: 7,
                typical_tools_json: serde_json::json!([
                    "read_file",
                    "search_files",
                    "bash_command"
                ])
                .to_string(),
            }],
        )
        .await?;

    let biases = store.list_cognitive_biases(model.id).await?;
    assert_eq!(biases.len(), 1);
    assert_eq!(biases[0].name, "sunk_cost");
    assert_eq!(biases[0].occurrence_count, 4);

    let workflows = store.list_workflow_profiles(model.id).await?;
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0].name, "debug_loop");
    assert_eq!(workflows[0].avg_steps, 7);

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn init_schema_adds_implicit_feedback_tables() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .conn
        .call(|conn| {
            conn.execute_batch(
                "
                DROP TABLE IF EXISTS satisfaction_scores;
                DROP TABLE IF EXISTS implicit_signals;
                ",
            )?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    store.init_schema().await?;

    let status = store
        .conn
        .call(|conn| {
            let signals_has_type = table_has_column(conn, "implicit_signals", "signal_type")?;
            let signals_has_context =
                table_has_column(conn, "implicit_signals", "context_snapshot_json")?;
            let signals_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_implicit_signals_session_ts'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let scores_has_label = table_has_column(conn, "satisfaction_scores", "label")?;
            let scores_has_signal_count =
                table_has_column(conn, "satisfaction_scores", "signal_count")?;
            let scores_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_satisfaction_scores_session_ts'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            Ok((
                signals_has_type,
                signals_has_context,
                signals_index,
                scores_has_label,
                scores_has_signal_count,
                scores_index,
            ))
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    assert!(status.0);
    assert!(status.1);
    assert_eq!(status.2.as_deref(), Some("idx_implicit_signals_session_ts"));
    assert!(status.3);
    assert!(status.4);
    assert_eq!(
        status.5.as_deref(),
        Some("idx_satisfaction_scores_session_ts")
    );

    let intent_status = store
        .conn
        .call(|conn| {
            let predictions_has_actual = table_has_column(conn, "intent_predictions", "actual_action")?;
            let predictions_has_correct = table_has_column(conn, "intent_predictions", "was_correct")?;
            let predictions_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_intent_predictions_session_ts'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            Ok((predictions_has_actual, predictions_has_correct, predictions_index))
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    assert!(intent_status.0);
    assert!(intent_status.1);
    assert_eq!(
        intent_status.2.as_deref(),
        Some("idx_intent_predictions_session_ts")
    );

    let foresight_status = store
        .conn
        .call(|conn| {
            let has_prediction_type =
                table_has_column(conn, "system_outcome_predictions", "prediction_type")?;
            let has_predicted_outcome =
                table_has_column(conn, "system_outcome_predictions", "predicted_outcome")?;
            let has_actual_outcome =
                table_has_column(conn, "system_outcome_predictions", "actual_outcome")?;
            let has_was_correct =
                table_has_column(conn, "system_outcome_predictions", "was_correct")?;
            let predictions_index: Option<String> = conn
                .query_row(
                    "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_system_outcome_predictions_session_ts'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            Ok((
                has_prediction_type,
                has_predicted_outcome,
                has_actual_outcome,
                has_was_correct,
                predictions_index,
            ))
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    assert!(foresight_status.0);
    assert!(foresight_status.1);
    assert!(foresight_status.2);
    assert!(foresight_status.3);
    assert_eq!(
        foresight_status.4.as_deref(),
        Some("idx_system_outcome_predictions_session_ts")
    );

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn system_outcome_prediction_rows_round_trip_and_resolve() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .insert_system_outcome_prediction(&SystemOutcomePredictionRow {
            id: "foresight-1".to_string(),
            session_id: "thread-foresight".to_string(),
            prediction_type: "build_test_risk".to_string(),
            predicted_outcome: "build/test failure".to_string(),
            confidence: 0.82,
            actual_outcome: None,
            was_correct: None,
            created_at_ms: 1_717_300_101,
        })
        .await?;

    let before = store
        .list_system_outcome_predictions("thread-foresight", 10)
        .await?;
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].prediction_type, "build_test_risk");
    assert_eq!(before[0].predicted_outcome, "build/test failure");
    assert!((before[0].confidence - 0.82).abs() < f64::EPSILON);
    assert_eq!(before[0].was_correct, None);

    store
        .resolve_latest_system_outcome_prediction(
            "thread-foresight",
            "build/test failure",
            Some("build/test failure"),
        )
        .await?;

    let after = store
        .list_system_outcome_predictions("thread-foresight", 10)
        .await?;
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].actual_outcome.as_deref(),
        Some("build/test failure")
    );
    assert_eq!(after[0].was_correct, Some(true));

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn implicit_feedback_rows_round_trip() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .insert_implicit_signal(&ImplicitSignalRow {
            id: "implicit-1".to_string(),
            session_id: "thread-implicit".to_string(),
            signal_type: "tool_fallback".to_string(),
            weight: -0.12,
            timestamp_ms: 1_717_300_001,
            context_snapshot_json: Some(
                serde_json::json!({
                    "from_tool": "read_file",
                    "to_tool": "search_files"
                })
                .to_string(),
            ),
        })
        .await?;

    store
        .insert_satisfaction_score(&SatisfactionScoreRow {
            id: "score-1".to_string(),
            session_id: "thread-implicit".to_string(),
            score: 0.68,
            computed_at_ms: 1_717_300_002,
            label: "healthy".to_string(),
            signal_count: 2,
        })
        .await?;

    let signals = store.list_implicit_signals("thread-implicit", 10).await?;
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].signal_type, "tool_fallback");
    assert!((signals[0].weight + 0.12).abs() < f64::EPSILON);

    let scores = store
        .list_satisfaction_scores("thread-implicit", 10)
        .await?;
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0].label, "healthy");
    assert_eq!(scores[0].signal_count, 2);
    assert!((scores[0].score - 0.68).abs() < f64::EPSILON);

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn intent_prediction_rows_round_trip_and_resolve() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .insert_intent_prediction(&IntentPredictionRow {
            id: "intent-1".to_string(),
            session_id: "thread-intent".to_string(),
            context_state_hash: "ctx-1".to_string(),
            predicted_action: "review pending approval".to_string(),
            confidence: 0.86,
            actual_action: None,
            was_correct: None,
            created_at_ms: 1_717_300_003,
        })
        .await?;

    let before = store.list_intent_predictions("thread-intent", 10).await?;
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].predicted_action, "review pending approval");
    assert_eq!(before[0].was_correct, None);

    store
        .resolve_latest_intent_prediction(
            "thread-intent",
            "review pending approval",
            Some("review pending approval"),
        )
        .await?;

    let after = store.list_intent_predictions("thread-intent", 10).await?;
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].actual_action.as_deref(),
        Some("review pending approval")
    );
    assert_eq!(after[0].was_correct, Some(true));

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn intent_prediction_success_rate_uses_recent_resolved_outcomes() -> Result<()> {
    let (store, root) = make_test_store().await?;

    for (id, was_correct, created_at_ms) in [
        ("intent-a", true, 100),
        ("intent-b", false, 200),
        ("intent-c", true, 300),
    ] {
        store
            .insert_intent_prediction(&IntentPredictionRow {
                id: id.to_string(),
                session_id: "thread-intent".to_string(),
                context_state_hash: format!("ctx-{id}"),
                predicted_action: "review pending approval".to_string(),
                confidence: 0.8,
                actual_action: Some("review pending approval".to_string()),
                was_correct: Some(was_correct),
                created_at_ms,
            })
            .await?;
    }

    let rate = store
        .recent_intent_prediction_success_rate("review pending approval", 10)
        .await?
        .expect("success rate should be present");
    assert!((rate - (2.0 / 3.0)).abs() < 1e-9);

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn recent_implicit_signal_samples_return_latest_weights() -> Result<()> {
    let (store, root) = make_test_store().await?;

    store
        .insert_implicit_signal(&ImplicitSignalRow {
            id: "implicit-a".to_string(),
            session_id: "thread-a".to_string(),
            signal_type: "tool_fallback".to_string(),
            weight: -0.12,
            timestamp_ms: 100,
            context_snapshot_json: None,
        })
        .await?;
    store
        .insert_implicit_signal(&ImplicitSignalRow {
            id: "implicit-b".to_string(),
            session_id: "thread-b".to_string(),
            signal_type: "fast_denial".to_string(),
            weight: -0.18,
            timestamp_ms: 300,
            context_snapshot_json: None,
        })
        .await?;
    store
        .insert_implicit_signal(&ImplicitSignalRow {
            id: "implicit-c".to_string(),
            session_id: "thread-c".to_string(),
            signal_type: "operator_correction".to_string(),
            weight: -0.16,
            timestamp_ms: 200,
            context_snapshot_json: None,
        })
        .await?;

    let samples = store.list_recent_implicit_signal_samples(2).await?;
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[0], (-0.18, 300));
    assert_eq!(samples[1], (-0.16, 200));

    std::fs::remove_dir_all(root)?;
    Ok(())
}
