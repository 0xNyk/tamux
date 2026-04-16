    #[tokio::test]
    async fn managed_command_wait_can_be_cancelled() {
        let (_tx, mut rx) = broadcast::channel(4);
        let token = CancellationToken::new();
        token.cancel();

        let error =
            wait_for_managed_command_outcome(&mut rx, SessionId::nil(), "exec-1", 30, Some(&token))
                .await
                .err()
                .expect("managed wait should abort when cancellation is requested");

        assert!(error.to_string().contains("cancelled"));
    }

    #[tokio::test]
    async fn managed_command_wait_fails_when_session_exits() {
        let (tx, mut rx) = broadcast::channel(4);
        tx.send(DaemonMessage::SessionExited {
            id: SessionId::nil(),
            exit_code: Some(1),
        })
        .expect("session exit should broadcast");

        let error = wait_for_managed_command_outcome(&mut rx, SessionId::nil(), "exec-1", 30, None)
            .await
            .expect_err("managed wait should fail when the session exits");

        assert!(error.to_string().contains("session exited"));
    }

    #[tokio::test]
    async fn headless_shell_command_can_be_cancelled() {
        let root = tempfile::tempdir().unwrap();
        let session_manager = SessionManager::new_test(root.path()).await;
        let token = CancellationToken::new();
        let cancel = token.clone();

        let join = tokio::spawn(async move {
            execute_headless_shell_command(
                &serde_json::json!({
                    "command": "sleep 30",
                    "timeout_seconds": 30
                }),
                &session_manager,
                None,
                "bash_command",
                Some(token),
            )
            .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cancel.cancel();

        let error = join
            .await
            .expect("headless shell join should complete")
            .expect_err("headless shell should abort when cancellation is requested");

        assert!(error.to_string().contains("cancelled"));
    }

    #[test]
    fn resolve_skill_path_finds_generated_skill_by_stem() {
        let root = std::env::temp_dir().join(format!("tamux-skill-test-{}", uuid::Uuid::new_v4()));
        let generated = root.join("generated");
        fs::create_dir_all(&generated).expect("skill test directory should be created");
        let skill_path = generated.join("build-release.md");
        fs::write(&skill_path, "# Build release\n").expect("skill file should be written");

        let resolved = resolve_skill_path(&root, "build-release", None)
            .expect("generated skill should resolve");
        assert_eq!(resolved, skill_path);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_skill_path_prefers_selected_variant() {
        let root = std::env::temp_dir().join(format!("tamux-skill-test-{}", uuid::Uuid::new_v4()));
        let generated = root.join("generated");
        fs::create_dir_all(&generated).expect("skill test directory should be created");
        let canonical = generated.join("build-release.md");
        let frontend = generated.join("build-release--frontend.md");
        fs::write(&canonical, "# Build release\n").expect("canonical skill file should be written");
        fs::write(&frontend, "# Frontend build release\n")
            .expect("variant skill file should be written");

        let resolved = resolve_skill_path(
            &root,
            "build-release",
            Some(&SkillVariantRecord {
                variant_id: "variant-1".to_string(),
                skill_name: "build-release".to_string(),
                variant_name: "frontend".to_string(),
                relative_path: "generated/build-release--frontend.md".to_string(),
                parent_variant_id: Some("parent-1".to_string()),
                version: "v2.0".to_string(),
                context_tags: vec!["frontend".to_string()],
                use_count: 0,
                success_count: 0,
                failure_count: 0,
                fitness_score: 0.0,
                status: "active".to_string(),
                last_used_at: None,
                created_at: 0,
                updated_at: 0,
            }),
        )
        .expect("selected variant should resolve");
        assert_eq!(resolved, frontend);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_skill_path_maps_builtin_variant_record_to_seeded_superpowers_path() {
        let root = std::env::temp_dir().join(format!("tamux-skill-test-{}", uuid::Uuid::new_v4()));
        let canonical = root
            .join("development")
            .join("superpowers")
            .join("subagent-driven-development")
            .join("SKILL.md");
        fs::create_dir_all(canonical.parent().expect("skill parent"))
            .expect("skill test directory should be created");
        fs::write(&canonical, "# Subagent-Driven Development\n")
            .expect("canonical skill file should be written");

        let resolved = resolve_skill_path(
            &root,
            "subagent-driven-development",
            Some(&SkillVariantRecord {
                variant_id: "variant-1".to_string(),
                skill_name: "subagent-driven-development".to_string(),
                variant_name: "canonical".to_string(),
                relative_path: "builtin/superpowers/subagent-driven-development/SKILL.md"
                    .to_string(),
                parent_variant_id: None,
                version: "v1.0".to_string(),
                context_tags: vec!["rust".to_string()],
                use_count: 0,
                success_count: 0,
                failure_count: 0,
                fitness_score: 0.0,
                status: "active".to_string(),
                last_used_at: None,
                created_at: 0,
                updated_at: 0,
            }),
        )
        .expect("seeded superpowers skill should resolve from builtin variant path");
        assert_eq!(resolved, canonical);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn list_sessions_tool_requires_workspace_topology() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();

        let no_topology = get_available_tools(&config, &temp_dir, false);
        assert!(no_topology
            .iter()
            .all(|tool| tool.function.name != "list_sessions"));
        assert!(no_topology
            .iter()
            .any(|tool| tool.function.name == "list_terminals"));

        let with_topology = get_available_tools(&config, &temp_dir, true);
        assert!(with_topology
            .iter()
            .any(|tool| tool.function.name == "list_sessions"));
    }

    #[test]
    fn python_execute_tool_is_exposed_with_expected_schema() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);
        let python_execute = tools
            .iter()
            .find(|tool| tool.function.name == "python_execute")
            .expect("python_execute tool should be available");

        let properties = python_execute
            .function
            .parameters
            .get("properties")
            .expect("python_execute schema should expose properties");

        assert!(properties.get("code").is_some(), "schema should include code");
        assert!(properties.get("cwd").is_some(), "schema should include cwd");
        assert!(
            properties.get("timeout_seconds").is_some(),
            "schema should include timeout_seconds"
        );
        assert_eq!(
            python_execute
                .function
                .parameters
                .get("required")
                .and_then(|value| value.as_array())
                .map(|items| items.iter().filter_map(|item| item.as_str()).collect::<Vec<_>>()),
            Some(vec!["code"])
        );
    }

    #[test]
    fn current_datetime_tool_is_exposed() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);
        let current_datetime = tools
            .iter()
            .find(|tool| tool.function.name == "get_current_datetime")
            .expect("get_current_datetime tool should be available");

        let properties = current_datetime
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("get_current_datetime schema should expose properties object");

        assert!(properties.is_empty(), "datetime tool should not require arguments");
        assert!(current_datetime.function.parameters.get("required").is_none());
    }

    #[test]
    fn memory_read_tools_are_exposed_with_injection_aware_schema() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);

        for (tool_name, expected_layers) in [
            (
                "read_memory",
                vec![
                    "include_base_markdown",
                    "include_operator_profile_json",
                    "include_operator_model_summary",
                    "include_thread_structural_memory",
                ],
            ),
            (
                "read_user",
                vec![
                    "include_base_markdown",
                    "include_operator_profile_json",
                    "include_operator_model_summary",
                    "include_thread_structural_memory",
                ],
            ),
            (
                "read_soul",
                vec![
                    "include_base_markdown",
                    "include_operator_profile_json",
                    "include_operator_model_summary",
                    "include_thread_structural_memory",
                ],
            ),
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool.function.name == tool_name)
                .unwrap_or_else(|| panic!("{tool_name} should be available"));
            let properties = tool
                .function
                .parameters
                .get("properties")
                .and_then(|value| value.as_object())
                .unwrap_or_else(|| panic!("{tool_name} schema should expose properties"));

            assert!(
                properties.contains_key("include_already_injected"),
                "{tool_name} should expose include_already_injected"
            );
            assert!(
                properties.contains_key("limit_per_layer"),
                "{tool_name} should expose limit_per_layer"
            );
            for layer in expected_layers {
                assert!(
                    properties.contains_key(layer),
                    "{tool_name} should expose {layer}"
                );
                assert_eq!(
                    properties
                        .get(layer)
                        .and_then(|value| value.get("type"))
                        .and_then(|value| value.as_str()),
                    Some("boolean"),
                    "{tool_name} {layer} should be a boolean toggle"
                );
            }
        }
    }

    #[test]
    fn get_background_task_status_tool_is_exposed_with_expected_schema() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);
        let status_tool = tools
            .iter()
            .find(|tool| tool.function.name == "get_background_task_status")
            .expect("get_background_task_status tool should be available");

        let properties = status_tool
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("get_background_task_status schema should expose properties");

        assert!(
            properties.get("background_task_id").is_some(),
            "schema should include background_task_id"
        );
        let required = status_tool
            .function
            .parameters
            .get("required")
            .and_then(|value| value.as_array())
            .map(|items| items.iter().filter_map(|item| item.as_str()).collect::<Vec<_>>())
            .expect("get_background_task_status should define required fields");
        assert_eq!(required, vec!["background_task_id"]);
    }

    #[test]
    fn get_operation_status_tool_is_exposed_with_expected_schema() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);
        let status_tool = tools
            .iter()
            .find(|tool| tool.function.name == "get_operation_status")
            .expect("get_operation_status tool should be available");

        let properties = status_tool
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("get_operation_status schema should expose properties");

        assert!(
            properties.get("operation_id").is_some(),
            "schema should include operation_id"
        );
        let required = status_tool
            .function
            .parameters
            .get("required")
            .and_then(|value| value.as_array())
            .map(|items| items.iter().filter_map(|item| item.as_str()).collect::<Vec<_>>())
            .expect("get_operation_status should define required fields");
        assert_eq!(required, vec!["operation_id"]);
    }

    #[test]
    fn ask_questions_tool_is_exposed_with_compact_option_schema() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);
        let ask_questions = tools
            .iter()
            .find(|tool| tool.function.name == "ask_questions")
            .expect("ask_questions tool should be available");

        let properties = ask_questions
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("ask_questions schema should expose properties object");

        assert!(properties.get("content").is_some(), "schema should include content");
        assert!(properties.get("options").is_some(), "schema should include options");
        assert!(properties.get("session").is_some(), "schema should include session");

        let required = ask_questions
            .function
            .parameters
            .get("required")
            .and_then(|value| value.as_array())
            .map(|items| items.iter().filter_map(|item| item.as_str()).collect::<Vec<_>>())
            .expect("ask_questions should define required fields");
        assert_eq!(required, vec!["content", "options"]);
    }

    #[test]
    fn discover_skills_tool_is_exposed_with_expected_schema() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);
        let discover_skills = tools
            .iter()
            .find(|tool| tool.function.name == "discover_skills")
            .expect("discover_skills tool should be available");
        assert_eq!(
            discover_skills.function.description,
            "Find matching local skills fast."
        );

        let properties = discover_skills
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("discover_skills schema should expose properties object");

        assert!(properties.get("query").is_some(), "schema should include query");
        assert!(properties.get("limit").is_some(), "schema should include limit");
        assert!(properties.get("session").is_some(), "schema should include session");
        assert_eq!(
            properties
                .get("query")
                .and_then(|value| value.get("description"))
                .and_then(|value| value.as_str()),
            Some("Brief intent query, 3-6 words.")
        );

        let required = discover_skills
            .function
            .parameters
            .get("required")
            .and_then(|value| value.as_array())
            .map(|items| items.iter().filter_map(|item| item.as_str()).collect::<Vec<_>>())
            .expect("discover_skills should define required fields");
        assert_eq!(required, vec!["query"]);
    }

    #[test]
    fn list_tools_tool_is_exposed_with_paging_schema() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);
        let list_tools = tools
            .iter()
            .find(|tool| tool.function.name == "list_tools")
            .expect("list_tools tool should be available");

        let properties = list_tools
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("list_tools schema should expose properties object");

        assert!(properties.get("limit").is_some(), "schema should include limit");
        assert!(properties.get("offset").is_some(), "schema should include offset");
        assert!(list_tools.function.parameters.get("required").is_none());
    }

    #[test]
    fn tool_search_tool_is_exposed_with_query_schema() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);
        let tool_search = tools
            .iter()
            .find(|tool| tool.function.name == "tool_search")
            .expect("tool_search tool should be available");

        let properties = tool_search
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("tool_search schema should expose properties object");

        assert!(properties.get("query").is_some(), "schema should include query");
        assert!(properties.get("limit").is_some(), "schema should include limit");
        assert!(properties.get("offset").is_some(), "schema should include offset");

        let required = tool_search
            .function
            .parameters
            .get("required")
            .and_then(|value| value.as_array())
            .map(|items| items.iter().filter_map(|item| item.as_str()).collect::<Vec<_>>())
            .expect("tool_search should define required fields");
        assert_eq!(required, vec!["query"]);
    }

    #[test]
    fn provider_and_agent_listing_tools_are_exposed_with_expected_schema() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);

        let list_providers = tools
            .iter()
            .find(|tool| tool.function.name == "list_providers")
            .expect("list_providers tool should be available");
        let list_providers_properties = list_providers
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("list_providers schema should expose properties object");
        assert!(
            list_providers_properties.is_empty(),
            "list_providers should not require arguments"
        );

        let list_models = tools
            .iter()
            .find(|tool| tool.function.name == "list_models")
            .expect("list_models tool should be available");
        let list_models_properties = list_models
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("list_models schema should expose properties object");
        assert!(
            list_models_properties.contains_key("provider"),
            "list_models should require a provider"
        );
        let list_models_required = list_models
            .function
            .parameters
            .get("required")
            .and_then(|value| value.as_array())
            .map(|items| items.iter().filter_map(|item| item.as_str()).collect::<Vec<_>>())
            .expect("list_models should define required fields");
        assert_eq!(list_models_required, vec!["provider"]);

        let list_agents = tools
            .iter()
            .find(|tool| tool.function.name == "list_agents")
            .expect("list_agents tool should be available");
        let list_agents_properties = list_agents
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("list_agents schema should expose properties object");
        assert!(
            list_agents_properties.is_empty(),
            "list_agents should not require arguments"
        );
    }

    #[tokio::test]
    async fn switch_model_tool_is_only_exposed_to_svarog_scope() {
        let config = AgentConfig::default();
        let temp_dir = tempfile::tempdir().expect("tempdir");

        let svarog_tools = crate::agent::agent_identity::run_with_agent_scope(
            crate::agent::agent_identity::MAIN_AGENT_ID.to_string(),
            async { get_available_tools(&config, temp_dir.path(), false) },
        )
        .await;
        let switch_model = svarog_tools
            .iter()
            .find(|tool| tool.function.name == "switch_model")
            .expect("switch_model should be available to svarog");
        let switch_model_properties = switch_model
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("switch_model schema should expose properties object");
        for field in ["agent", "provider", "model"] {
            assert!(
                switch_model_properties.contains_key(field),
                "switch_model schema should include {field}"
            );
        }
        let switch_model_required = switch_model
            .function
            .parameters
            .get("required")
            .and_then(|value| value.as_array())
            .map(|items| items.iter().filter_map(|item| item.as_str()).collect::<Vec<_>>())
            .expect("switch_model should define required fields");
        assert_eq!(switch_model_required, vec!["agent", "provider", "model"]);

        let rarog_tools = crate::agent::agent_identity::run_with_agent_scope(
            crate::agent::agent_identity::CONCIERGE_AGENT_ID.to_string(),
            async { get_available_tools(&config, temp_dir.path(), false) },
        )
        .await;
        assert!(
            rarog_tools
                .iter()
                .all(|tool| tool.function.name != "switch_model"),
            "switch_model should be hidden outside svarog scope"
        );
    }

#[test]
fn apply_patch_tool_uses_top_level_object_schema() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);
        let apply_patch = tools
            .iter()
            .find(|tool| tool.function.name == "apply_patch")
            .expect("apply_patch tool should be available");

        assert_eq!(
            apply_patch.function.parameters.get("type"),
            Some(&serde_json::json!("object"))
        );
        for forbidden_key in ["oneOf", "anyOf", "allOf", "enum", "not"] {
            assert!(
                apply_patch.function.parameters.get(forbidden_key).is_none(),
                "apply_patch schema must not use top-level {forbidden_key}"
            );
        }

        let properties = apply_patch
            .function
            .parameters
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("apply_patch schema should expose top-level properties");

        assert!(properties.get("input").is_some(), "schema should include harness patch input");
        assert!(properties.get("patch").is_some(), "schema should include patch alias");
        assert!(
            properties.get("path").is_none(),
            "apply_patch schema should not advertise legacy exact-replacement path mode"
        );
        assert!(
            properties.get("edits").is_none(),
            "apply_patch schema should not advertise legacy exact-replacement edits mode"
        );
    }

    #[test]
    fn scrub_sensitive_redacts_common_api_key_lines() {
        let input = "openai api_key=sk-live-secret\nAuthorization: Bearer abc123secret";
        let scrubbed = crate::scrub::scrub_sensitive(input);
        assert!(!scrubbed.contains("sk-live-secret"));
        assert!(!scrubbed.contains("abc123secret"));
        assert!(scrubbed.contains("***REDACTED***"));
    }

    #[tokio::test]
    async fn divergent_tool_get_session_serializes_completion_fields() {
        let root = tempdir().expect("tempdir");
        let manager = SessionManager::new_test(root.path()).await;
        let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
        let session_id = engine
            .start_divergent_session("pick caching strategy", None, "thread-tool-div", None)
            .await
            .expect("start divergent session");
        let labels = {
            let sessions = engine.divergent_sessions.read().await;
            sessions
                .get(&session_id)
                .expect("session exists")
                .framings
                .iter()
                .map(|f| f.label.clone())
                .collect::<Vec<_>>()
        };
        for (idx, label) in labels.iter().enumerate() {
            engine
                .record_divergent_contribution(
                    &session_id,
                    label,
                    if idx == 0 {
                        "Prefer deterministic correctness-first approach"
                    } else {
                        "Prefer lower-latency pragmatic approach"
                    },
                )
                .await
                .expect("record contribution");
        }
        engine
            .complete_divergent_session(&session_id)
            .await
            .expect("complete divergent session");

        let response = execute_get_divergent_session(
            &serde_json::json!({ "session_id": session_id }),
            &engine,
        )
        .await
        .expect("tool execution should succeed");
        let payload: serde_json::Value =
            serde_json::from_str(&response).expect("tool payload should be valid JSON");
        assert_eq!(
            payload.get("status").and_then(|v| v.as_str()),
            Some("complete")
        );
        assert!(payload
            .get("tensions_markdown")
            .and_then(|v| v.as_str())
            .is_some_and(|value| !value.is_empty()));
        assert!(payload
            .get("mediator_prompt")
            .and_then(|v| v.as_str())
            .is_some_and(|value| !value.is_empty()));
    }

    #[tokio::test]
    async fn divergent_tool_get_session_in_progress_omits_completion_output() {
        let root = tempdir().expect("tempdir");
        let manager = SessionManager::new_test(root.path()).await;
        let engine = AgentEngine::new_test(manager, AgentConfig::default(), root.path()).await;
        let session_id = engine
            .start_divergent_session("evaluate rollout sequence", None, "thread-tool-div-2", None)
            .await
            .expect("start divergent session");

        let response = execute_get_divergent_session(
            &serde_json::json!({ "session_id": session_id }),
            &engine,
        )
        .await
        .expect("tool execution should succeed");
        let payload: serde_json::Value =
            serde_json::from_str(&response).expect("tool payload should be valid JSON");
        assert_eq!(
            payload.get("status").and_then(|v| v.as_str()),
            Some("running")
        );
        assert!(
            payload
                .get("tensions_markdown")
                .is_some_and(|v| v.is_null()),
            "in-progress session should not report tensions output"
        );
        assert!(
            payload.get("mediator_prompt").is_some_and(|v| v.is_null()),
            "in-progress session should not report mediator output"
        );
        let progress = payload
            .get("framing_progress")
            .expect("framing_progress should exist");
        assert_eq!(progress.get("completed").and_then(|v| v.as_u64()), Some(0));
    }

    #[tokio::test]
    async fn debate_tools_are_exposed_with_expected_schema() {
        let config = AgentConfig::default();
        let temp_dir = std::env::temp_dir();
        let tools = get_available_tools(&config, &temp_dir, false);

        for tool_name in [
            "run_debate",
            "get_debate_session",
            "append_debate_argument",
            "advance_debate_round",
            "complete_debate_session",
        ] {
            assert!(
                tools.iter().any(|tool| tool.function.name == tool_name),
                "{tool_name} tool should be available"
            );
        }
    }

    #[tokio::test]
    async fn debate_tool_run_and_get_session_round_trip() {
        let root = tempdir().expect("tempdir");
        let manager = SessionManager::new_test(root.path()).await;
        let mut config = AgentConfig::default();
        config.debate.enabled = true;
        let engine = AgentEngine::new_test(manager, config, root.path()).await;

        let response = execute_run_debate(
            &serde_json::json!({ "topic": "choose rollout strategy" }),
            &engine,
            "thread-tool-debate",
            None,
        )
        .await
        .expect("run_debate should succeed");
        let payload: serde_json::Value =
            serde_json::from_str(&response).expect("run_debate payload should be valid JSON");
        assert_eq!(payload.get("status").and_then(|v| v.as_str()), Some("started"));
        let session_id = payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .expect("session_id should exist")
            .to_string();

        let get_response = execute_get_debate_session(
            &serde_json::json!({ "session_id": session_id }),
            &engine,
        )
        .await
        .expect("get_debate_session should succeed");
        let get_payload: serde_json::Value = serde_json::from_str(&get_response)
            .expect("get_debate_session payload should be valid JSON");
        assert_eq!(
            get_payload.get("status").and_then(|v| v.as_str()),
            Some("in_progress")
        );
        assert_eq!(
            get_payload.get("current_round").and_then(|v| v.as_u64()),
            Some(2)
        );
        assert!(get_payload
            .get("arguments")
            .and_then(|v| v.as_array())
            .is_some_and(|arguments| {
                arguments.len() == 3
                    && arguments.iter().any(|argument| argument["role"] == "proponent")
                    && arguments.iter().any(|argument| argument["role"] == "skeptic")
                    && arguments.iter().any(|argument| argument["role"] == "synthesizer")
            }));
        assert!(get_payload
            .get("roles")
            .and_then(|v| v.as_array())
            .is_some_and(|roles| !roles.is_empty()));
    }

    #[tokio::test]
    async fn route_to_specialist_surfaces_routing_rationale_in_dispatch_result() {
        let root = tempdir().expect("tempdir");
        let manager = SessionManager::new_test(root.path()).await;
        let mut config = AgentConfig::default();
        config.routing.enabled = true;
        config.routing.method = crate::agent::types::RoutingMode::Probabilistic;
        let engine = AgentEngine::new_test(manager, config, root.path()).await;

        let response = execute_route_to_specialist(
            &serde_json::json!({
                "task_description": "Investigate the regression and summarize likely causes.",
                "capability_tags": ["research", "analysis"],
                "acceptance_criteria": "non_empty"
            }),
            &engine,
            "thread-routing-rationale",
            None,
        )
        .await
        .expect("route_to_specialist should succeed");

        let payload: serde_json::Value =
            serde_json::from_str(&response).expect("route_to_specialist payload should be valid JSON");
        assert_eq!(payload.get("status").and_then(|v| v.as_str()), Some("dispatched"));
        assert!(payload
            .get("routing_rationale")
            .and_then(|v| v.as_str())
            .is_some_and(|text| {
                text.contains("routing") || text.contains("fallback") || text.contains("score")
            }));
    }

    #[tokio::test]
    async fn send_slack_message_emits_gateway_ipc_request() {
        let root = tempdir().expect("tempdir");
        let manager = SessionManager::new_test(root.path()).await;
        let mut config = AgentConfig::default();
        config.gateway.enabled = true;
        let engine = AgentEngine::new_test(manager, config, root.path()).await;
        engine.init_gateway().await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        engine.set_gateway_ipc_sender(Some(tx)).await;

        let send_engine = engine.clone();
        let send_task = tokio::spawn(async move {
            execute_gateway_message(
                "send_slack_message",
                &serde_json::json!({
                    "channel": "C123",
                    "message": "hello from daemon",
                    "thread_ts": "1712345678.000100"
                }),
                &send_engine,
                &reqwest::Client::new(),
            )
            .await
        });

        let request = match timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("gateway send request should be emitted")
            .expect("gateway send request should exist")
        {
            DaemonMessage::GatewaySendRequest { request } => request,
            other => panic!("expected GatewaySendRequest, got {other:?}"),
        };
        assert_eq!(request.platform, "slack");
        assert_eq!(request.channel_id, "C123");
        assert_eq!(request.thread_id.as_deref(), Some("1712345678.000100"));
        assert_eq!(request.content, "hello from daemon");

        engine
            .complete_gateway_send_result(GatewaySendResult {
                correlation_id: request.correlation_id.clone(),
                platform: "slack".to_string(),
                channel_id: "C123".to_string(),
                requested_channel_id: Some("C123".to_string()),
                delivery_id: Some("1712345678.000200".to_string()),
                ok: true,
                error: None,
                completed_at_ms: 1,
            })
            .await;

        let result = send_task
            .await
            .expect("send task should join")
            .expect("send should succeed");
        assert_eq!(result, "Slack message sent to #C123");
    }

    #[tokio::test]
    async fn send_discord_message_emits_gateway_ipc_request() {
        let root = tempdir().expect("tempdir");
        let manager = SessionManager::new_test(root.path()).await;
        let mut config = AgentConfig::default();
        config.gateway.enabled = true;
        let engine = AgentEngine::new_test(manager, config, root.path()).await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        engine.set_gateway_ipc_sender(Some(tx)).await;

        let send_engine = engine.clone();
        let send_task = tokio::spawn(async move {
            execute_gateway_message(
                "send_discord_message",
                &serde_json::json!({
                    "user_id": "123456789",
                    "message": "discord reply",
                    "reply_to_message_id": "987654321"
                }),
                &send_engine,
                &reqwest::Client::new(),
            )
            .await
        });

        let request = match timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("gateway send request should be emitted")
            .expect("gateway send request should exist")
        {
            DaemonMessage::GatewaySendRequest { request } => request,
            other => panic!("expected GatewaySendRequest, got {other:?}"),
        };
        assert_eq!(request.platform, "discord");
        assert_eq!(request.channel_id, "user:123456789");
        assert_eq!(request.thread_id.as_deref(), Some("987654321"));
        assert_eq!(request.content, "discord reply");

        engine
            .complete_gateway_send_result(GatewaySendResult {
                correlation_id: request.correlation_id.clone(),
                platform: "discord".to_string(),
                channel_id: "user:123456789".to_string(),
                requested_channel_id: Some("user:123456789".to_string()),
                delivery_id: Some("delivery-1".to_string()),
                ok: true,
                error: None,
                completed_at_ms: 1,
            })
            .await;

        let result = send_task
            .await
            .expect("send task should join")
            .expect("send should succeed");
        assert_eq!(result, "Discord message sent to user:123456789");
    }
