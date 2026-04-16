use super::*;
use amux_shared::providers::{PROVIDER_ID_GROQ, PROVIDER_ID_OPENAI};

#[tokio::test]
async fn set_provider_model_json_updates_provider_and_model_atomically() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    let mut config = engine.get_config().await;
    config.api_key = "sk-test".to_string();
    engine.set_config(config).await;

    engine
        .set_provider_model_json(PROVIDER_ID_OPENAI, "gpt-5.4-mini")
        .await
        .unwrap();

    let updated = engine.get_config().await;
    assert_eq!(updated.provider, PROVIDER_ID_OPENAI);
    assert_eq!(updated.model, "gpt-5.4-mini");
}

#[tokio::test]
async fn set_provider_model_json_restores_canonical_base_url_for_predefined_provider() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    let mut config = engine.get_config().await;
    config.api_key = "groq-key".to_string();
    config.base_url = "https://stale.example.invalid/v1".to_string();
    engine.set_config(config).await;

    engine
        .set_provider_model_json(PROVIDER_ID_GROQ, "llama-3.3-70b-versatile")
        .await
        .unwrap();

    let updated = engine.get_config().await;
    assert_eq!(updated.provider, PROVIDER_ID_GROQ);
    assert_eq!(updated.model, "llama-3.3-70b-versatile");
    assert_eq!(updated.base_url, "https://api.groq.com/openai/v1");
}

#[tokio::test]
async fn set_config_item_json_persists_managed_execution_security_level() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    engine
        .set_config_item_json("/managed_execution/security_level", r#""yolo""#)
        .await
        .expect("managed execution security level should update");

    let updated = engine.get_config().await;
    assert_eq!(
        updated.managed_execution.security_level,
        SecurityLevel::Yolo
    );

    let persisted_items = engine
        .history
        .list_agent_config_items()
        .await
        .expect("persisted config should be readable");
    let rehydrated =
        load_config_from_items(persisted_items).expect("persisted config should deserialize");
    assert_eq!(
        rehydrated.managed_execution.security_level,
        SecurityLevel::Yolo
    );
}

#[tokio::test]
async fn set_config_item_json_persists_sleep_delay_settings() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    engine
        .set_config_item_json("/message_loop_delay_ms", "250")
        .await
        .expect("message loop delay should update");
    engine
        .set_config_item_json("/tool_call_delay_ms", "750")
        .await
        .expect("tool call delay should update");

    let updated = engine.get_config().await;
    assert_eq!(updated.message_loop_delay_ms, 250);
    assert_eq!(updated.tool_call_delay_ms, 750);

    let persisted_items = engine
        .history
        .list_agent_config_items()
        .await
        .expect("persisted config should be readable");
    let rehydrated =
        load_config_from_items(persisted_items).expect("persisted config should deserialize");
    assert_eq!(rehydrated.message_loop_delay_ms, 250);
    assert_eq!(rehydrated.tool_call_delay_ms, 750);
}

#[tokio::test]
async fn set_config_item_json_persists_snapshot_retention_settings() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    engine
        .set_config_item_json("/snapshot_retention/max_snapshots", "0")
        .await
        .expect("snapshot max count should update");
    engine
        .set_config_item_json("/snapshot_retention/max_total_size_mb", "2048")
        .await
        .expect("snapshot size limit should update");

    let updated = engine.get_config().await;
    assert_eq!(updated.snapshot_retention.max_snapshots, 0);
    assert_eq!(updated.snapshot_retention.max_total_size_mb, 2048);
    assert!(!updated.snapshot_retention.auto_cleanup);

    let persisted_items = engine
        .history
        .list_agent_config_items()
        .await
        .expect("persisted config should be readable");
    let rehydrated =
        load_config_from_items(persisted_items).expect("persisted config should deserialize");
    assert_eq!(rehydrated.snapshot_retention.max_snapshots, 0);
    assert_eq!(rehydrated.snapshot_retention.max_total_size_mb, 2048);
    assert!(!rehydrated.snapshot_retention.auto_cleanup);
}

#[tokio::test]
async fn prepare_config_item_json_validates_without_mutating_runtime_config() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    let (prepared_config, prepared_value) = engine
        .prepare_config_item_json("/managed_execution/security_level", r#""yolo""#)
        .await
        .expect("config item preparation should succeed");

    assert_eq!(prepared_value, serde_json::json!("yolo"));
    assert_eq!(
        prepared_config.managed_execution.security_level,
        SecurityLevel::Yolo
    );

    let current = engine.get_config().await;
    assert_eq!(
        current.managed_execution.security_level,
        SecurityLevel::Lowest
    );
}

#[tokio::test]
async fn set_provider_model_json_rejects_invalid_model_without_changing_config() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    let mut config = engine.get_config().await;
    config.api_key = "sk-test".to_string();
    engine.set_config(config).await;

    let before = engine.get_config().await;
    let result = engine
        .set_provider_model_json(PROVIDER_ID_OPENAI, "definitely-not-a-real-model")
        .await;
    assert!(result.is_err());

    let after = engine.get_config().await;
    assert_eq!(before.provider, after.provider);
    assert_eq!(before.model, after.model);
    assert_eq!(before.base_url, after.base_url);
}

#[tokio::test]
async fn prepare_provider_model_json_validates_without_mutating_runtime_config() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    let mut config = engine.get_config().await;
    config.api_key = "sk-test".to_string();
    engine.set_config(config).await;

    let prepared = engine
        .prepare_provider_model_json(PROVIDER_ID_OPENAI, "gpt-5.4-mini")
        .await
        .expect("provider/model preparation should succeed");

    assert_eq!(prepared.provider, PROVIDER_ID_OPENAI);
    assert_eq!(prepared.model, "gpt-5.4-mini");

    let current = engine.get_config().await;
    assert_ne!(current.model, "gpt-5.4-mini");
}

#[tokio::test]
async fn prepare_agent_provider_model_json_updates_builtin_persona_overrides() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    let mut config = engine.get_config().await;
    config.api_key = "sk-test".to_string();
    engine.set_config(config).await;

    let prepared = engine
        .prepare_agent_provider_model_json("swarozyc", PROVIDER_ID_OPENAI, "gpt-5.4-mini")
        .await
        .expect("builtin persona provider/model preparation should succeed");

    assert_eq!(
        prepared.builtin_sub_agents.swarozyc.provider.as_deref(),
        Some(PROVIDER_ID_OPENAI)
    );
    assert_eq!(
        prepared.builtin_sub_agents.swarozyc.model.as_deref(),
        Some("gpt-5.4-mini")
    );

    let current = engine.get_config().await;
    assert!(current.builtin_sub_agents.swarozyc.provider.is_none());
    assert!(current.builtin_sub_agents.swarozyc.model.is_none());
}

#[tokio::test]
async fn prepare_agent_provider_model_json_updates_new_builtin_persona_overrides() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    let mut config = engine.get_config().await;
    config.api_key = "sk-test".to_string();
    engine.set_config(config).await;

    let prepared = engine
        .prepare_agent_provider_model_json("perun", PROVIDER_ID_OPENAI, "gpt-5.4-mini")
        .await
        .expect("new builtin persona provider/model preparation should succeed");

    let prepared_json = serde_json::to_value(&prepared).expect("config should serialize");
    assert_eq!(
        prepared_json["builtin_sub_agents"]["perun"]["provider"].as_str(),
        Some(PROVIDER_ID_OPENAI)
    );
    assert_eq!(
        prepared_json["builtin_sub_agents"]["perun"]["model"].as_str(),
        Some("gpt-5.4-mini")
    );

    let current_json =
        serde_json::to_value(engine.get_config().await).expect("config should serialize");
    assert!(
        current_json["builtin_sub_agents"]["perun"]["provider"].is_null(),
        "runtime config should not be mutated during preparation"
    );
    assert!(
        current_json["builtin_sub_agents"]["perun"]["model"].is_null(),
        "runtime config should not be mutated during preparation"
    );
}

#[tokio::test]
async fn persisted_config_is_visible_while_runtime_reconcile_is_still_in_flight() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    engine
        .set_test_config_reconcile_delay(Some(std::time::Duration::from_secs(1)))
        .await;

    let (prepared, value) = engine
        .prepare_config_item_json("/managed_execution/security_level", r#""yolo""#)
        .await
        .expect("config item preparation should succeed");

    engine
        .persist_prepared_config_item_json("/managed_execution/security_level", &value, prepared)
        .await
        .expect("persist prepared config item");

    let reconcile = {
        let engine = engine.clone();
        tokio::spawn(async move {
            let _ = engine.reconcile_config_runtime_after_commit().await;
        })
    };

    tokio::task::yield_now().await;

    let desired = engine.get_config().await;
    assert_eq!(
        desired.managed_execution.security_level,
        SecurityLevel::Yolo
    );

    let projection = engine.current_config_runtime_projection().await;
    assert_eq!(projection.state, ConfigReconcileState::Reconciling);
    assert!(projection.effective_revision < projection.desired_revision);

    reconcile.await.expect("reconcile join should succeed");

    let settled = engine.current_config_runtime_projection().await;
    assert_eq!(settled.state, ConfigReconcileState::Applied);
    assert_eq!(settled.effective_revision, settled.desired_revision);
}

#[tokio::test]
async fn reconcile_failure_marks_projection_error_without_reverting_desired_config() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    engine
        .set_test_config_reconcile_failure(Some("forced reconcile failure".to_string()))
        .await;

    let (prepared, value) = engine
        .prepare_config_item_json("/managed_execution/security_level", r#""yolo""#)
        .await
        .expect("config item preparation should succeed");

    engine
        .persist_prepared_config_item_json("/managed_execution/security_level", &value, prepared)
        .await
        .expect("persist prepared config item");

    let result = engine.reconcile_config_runtime_after_commit().await;
    assert!(result.is_err(), "reconcile should fail under the test hook");

    let desired = engine.get_config().await;
    assert_eq!(
        desired.managed_execution.security_level,
        SecurityLevel::Yolo
    );

    let projection = engine.current_config_runtime_projection().await;
    assert_eq!(projection.state, ConfigReconcileState::Error);
    assert_eq!(
        projection.effective_revision + 1,
        projection.desired_revision
    );
    assert_eq!(
        projection.last_error.as_deref(),
        Some("forced reconcile failure")
    );
}

#[tokio::test]
async fn gateway_reconcile_without_connected_runtime_marks_projection_degraded() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    let mut config = engine.get_config().await;
    config.gateway.enabled = true;
    config.gateway.slack_token = "xoxb-test".to_string();
    engine.set_config(config).await;

    let (prepared, value) = engine
        .prepare_config_item_json("/gateway/command_prefix", r#""!tamux""#)
        .await
        .expect("config item preparation should succeed");

    engine
        .persist_prepared_config_item_json("/gateway/command_prefix", &value, prepared)
        .await
        .expect("persist prepared config item");

    engine
        .reconcile_config_runtime_after_commit()
        .await
        .expect("reconcile should degrade, not fail");

    let desired = engine.get_config().await;
    assert_eq!(desired.gateway.command_prefix, "!tamux");

    let projection = engine.current_config_runtime_projection().await;
    assert_eq!(projection.state, ConfigReconcileState::Degraded);
    assert!(projection.effective_revision < projection.desired_revision);
    assert!(projection
        .last_error
        .as_deref()
        .unwrap_or_default()
        .contains("reload"));
}

#[tokio::test]
async fn desired_and_effective_config_surfaces_diverge_during_reconcile() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    engine
        .set_test_config_reconcile_delay(Some(std::time::Duration::from_secs(1)))
        .await;

    let (prepared, value) = engine
        .prepare_config_item_json("/managed_execution/security_level", r#""yolo""#)
        .await
        .expect("config item preparation should succeed");

    engine
        .persist_prepared_config_item_json("/managed_execution/security_level", &value, prepared)
        .await
        .expect("persist prepared config item");

    let reconcile = {
        let engine = engine.clone();
        tokio::spawn(async move {
            let _ = engine.reconcile_config_runtime_after_commit().await;
        })
    };

    tokio::task::yield_now().await;

    let desired = engine.current_desired_config_snapshot().await;
    let effective = engine.current_effective_config_runtime_state().await;

    assert_eq!(
        desired.managed_execution.security_level,
        SecurityLevel::Yolo
    );
    assert_eq!(effective.reconcile.state, ConfigReconcileState::Reconciling);
    assert!(effective.reconcile.effective_revision < effective.reconcile.desired_revision);

    reconcile.await.expect("reconcile join should succeed");
}

#[tokio::test]
async fn startup_silently_rederives_degraded_effective_state_from_desired_gateway_config() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.gateway.enabled = true;
    config.gateway.slack_token = "xoxb-test".to_string();

    let engine = AgentEngine::new_test(manager, config, root.path()).await;

    let desired = engine.current_desired_config_snapshot().await;
    let effective = engine.current_effective_config_runtime_state().await;

    assert!(desired.gateway.enabled);
    assert_eq!(effective.reconcile.state, ConfigReconcileState::Degraded);
    assert!(effective.reconcile.effective_revision < effective.reconcile.desired_revision);
    assert!(!effective.gateway_runtime_connected);
    assert!(effective
        .reconcile
        .last_error
        .as_deref()
        .unwrap_or_default()
        .contains("startup"));
}
