use super::*;
use amux_shared::providers::PROVIDER_ID_OPENAI;

#[tokio::test]
async fn metacognitive_sunk_cost_block_injects_reflection_and_skips_tool_execution() {
    let root = tempdir().unwrap();
    let readable_path = root.path().join("blocked.txt");
    fs::write(&readable_path, "should never be read\n").expect("write blocked file");

    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = PROVIDER_ID_OPENAI.to_string();
    config.base_url = spawn_scripted_tool_call_server(vec![
        (
            "bash_command".to_string(),
            serde_json::json!({ "command": "false" }).to_string(),
        ),
        (
            "bash_command".to_string(),
            serde_json::json!({ "command": "false" }).to_string(),
        ),
        (
            "bash_command".to_string(),
            serde_json::json!({ "command": "false" }).to_string(),
        ),
    ])
    .await;
    config.model = "gpt-4o-mini".to_string();
    config.api_key = "test-key".to_string();
    config.api_transport = ApiTransport::ChatCompletions;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 3;

    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let thread_id = "thread-metacognitive-sunk-cost-block";

    engine
        .send_message_inner(
            Some(thread_id),
            "keep trying the shell command until it works",
            None,
            None,
            None,
            None,
            None,
            None,
            true,
        )
        .await
        .expect("send message should complete");

    let threads = engine.threads.read().await;
    let thread = threads.get(thread_id).expect("thread should exist");

    let blocked_tool_result = thread
        .messages
        .iter()
        .find(|message| {
            message.role == MessageRole::Tool
                && message.tool_name.as_deref() == Some("bash_command")
                && message
                    .content
                    .contains("Tool call blocked by meta-cognitive regulator before execution")
        })
        .cloned()
        .expect("expected blocked tool result for sunk-cost intervention");
    assert_eq!(blocked_tool_result.tool_status.as_deref(), Some("error"));

    assert!(thread.messages.iter().any(|message| {
        message.role == MessageRole::System
            && message
                .content
                .contains("Meta-cognitive intervention: tool call blocked before execution.")
    }));

    let bash_tool_messages = thread
        .messages
        .iter()
        .filter(|message| {
            message.role == MessageRole::Tool
                && message.tool_name.as_deref() == Some("bash_command")
        })
        .count();
    assert_eq!(
        bash_tool_messages, 3,
        "expected two real failures plus one metacognitive block, not an executed fourth call"
    );
    drop(threads);

    let model = engine.meta_cognitive_self_model.read().await;
    let sunk_cost = model
        .biases
        .iter()
        .find(|bias| bias.name == "sunk_cost")
        .expect("sunk_cost bias should exist");
    assert!(
        sunk_cost.occurrence_count >= 1,
        "sunk-cost intervention should reinforce bias occurrence count"
    );
}

#[tokio::test]
async fn metacognitive_confirmation_warning_restarts_with_reflection_before_tool_runs() {
    let root = tempdir().unwrap();
    let readable_path = root.path().join("confirm.txt");
    fs::write(&readable_path, "match confirmed\n").expect("write confirm file");

    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = PROVIDER_ID_OPENAI.to_string();
    config.base_url = spawn_scripted_tool_call_server(vec![
        (
            "read_file".to_string(),
            serde_json::json!({ "path": readable_path, "offset": 0, "limit": 1 }).to_string(),
        ),
        (
            "read_file".to_string(),
            serde_json::json!({ "path": readable_path, "offset": 0, "limit": 1 }).to_string(),
        ),
        (
            "search_files".to_string(),
            serde_json::json!({
                "path": root.path(),
                "pattern": "match",
                "file_pattern": "*.txt",
                "max_results": 5,
                "timeout_seconds": 30
            })
            .to_string(),
        ),
    ])
    .await;
    config.model = "gpt-4o-mini".to_string();
    config.api_key = "test-key".to_string();
    config.api_transport = ApiTransport::ChatCompletions;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 3;

    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let thread_id = "thread-metacognitive-confirmation-warning";

    engine
        .send_message_inner(
            Some(thread_id),
            "verify that the file still matches what we expect",
            None,
            None,
            None,
            None,
            None,
            None,
            true,
        )
        .await
        .expect("send message should complete");

    let threads = engine.threads.read().await;
    let thread = threads.get(thread_id).expect("thread should exist");

    let first_system_idx = thread
        .messages
        .iter()
        .position(|message| {
            message.role == MessageRole::System
                && message
                    .content
                    .contains("Meta-cognitive intervention: warning before tool execution.")
        })
        .expect("expected metacognitive warning system message");
    let first_search_idx = thread
        .messages
        .iter()
        .position(|message| {
            message.role == MessageRole::Tool
                && message.tool_name.as_deref() == Some("search_files")
        })
        .expect("expected eventual search_files tool execution");
    assert!(
        first_system_idx < first_search_idx,
        "reflection warning should be injected before the warned tool executes"
    );

    let search_files_results = thread
        .messages
        .iter()
        .filter(|message| {
            message.role == MessageRole::Tool
                && message.tool_name.as_deref() == Some("search_files")
                && message.tool_status.as_deref() == Some("done")
        })
        .count();
    assert_eq!(search_files_results, 1);
    drop(threads);

    let model = engine.meta_cognitive_self_model.read().await;
    assert!(
        model.calibration_offset < 0.0,
        "warning-level confirmation bias should feed a conservative calibration adjustment"
    );
}

#[tokio::test]
async fn metacognitive_learning_updates_persist_across_rehydrate() {
    let root = tempdir().unwrap();
    let manager = SessionManager::new_test(root.path()).await;
    let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;

    engine
        .reinforce_meta_cognitive_bias_occurrence("confirmation")
        .await;
    engine
        .apply_meta_cognitive_calibration_adjustment(
            -0.08,
            crate::agent::explanation::ConfidenceBand::Likely,
        )
        .await;

    let live_model = engine.meta_cognitive_self_model.read().await.clone();
    let live_confirmation = live_model
        .biases
        .iter()
        .find(|bias| bias.name == "confirmation")
        .expect("confirmation bias should exist")
        .occurrence_count;
    assert!(live_confirmation >= 1);
    assert!(live_model.calibration_offset < 0.0);

    let rehydrated = AgentEngine::new_test(
        SessionManager::new_test(root.path()).await,
        AgentConfig::default(),
        root.path(),
    )
    .await;
    rehydrated.hydrate().await.expect("hydrate should succeed");

    let loaded_model = rehydrated.meta_cognitive_self_model.read().await.clone();
    let loaded_confirmation = loaded_model
        .biases
        .iter()
        .find(|bias| bias.name == "confirmation")
        .expect("confirmation bias should exist after hydrate")
        .occurrence_count;

    assert!(
        loaded_model.calibration_offset < 0.0,
        "metacognitive calibration adjustments should persist across rehydrate"
    );
    assert!(
        loaded_confirmation >= 1,
        "bias occurrence reinforcement should persist across rehydrate"
    );
}

#[tokio::test]
async fn neutral_investigative_sequence_does_not_false_positive_confirmation_bias() {
    let root = tempdir().unwrap();
    let readable_path = root.path().join("neutral.txt");
    fs::write(&readable_path, "neutral sequence\n").expect("write neutral file");

    let manager = SessionManager::new_test(root.path()).await;
    let mut config = AgentConfig::default();
    config.provider = PROVIDER_ID_OPENAI.to_string();
    config.base_url = spawn_scripted_tool_call_server(vec![
        (
            "read_file".to_string(),
            serde_json::json!({ "path": readable_path, "offset": 0, "limit": 1 }).to_string(),
        ),
        (
            "search_files".to_string(),
            serde_json::json!({
                "path": root.path(),
                "pattern": "neutral",
                "file_pattern": "*.txt",
                "max_results": 5,
                "timeout_seconds": 30
            })
            .to_string(),
        ),
    ])
    .await;
    config.model = "gpt-4o-mini".to_string();
    config.api_key = "test-key".to_string();
    config.api_transport = ApiTransport::ChatCompletions;
    config.auto_retry = false;
    config.max_retries = 0;
    config.max_tool_loops = 2;

    let engine = AgentEngine::new_test(manager, config, root.path()).await;
    let thread_id = "thread-metacognitive-confirmation-neutral";

    engine
        .send_message_inner(
            Some(thread_id),
            "inspect the file and then search for the string",
            None,
            None,
            None,
            None,
            None,
            None,
            true,
        )
        .await
        .expect("send message should complete");

    let threads = engine.threads.read().await;
    let thread = threads.get(thread_id).expect("thread should exist");

    assert!(
        !thread.messages.iter().any(|message| {
            message.role == MessageRole::System
                && message
                    .content
                    .contains("Meta-cognitive intervention: warning before tool execution.")
                && message.content.contains("confirmation")
        }),
        "neutral evidence gathering should not false-positive as confirmation bias"
    );

    assert!(thread.messages.iter().any(|message| {
        message.role == MessageRole::Tool
            && message.tool_name.as_deref() == Some("search_files")
            && message.tool_status.as_deref() == Some("done")
    }));
}
