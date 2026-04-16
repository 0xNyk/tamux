use crate::agent::types::AgentTask;

const MAX_RECURSIVE_SUBAGENT_DEPTH: u8 = 3;
const RECURSIVE_SUBAGENT_BUDGET_CURVE: [f64; 3] = [1.0, 0.6, 0.3];
const DEFAULT_SUBAGENT_MAX_DURATION_SECS: u64 = 300;
const DEFAULT_SUBAGENT_MAX_TOOL_CALLS: u32 = 50;

#[derive(Debug, Clone, Copy, Default)]
struct RequestedSubagentBudget {
    max_tokens: Option<u32>,
    max_wall_time_secs: Option<u64>,
    max_tool_calls: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
struct DerivedSubagentLimits {
    child_depth: u8,
    max_depth: u8,
    context_budget_tokens: Option<u32>,
    max_duration_secs: Option<u64>,
    max_tool_calls: Option<u32>,
}

fn budget_fraction_for_depth(depth: u8) -> f64 {
    RECURSIVE_SUBAGENT_BUDGET_CURVE
        .get(depth.saturating_sub(1) as usize)
        .copied()
        .unwrap_or(0.1)
}

pub(super) fn parse_subagent_containment_scope(scope: Option<&str>) -> Option<(u8, u8)> {
    let scope = scope?.trim();
    let payload = scope.strip_prefix("subagent-depth:")?;
    let (depth, max_depth) = payload.split_once('/')?;
    let depth = depth.trim().parse::<u8>().ok()?;
    let max_depth = max_depth.trim().parse::<u8>().ok()?;
    Some((depth, max_depth))
}

fn format_subagent_containment_scope(depth: u8, max_depth: u8) -> String {
    format!("subagent-depth:{depth}/{max_depth}")
}

pub(super) fn compute_task_delegation_depth(task: &AgentTask, all_tasks: &[AgentTask]) -> u8 {
    let mut depth = 0u8;
    let mut current_parent_id = task.parent_task_id.as_deref();
    while let Some(parent_id) = current_parent_id {
        depth = depth.saturating_add(1);
        current_parent_id = all_tasks
            .iter()
            .find(|candidate| candidate.id == parent_id)
            .and_then(|parent| parent.parent_task_id.as_deref());
    }
    depth
}

pub(super) fn effective_subagent_max_depth(task: &AgentTask, all_tasks: &[AgentTask]) -> u8 {
    parse_subagent_containment_scope(task.containment_scope.as_deref())
        .map(|(_, max_depth)| max_depth)
        .unwrap_or_else(|| compute_task_delegation_depth(task, all_tasks).max(1))
}

pub(super) fn extract_tool_call_limit(dsl: Option<&str>) -> Option<u32> {
    let mut remaining = dsl?;
    let mut limit = None::<u32>;
    let marker = "tool_call_count(";
    while let Some(idx) = remaining.find(marker) {
        let after = &remaining[idx + marker.len()..];
        let Some(close_idx) = after.find(')') else {
            break;
        };
        if let Ok(value) = after[..close_idx].trim().parse::<u32>() {
            limit = Some(limit.map_or(value, |current| current.min(value)));
        }
        remaining = &after[close_idx + 1..];
    }
    limit
}

fn merge_tool_call_limit(existing: Option<String>, max_tool_calls: Option<u32>) -> Option<String> {
    let Some(max_tool_calls) = max_tool_calls else {
        return existing;
    };
    match existing {
        Some(existing) if !existing.trim().is_empty() => {
            Some(format!("({existing}) OR tool_call_count({max_tool_calls})"))
        }
        _ => Some(format!("tool_call_count({max_tool_calls})")),
    }
}

fn parse_requested_subagent_budget(
    args: &serde_json::Value,
) -> Result<Option<RequestedSubagentBudget>> {
    let Some(budget) = args.get("budget") else {
        return Ok(None);
    };
    let Some(budget) = budget.as_object() else {
        anyhow::bail!("'budget' must be an object when provided");
    };
    let max_tokens = budget
        .get("max_tokens")
        .and_then(|value| value.as_u64())
        .map(|value| value.min(u32::MAX as u64) as u32);
    let max_wall_time_secs = budget
        .get("max_wall_time_secs")
        .and_then(|value| value.as_u64());
    let max_tool_calls = budget
        .get("max_tool_calls")
        .and_then(|value| value.as_u64())
        .map(|value| value.min(u32::MAX as u64) as u32);

    Ok(Some(RequestedSubagentBudget {
        max_tokens,
        max_wall_time_secs,
        max_tool_calls,
    }))
}

fn derive_subagent_limits(
    current_task: Option<&AgentTask>,
    all_tasks: &[AgentTask],
    requested_max_depth: Option<u8>,
    requested_budget: Option<RequestedSubagentBudget>,
    default_context_window_tokens: u32,
) -> Result<DerivedSubagentLimits> {
    let parent_depth = current_task
        .map(|task| compute_task_delegation_depth(task, all_tasks))
        .unwrap_or(0);
    let parent_max_depth = current_task
        .map(|task| effective_subagent_max_depth(task, all_tasks))
        .unwrap_or(1);
    let child_depth = parent_depth.saturating_add(1);
    if child_depth > MAX_RECURSIVE_SUBAGENT_DEPTH {
        anyhow::bail!(
            "recursive subagent depth limit exceeded: requested depth {} but hard cap is {}",
            child_depth,
            MAX_RECURSIVE_SUBAGENT_DEPTH
        );
    }

    let max_depth = requested_max_depth.unwrap_or(parent_max_depth);
    if max_depth == 0 {
        anyhow::bail!("'max_depth' must be at least 1");
    }
    if max_depth > MAX_RECURSIVE_SUBAGENT_DEPTH {
        anyhow::bail!(
            "requested max_depth {} exceeds hard cap {}",
            max_depth,
            MAX_RECURSIVE_SUBAGENT_DEPTH
        );
    }
    if max_depth < child_depth {
        anyhow::bail!(
            "requested max_depth {} is below child delegation depth {}",
            max_depth,
            child_depth
        );
    }
    if current_task.is_some() && max_depth > parent_max_depth {
        anyhow::bail!(
            "requested max_depth {} exceeds parent allowance {}",
            max_depth,
            parent_max_depth
        );
    }

    let fraction = budget_fraction_for_depth(child_depth);
    let derived_context_budget = {
        let base = (default_context_window_tokens as f64 * fraction).round() as u32;
        current_task
            .and_then(|task| task.context_budget_tokens)
            .map(|parent: u32| parent.min(base))
            .or(Some(base.max(256)))
    };
    let derived_max_duration = {
        let base = (DEFAULT_SUBAGENT_MAX_DURATION_SECS as f64 * fraction).round() as u64;
        current_task
            .and_then(|task| task.max_duration_secs)
            .map(|parent: u64| parent.min(base.max(30)))
            .or(Some(base.max(30)))
    };
    let derived_max_tool_calls = {
        let base = (DEFAULT_SUBAGENT_MAX_TOOL_CALLS as f64 * fraction).round() as u32;
        current_task
            .and_then(|task| extract_tool_call_limit(task.termination_conditions.as_deref()))
            .map(|parent: u32| parent.min(base.max(1)))
            .or(Some(base.max(1)))
    };

    let requested_budget = requested_budget.unwrap_or_default();
    if let Some(current_task) = current_task {
        if let (Some(requested), Some(parent)) = (
            requested_budget.max_tokens,
            current_task.context_budget_tokens,
        ) {
            if requested > parent {
                anyhow::bail!(
                    "requested budget.max_tokens {} exceeds parent context budget {}",
                    requested,
                    parent
                );
            }
        }
        if let (Some(requested), Some(parent)) = (
            requested_budget.max_wall_time_secs,
            current_task.max_duration_secs,
        ) {
            if requested > parent {
                anyhow::bail!(
                    "requested budget.max_wall_time_secs {} exceeds parent max_duration_secs {}",
                    requested,
                    parent
                );
            }
        }
        if let (Some(requested), Some(parent)) = (
            requested_budget.max_tool_calls,
            extract_tool_call_limit(current_task.termination_conditions.as_deref()),
        ) {
            if requested > parent {
                anyhow::bail!(
                    "requested budget.max_tool_calls {} exceeds parent tool-call budget {}",
                    requested,
                    parent
                );
            }
        }
    }

    Ok(DerivedSubagentLimits {
        child_depth,
        max_depth,
        context_budget_tokens: requested_budget.max_tokens.or(derived_context_budget),
        max_duration_secs: requested_budget.max_wall_time_secs.or(derived_max_duration),
        max_tool_calls: requested_budget.max_tool_calls.or(derived_max_tool_calls),
    })
}

async fn execute_spawn_subagent(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: &str,
    task_id: Option<&str>,
    session_manager: &Arc<SessionManager>,
    preferred_session_id: Option<SessionId>,
    event_tx: &broadcast::Sender<AgentEvent>,
) -> Result<String> {
    fn contains_hidden_weles_fields(args: &serde_json::Value) -> bool {
        [
            "weles_internal_scope",
            "weles_tool_name",
            "weles_tool_args",
            "weles_security_level",
            "weles_suspicion_reasons",
        ]
        .iter()
        .any(|key| args.get(key).is_some())
    }

    if contains_hidden_weles_fields(args) {
        anyhow::bail!(
            "daemon-owned WELES governance fields are unavailable to normal spawn_subagent callers"
        );
    }

    let title = args
        .get("title")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'title' argument"))?
        .to_string();
    let description = args
        .get("description")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'description' argument"))?
        .to_string();
    let provider_override = args
        .get("provider")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let model_override = args
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if model_override.is_some() && provider_override.is_none() {
        anyhow::bail!(
            "'model' requires an explicit 'provider'. Use `list_providers` first, then `list_models` for the chosen provider."
        );
    }
    let runtime = normalize_task_runtime(args.get("runtime").and_then(|value| value.as_str()))?;
    if runtime != "daemon" {
        let status = agent
            .external_agent_status(&runtime)
            .await
            .ok_or_else(|| anyhow::anyhow!("runtime {runtime} is not configured"))?;
        if !status.available {
            anyhow::bail!("runtime {runtime} is not available on this machine");
        }
    }

    let priority = args
        .get("priority")
        .and_then(|value| value.as_str())
        .unwrap_or("normal");
    let command = args
        .get("command")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let dependencies = args
        .get("dependencies")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let existing_tasks = agent.list_tasks().await;
    let task_snapshot = if let Some(current_task_id) = task_id {
        existing_tasks
            .iter()
            .find(|task| task.id == current_task_id)
            .cloned()
    } else {
        None
    };
    let requested_max_depth = args
        .get("max_depth")
        .and_then(|value| value.as_u64())
        .map(|value| value.min(u8::MAX as u64) as u8);
    let requested_budget = parse_requested_subagent_budget(args)?;
    let scheduled_at = parse_scheduled_at(args)?;
    let default_context_window_tokens = agent.config.read().await.context_window_tokens;
    let derived_limits = derive_subagent_limits(
        task_snapshot.as_ref(),
        &existing_tasks,
        requested_max_depth,
        requested_budget,
        default_context_window_tokens,
    )?;

    let mut chosen_session = args
        .get("session")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let mut allocated_lane_summary = None;
    if chosen_session.is_none() && scheduled_at.is_none() {
        let default_source_session = task_snapshot
            .as_ref()
            .and_then(|task| task.session_id.as_deref())
            .map(ToOwned::to_owned);
        let lane_request = serde_json::json!({
            "session": default_source_session,
            "cwd": args.get("cwd").and_then(|value| value.as_str()),
            "pane_name": format!("Subagent · {}", title.chars().take(24).collect::<String>()),
        });
        if let Ok(lane) = allocate_terminal_lane(
            &lane_request,
            session_manager,
            preferred_session_id,
            event_tx,
            "Subagent",
        )
        .await
        {
            chosen_session = Some(lane.session_id.to_string());
            allocated_lane_summary = Some(format!(
                "allocated terminal {} in workspace {} as \"{}\"",
                lane.session_id, lane.workspace_id, lane.pane_name
            ));
        }
    }

    let mut subagent = agent
        .enqueue_task(
            title.clone(),
            description,
            priority,
            command,
            chosen_session,
            dependencies,
            scheduled_at,
            "subagent",
            task_snapshot
                .as_ref()
                .and_then(|task| task.goal_run_id.clone()),
            task_id.map(ToOwned::to_owned),
            Some(thread_id.to_string()),
            Some(runtime.clone()),
        )
        .await;

    if let Some(provider_id) = provider_override.as_deref() {
        validate_spawn_provider_override(agent, provider_id, model_override.as_deref()).await?;
        subagent.override_provider = Some(provider_id.to_string());
        subagent.override_model = model_override.clone();
    }

    // Look up a matching SubAgentDefinition by title/name and apply overrides.
    {
        let effective_sub_agents = agent.list_sub_agents().await;
        let matched_def = effective_sub_agents
            .iter()
            .find(|sa| sa.enabled && sa.matches_spawn_request(&title));
        if let Some(def) = matched_def {
            if let Some(reason) = def.protected_reason.as_deref() {
                anyhow::bail!(
                    "protected sub-agent '{}' is reserved and cannot be spawned via spawn_subagent: {}",
                    def.name,
                    reason
                );
            }
        }
        if let Some(def) = matched_def {
            subagent.override_provider = Some(def.provider.clone());
            subagent.override_model = Some(def.model.clone());
            subagent.override_system_prompt = def.system_prompt.clone();
            subagent.sub_agent_def_id = Some(def.id.clone());

            if def.tool_whitelist.is_some() {
                subagent.tool_whitelist = def.tool_whitelist.clone();
            }
            if def.tool_blacklist.is_some() {
                subagent.tool_blacklist = def.tool_blacklist.clone();
            }
            if def.context_budget_tokens.is_some() {
                subagent.context_budget_tokens = def.context_budget_tokens;
            }
            if def.max_duration_secs.is_some() {
                subagent.max_duration_secs = def.max_duration_secs;
            }
            if def.supervisor_config.is_some() {
                subagent.supervisor_config = def.supervisor_config.clone();
            }
            // Persist the updated task fields.
            let mut tasks = agent.tasks.lock().await;
            if let Some(existing) = tasks.iter_mut().find(|t| t.id == subagent.id) {
                *existing = subagent.clone();
            }
            drop(tasks);
            agent.persist_tasks().await;
        }
    }
    if let Some(parent_task_id) = task_id {
        agent
            .register_subagent_collaboration(parent_task_id, &subagent)
            .await;
    }

    subagent.containment_scope = Some(format_subagent_containment_scope(
        derived_limits.child_depth,
        derived_limits.max_depth,
    ));
    subagent.context_budget_tokens = derived_limits.context_budget_tokens;
    subagent.max_duration_secs = derived_limits.max_duration_secs;
    if subagent.context_budget_tokens.is_some() {
        subagent.context_overflow_action = Some(crate::agent::types::ContextOverflowAction::Error);
    }
    subagent.termination_conditions = merge_tool_call_limit(
        subagent.termination_conditions.clone(),
        derived_limits.max_tool_calls,
    );

    let persona_prompt = if subagent.sub_agent_def_id.as_deref()
        == Some(crate::agent::agent_identity::WELES_BUILTIN_SUBAGENT_ID)
    {
        let scope = subagent
            .override_system_prompt
            .as_deref()
            .and_then(crate::agent::weles_governance::parse_weles_internal_override_payload)
            .map(|(scope, _, _)| scope)
            .unwrap_or_else(|| crate::agent::agent_identity::WELES_GOVERNANCE_SCOPE.to_string());
        crate::agent::agent_identity::build_weles_persona_prompt(&scope)
    } else {
        build_spawned_persona_prompt(&subagent.id)
    };
    subagent.override_system_prompt = Some(match subagent.override_system_prompt.take() {
        Some(existing) if !existing.trim().is_empty() => {
            format!("{persona_prompt}\n\n{existing}")
        }
        _ => persona_prompt.clone(),
    });
    {
        let mut tasks = agent.tasks.lock().await;
        if let Some(existing) = tasks.iter_mut().find(|t| t.id == subagent.id) {
            *existing = subagent.clone();
        }
    }
    {
        let mut trusted = agent.trusted_weles_tasks.write().await;
        trusted.insert(subagent.id.clone());
    }
    agent.persist_tasks().await;

    let lane_suffix = allocated_lane_summary
        .map(|value| format!("\nDedicated lane: {value}"))
        .unwrap_or_default();
    let persona_suffix = extract_persona_name(subagent.override_system_prompt.as_deref())
        .map(|name| format!("\nAssigned persona: {name}"))
        .unwrap_or_default();
    let def_suffix = subagent
        .sub_agent_def_id
        .as_ref()
        .map(|id| format!("\nMatched sub-agent definition: {id}"))
        .unwrap_or_default();
    let depth_suffix = format!(
        "\nDelegation depth: {}/{}",
        derived_limits.child_depth, derived_limits.max_depth
    );
    let budget_suffix = format!(
        "\nBudget: {} tokens, {}s, {} tool calls",
        derived_limits.context_budget_tokens.unwrap_or(0),
        derived_limits.max_duration_secs.unwrap_or(0),
        derived_limits.max_tool_calls.unwrap_or(0)
    );
    Ok(format!(
        "Spawned subagent {} with runtime {}.{}{}{}{budget_suffix}{def_suffix}",
        subagent.id, runtime, lane_suffix, persona_suffix, depth_suffix
    ))
}

async fn execute_fetch_authenticated_providers(agent: &AgentEngine) -> Result<String> {
    let authenticated = agent
        .get_provider_auth_states()
        .await
        .into_iter()
        .filter(|state| state.authenticated)
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&authenticated)
        .map_err(|error| anyhow::anyhow!("failed to serialize authenticated providers: {error}"))
}

async fn execute_list_providers(agent: &AgentEngine) -> Result<String> {
    let providers = agent.get_provider_auth_states().await;
    serde_json::to_string_pretty(&providers)
        .map_err(|error| anyhow::anyhow!("failed to serialize providers: {error}"))
}

async fn execute_fetch_provider_models(
    args: &serde_json::Value,
    agent: &AgentEngine,
) -> Result<String> {
    let provider_id = args
        .get("provider")
        .or_else(|| args.get("provider_id"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'provider' argument"))?;

    let provider_config = resolve_authenticated_provider_config(agent, provider_id).await?;
    let models = crate::agent::llm_client::fetch_models(
        provider_id,
        &provider_config.base_url,
        &provider_config.api_key,
    )
    .await
    .map_err(|error| {
        anyhow::anyhow!(
            "failed to fetch models for provider '{}': {}. Check `list_providers` and try `list_models` again after fixing auth/base URL.",
            provider_id,
            error
        )
    })?;

    serde_json::to_string_pretty(&models)
        .map_err(|error| anyhow::anyhow!("failed to serialize provider models: {error}"))
}

async fn execute_list_models(args: &serde_json::Value, agent: &AgentEngine) -> Result<String> {
    execute_fetch_provider_models(args, agent).await
}

async fn execute_list_agents(agent: &AgentEngine) -> Result<String> {
    let config = agent.get_config().await;
    let mut rows = vec![
        serde_json::json!({
            "agent": amux_protocol::AGENT_HANDLE_SVAROG,
            "name": MAIN_AGENT_NAME,
            "kind": "main",
            "provider": config.provider,
            "model": config.model,
            "switchable": true
        }),
        serde_json::json!({
            "agent": CONCIERGE_AGENT_ID,
            "name": CONCIERGE_AGENT_NAME,
            "kind": "concierge",
            "provider": config.concierge.provider.clone().unwrap_or_else(|| config.provider.clone()),
            "model": config.concierge.model.clone().unwrap_or_else(|| config.model.clone()),
            "switchable": true
        }),
        serde_json::json!({
            "agent": crate::agent::agent_identity::WELES_AGENT_ID,
            "name": crate::agent::agent_identity::WELES_AGENT_NAME,
            "kind": "builtin",
            "provider": config.builtin_sub_agents.weles.provider.clone().unwrap_or_else(|| config.provider.clone()),
            "model": config.builtin_sub_agents.weles.model.clone().unwrap_or_else(|| config.model.clone()),
            "switchable": true
        }),
        serde_json::json!({
            "agent": crate::agent::agent_identity::SWAROZYC_AGENT_ID,
            "name": crate::agent::agent_identity::SWAROZYC_AGENT_NAME,
            "kind": "builtin",
            "provider": config.builtin_sub_agents.swarozyc.provider.clone(),
            "model": config.builtin_sub_agents.swarozyc.model.clone(),
            "switchable": true
        }),
        serde_json::json!({
            "agent": crate::agent::agent_identity::RADOGOST_AGENT_ID,
            "name": crate::agent::agent_identity::RADOGOST_AGENT_NAME,
            "kind": "builtin",
            "provider": config.builtin_sub_agents.radogost.provider.clone(),
            "model": config.builtin_sub_agents.radogost.model.clone(),
            "switchable": true
        }),
        serde_json::json!({
            "agent": crate::agent::agent_identity::DOMOWOJ_AGENT_ID,
            "name": crate::agent::agent_identity::DOMOWOJ_AGENT_NAME,
            "kind": "builtin",
            "provider": config.builtin_sub_agents.domowoj.provider.clone(),
            "model": config.builtin_sub_agents.domowoj.model.clone(),
            "switchable": true
        }),
        serde_json::json!({
            "agent": crate::agent::agent_identity::SWIETOWIT_AGENT_ID,
            "name": crate::agent::agent_identity::SWIETOWIT_AGENT_NAME,
            "kind": "builtin",
            "provider": config.builtin_sub_agents.swietowit.provider.clone(),
            "model": config.builtin_sub_agents.swietowit.model.clone(),
            "switchable": true
        }),
        serde_json::json!({
            "agent": crate::agent::agent_identity::PERUN_AGENT_ID,
            "name": crate::agent::agent_identity::PERUN_AGENT_NAME,
            "kind": "builtin",
            "provider": config.builtin_sub_agents.perun.provider.clone(),
            "model": config.builtin_sub_agents.perun.model.clone(),
            "switchable": true
        }),
        serde_json::json!({
            "agent": crate::agent::agent_identity::MOKOSH_AGENT_ID,
            "name": crate::agent::agent_identity::MOKOSH_AGENT_NAME,
            "kind": "builtin",
            "provider": config.builtin_sub_agents.mokosh.provider.clone(),
            "model": config.builtin_sub_agents.mokosh.model.clone(),
            "switchable": true
        }),
        serde_json::json!({
            "agent": crate::agent::agent_identity::DAZHBOG_AGENT_ID,
            "name": crate::agent::agent_identity::DAZHBOG_AGENT_NAME,
            "kind": "builtin",
            "provider": config.builtin_sub_agents.dazhbog.provider.clone(),
            "model": config.builtin_sub_agents.dazhbog.model.clone(),
            "switchable": true
        }),
    ];

    for sub_agent in agent.list_sub_agents().await {
        if sub_agent.id == crate::agent::agent_identity::WELES_BUILTIN_SUBAGENT_ID {
            continue;
        }
        rows.push(serde_json::json!({
            "agent": sub_agent.id,
            "name": sub_agent.name,
            "kind": if sub_agent.builtin { "builtin" } else { "subagent" },
            "provider": sub_agent.provider,
            "model": sub_agent.model,
            "switchable": sub_agent.enabled
        }));
    }

    serde_json::to_string_pretty(&rows)
        .map_err(|error| anyhow::anyhow!("failed to serialize agent targets: {error}"))
}

async fn execute_list_participants(agent: &AgentEngine, thread_id: &str) -> Result<String> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        anyhow::bail!("list_participants requires a thread context");
    }
    if crate::agent::agent_identity::is_internal_dm_thread(thread_id)
        || crate::agent::agent_identity::is_participant_playground_thread(thread_id)
        || crate::agent::is_internal_handoff_thread(thread_id)
    {
        anyhow::bail!("list_participants is only available on visible operator threads");
    }

    let rows = agent
        .list_thread_participants(thread_id)
        .await
        .into_iter()
        .map(|participant| {
            serde_json::json!({
                "agent": participant.agent_id,
                "name": participant.agent_name,
                "instruction": participant.instruction,
                "status": match participant.status {
                    crate::agent::ThreadParticipantStatus::Active => "active",
                    crate::agent::ThreadParticipantStatus::Inactive => "inactive",
                },
                "created_at": participant.created_at,
                "updated_at": participant.updated_at,
                "deactivated_at": participant.deactivated_at,
                "last_contribution_at": participant.last_contribution_at,
            })
        })
        .collect::<Vec<_>>();

    serde_json::to_string_pretty(&rows)
        .map_err(|error| anyhow::anyhow!("failed to serialize thread participants: {error}"))
}

async fn execute_switch_model(args: &serde_json::Value, agent: &AgentEngine) -> Result<String> {
    if current_agent_scope_id() != MAIN_AGENT_ID {
        anyhow::bail!("`switch_model` is only available to svarog");
    }

    let target_agent = args
        .get("agent")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'agent' argument"))?;
    let provider_id = args
        .get("provider")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'provider' argument"))?;
    let model = args
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'model' argument"))?;

    agent
        .switch_agent_provider_model_json(target_agent, provider_id, model)
        .await?;

    Ok(format!(
        "Updated agent '{}' to use provider '{}' with model '{}'.",
        target_agent, provider_id, model
    ))
}

async fn validate_spawn_provider_override(
    agent: &AgentEngine,
    provider_id: &str,
    model_override: Option<&str>,
) -> Result<()> {
    let provider_config = resolve_authenticated_provider_config(agent, provider_id).await?;
    let Some(model_override) = model_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };

    let models = crate::agent::llm_client::fetch_models(
        provider_id,
        &provider_config.base_url,
        &provider_config.api_key,
    )
    .await
    .map_err(|error| {
        anyhow::anyhow!(
            "failed to validate model '{}' for provider '{}': {}. Use `list_models` to inspect the provider's available models first.",
            model_override,
            provider_id,
            error
        )
    })?;

    if !models.is_empty() && !models.iter().any(|model| model.id == model_override) {
        anyhow::bail!(
            "model '{}' is not available for authenticated provider '{}'. Use `list_models` to choose one of the returned models.",
            model_override,
            provider_id
        );
    }

    Ok(())
}

async fn resolve_authenticated_provider_config(
    agent: &AgentEngine,
    provider_id: &str,
) -> Result<crate::agent::types::ProviderConfig> {
    let auth_state = agent
        .get_provider_auth_states()
        .await
        .into_iter()
        .find(|state| state.provider_id == provider_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unknown provider '{}'. Use `list_providers` to inspect available authenticated providers.",
                provider_id
            )
        })?;
    if !auth_state.authenticated {
        anyhow::bail!(
            "provider '{}' is not authenticated. Use `list_providers` to inspect which providers are ready before spawning a subagent.",
            provider_id
        );
    }

    let config = agent.get_config().await;
    agent
        .resolve_sub_agent_provider_config(&config, provider_id)
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to resolve provider '{}': {}. Use `list_providers` to verify the provider configuration.",
                provider_id,
                error
            )
        })
}

pub(in crate::agent) async fn spawn_weles_internal_subagent(
    agent: &AgentEngine,
    thread_id: &str,
    parent_task_id: Option<&str>,
    scope: &str,
    tool_name: &str,
    tool_args: &serde_json::Value,
    security_level: SecurityLevel,
    suspicion_reasons: &[String],
) -> Result<crate::agent::types::AgentTask> {
    if !crate::agent::agent_identity::is_weles_internal_scope(scope) {
        anyhow::bail!("invalid WELES internal scope: {scope}");
    }

    let title = "WELES".to_string();
    let description = match scope {
        crate::agent::agent_identity::WELES_VITALITY_SCOPE => {
            "Internal vitality/self-health review".to_string()
        }
        _ => format!("Internal governance review for {tool_name}"),
    };
    let task_snapshot = if let Some(current_task_id) = parent_task_id {
        let tasks = agent.tasks.lock().await;
        tasks
            .iter()
            .find(|task| task.id == current_task_id)
            .cloned()
    } else {
        None
    };
    let effective_sub_agents = agent.list_sub_agents().await;
    let def = effective_sub_agents
        .iter()
        .find(|sa| sa.id == crate::agent::agent_identity::WELES_BUILTIN_SUBAGENT_ID)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing daemon-owned WELES definition"))?;
    let task_health_signals =
        crate::agent::weles_governance::build_task_health_signals(task_snapshot.as_ref());

    let inspection_context = serde_json::json!({
        "tool_name": tool_name,
        "tool_args": tool_args,
        "security_level": match security_level {
            SecurityLevel::Highest => "highest",
            SecurityLevel::Moderate => "moderate",
            SecurityLevel::Lowest => "lowest",
            SecurityLevel::Yolo => "yolo",
        },
        "suspicion_reasons": suspicion_reasons,
        "task_health_signals": task_health_signals,
    });
    let internal_payload = crate::agent::weles_governance::build_weles_internal_override_payload(
        scope,
        &inspection_context,
    )
    .ok_or_else(|| anyhow::anyhow!("failed to build WELES internal payload"))?;
    let mut subagent = agent
        .enqueue_task(
            title,
            description,
            "high",
            None,
            task_snapshot
                .as_ref()
                .and_then(|task| task.session_id.clone()),
            Vec::new(),
            None,
            "subagent",
            task_snapshot
                .as_ref()
                .and_then(|task| task.goal_run_id.clone()),
            parent_task_id.map(ToOwned::to_owned),
            Some(thread_id.to_string()),
            Some("daemon".to_string()),
        )
        .await;
    subagent.override_provider = Some(def.provider.clone());
    subagent.override_model = Some(def.model.clone());
    subagent.sub_agent_def_id = Some(def.id.clone());
    subagent.override_system_prompt = Some(format!(
        "{}\n\n{}",
        crate::agent::agent_identity::build_weles_persona_prompt(scope),
        internal_payload
    ));
    if def.tool_whitelist.is_some() {
        subagent.tool_whitelist = def.tool_whitelist.clone();
    }
    if def.tool_blacklist.is_some() {
        subagent.tool_blacklist = def.tool_blacklist.clone();
    }
    if def.context_budget_tokens.is_some() {
        subagent.context_budget_tokens = def.context_budget_tokens;
    }
    if def.max_duration_secs.is_some() {
        subagent.max_duration_secs = def.max_duration_secs;
    }
    if def.supervisor_config.is_some() {
        subagent.supervisor_config = def.supervisor_config.clone();
    }

    {
        let mut tasks = agent.tasks.lock().await;
        if let Some(existing) = tasks.iter_mut().find(|t| t.id == subagent.id) {
            *existing = subagent.clone();
        }
    }
    {
        let mut trusted = agent.trusted_weles_tasks.write().await;
        trusted.insert(subagent.id.clone());
    }
    agent.persist_tasks().await;

    if let Some(parent_task_id) = parent_task_id {
        agent
            .register_subagent_collaboration(parent_task_id, &subagent)
            .await;
    }

    Ok(subagent)
}

async fn execute_route_to_specialist(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: &str,
    task_id: Option<&str>,
) -> Result<String> {
    let task_description = args
        .get("task_description")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'task_description' argument"))?
        .to_string();
    let capability_tags: Vec<String> = args
        .get("capability_tags")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if capability_tags.is_empty() {
        anyhow::bail!("'capability_tags' must be a non-empty array of strings");
    }
    let acceptance_criteria = args
        .get("acceptance_criteria")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("non_empty")
        .to_string();
    let current_depth: u8 = args
        .get("current_depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u8;

    match agent
        .route_handoff(
            &task_description,
            &capability_tags,
            task_id,
            None, // goal_run_id
            thread_id,
            &acceptance_criteria,
            current_depth,
        )
        .await
    {
        Ok(result) => {
            let response = serde_json::json!({
                "status": "dispatched",
                "task_id": result.task_id,
                "specialist_name": result.specialist_name,
                "specialist_profile_id": result.specialist_profile_id,
                "handoff_log_id": result.handoff_log_id,
                "context_bundle_tokens": result.context_bundle_tokens,
                "routing_method": result.routing_method.as_str(),
                "routing_score": result.routing_score,
                "fallback_used": result.fallback_used,
                "routing_rationale": result.routing_rationale,
            });
            Ok(serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string()))
        }
        Err(e) => Ok(format!("Handoff failed: {e}")),
    }
}

async fn execute_run_divergent(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: &str,
    task_id: Option<&str>,
) -> Result<String> {
    let problem_statement = args
        .get("problem_statement")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'problem_statement' argument"))?
        .to_string();

    // Parse optional custom framings
    let custom_framings = args
        .get("custom_framings")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let label = item.get("label")?.as_str()?.trim().to_string();
                    let prompt = item
                        .get("system_prompt_override")?
                        .as_str()?
                        .trim()
                        .to_string();
                    if label.is_empty() || prompt.is_empty() {
                        return None;
                    }
                    Some(super::handoff::divergent::Framing {
                        label,
                        system_prompt_override: prompt,
                        task_id: None,
                        contribution_id: None,
                    })
                })
                .collect::<Vec<_>>()
        })
        .filter(|v| v.len() >= 2);

    // Derive goal_run_id from context if available
    let goal_run_id = task_id.and_then(|_tid| {
        // Convention: goal-sourced tasks have source "goal_run"
        // but we don't have direct access here; pass None
        // and let start_divergent_session work without it
        None::<&str>
    });

    match agent
        .start_divergent_session(&problem_statement, custom_framings, thread_id, goal_run_id)
        .await
    {
        Ok(session_id) => {
            let response = serde_json::json!({
                "status": "started",
                "session_id": session_id,
                "problem_statement": problem_statement,
                "message": "Divergent session started. Parallel framings are being processed. Use get_divergent_session with this session_id to retrieve progress, tensions, and mediator output."
            });
            Ok(serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string()))
        }
        Err(e) => Ok(format!("Divergent session failed: {e}")),
    }
}

async fn execute_get_divergent_session(
    args: &serde_json::Value,
    agent: &AgentEngine,
) -> Result<String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'session_id' argument"))?;

    match agent.get_divergent_session(session_id).await {
        Ok(payload) => {
            Ok(serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()))
        }
        Err(error) => Ok(format!("Failed to fetch divergent session: {error}")),
    }
}

async fn execute_run_debate(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: &str,
    task_id: Option<&str>,
) -> Result<String> {
    let topic = args
        .get("topic")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'topic' argument"))?
        .to_string();

    let custom_framings = args
        .get("custom_framings")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let label = item.get("label")?.as_str()?.trim().to_string();
                    let prompt = item
                        .get("system_prompt_override")?
                        .as_str()?
                        .trim()
                        .to_string();
                    if label.is_empty() || prompt.is_empty() {
                        return None;
                    }
                    Some(super::handoff::divergent::Framing {
                        label,
                        system_prompt_override: prompt,
                        task_id: None,
                        contribution_id: None,
                    })
                })
                .collect::<Vec<_>>()
        })
        .filter(|v| v.len() >= 2);

    let goal_run_id = task_id.and_then(|_tid| None::<&str>);

    match agent
        .start_debate_session(&topic, custom_framings, thread_id, goal_run_id)
        .await
    {
        Ok(session_id) => {
            let response = serde_json::json!({
                "status": "started",
                "session_id": session_id,
                "topic": topic,
                "message": "Debate session started. Use get_debate_session with this session_id to retrieve the debate state and verdict as it progresses."
            });
            Ok(serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string()))
        }
        Err(e) => Ok(format!("Debate session failed: {e}")),
    }
}

async fn execute_get_debate_session(
    args: &serde_json::Value,
    agent: &AgentEngine,
) -> Result<String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'session_id' argument"))?;

    match agent.get_debate_session_payload(session_id).await {
        Ok(payload) => {
            Ok(serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()))
        }
        Err(error) => Ok(format!("Failed to fetch debate session: {error}")),
    }
}

async fn execute_get_critique_session(
    args: &serde_json::Value,
    agent: &AgentEngine,
) -> Result<String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'session_id' argument"))?;

    match agent.get_critique_session_payload(session_id).await {
        Ok(payload) => {
            Ok(serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()))
        }
        Err(error) => Ok(format!("Failed to fetch critique session: {error}")),
    }
}

async fn execute_lookup_emergent_protocol(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: &str,
) -> Result<String> {
    let token = args
        .get("token")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'token' argument"))?;

    let record_usage = args
        .get("record_usage")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let payload = if record_usage {
        let success = args.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
        let fallback_reason = args
            .get("fallback_reason")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned);
        let execution_time_ms = args.get("execution_time_ms").and_then(|v| v.as_u64());
        agent
            .record_protocol_registry_usage(
                thread_id,
                token,
                success,
                fallback_reason,
                execution_time_ms,
            )
            .await?
            .map(serde_json::to_value)
            .transpose()?
    } else {
        agent
            .lookup_thread_protocol_registry_entry(thread_id, token)
            .await?
            .map(serde_json::to_value)
            .transpose()?
    };

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "thread_id": thread_id,
        "token": token,
        "entry": payload,
    }))
    .unwrap_or_else(|_| "{}".to_string()))
}

async fn execute_reload_emergent_protocol_registry(
    _args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: &str,
) -> Result<String> {
    let payload = agent.reload_thread_protocol_registry(thread_id).await?;
    Ok(serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()))
}

async fn execute_decode_emergent_protocol(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: &str,
) -> Result<String> {
    let token = args
        .get("token")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'token' argument"))?;
    let current_role = args.get("current_role").and_then(|v| v.as_str());
    let target_role = args.get("target_role").and_then(|v| v.as_str());
    let normalized_pattern = args.get("normalized_pattern").and_then(|v| v.as_str());

    let payload = agent
        .decode_thread_protocol_token(thread_id, token, current_role, target_role, normalized_pattern)
        .await?
        .map(serde_json::to_value)
        .transpose()?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "thread_id": thread_id,
        "token": token,
        "decode": payload,
    }))
    .unwrap_or_else(|_| "{}".to_string()))
}

async fn execute_get_emergent_protocol_usage_log(
    args: &serde_json::Value,
    agent: &AgentEngine,
) -> Result<String> {
    let protocol_id = args
        .get("protocol_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'protocol_id' argument"))?;

    let payload = agent.get_protocol_usage_log_payload(protocol_id).await?;
    Ok(serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()))
}

async fn execute_append_debate_argument(
    args: &serde_json::Value,
    agent: &AgentEngine,
) -> Result<String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'session_id' argument"))?;
    let role = match args
        .get("role")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| anyhow::anyhow!("missing 'role' argument"))?
    {
        "proponent" => crate::agent::debate::types::RoleKind::Proponent,
        "skeptic" => crate::agent::debate::types::RoleKind::Skeptic,
        "synthesizer" => crate::agent::debate::types::RoleKind::Synthesizer,
        other => anyhow::bail!("invalid 'role' argument: {other}"),
    };
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'agent_id' argument"))?
        .to_string();
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'content' argument"))?
        .to_string();
    let evidence_refs = args
        .get("evidence_refs")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let responds_to = args
        .get("responds_to")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);

    let session = agent
        .get_persisted_debate_session(session_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("unknown debate session: {session_id}"))?;
    let argument = crate::agent::debate::types::Argument {
        id: format!("arg_{}", uuid::Uuid::new_v4()),
        round: session.current_round,
        role,
        agent_id,
        content,
        evidence_refs,
        responds_to,
        timestamp_ms: crate::agent::debate::protocol::now_millis(),
    };

    agent.append_debate_argument(session_id, argument).await?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "appended",
        "session_id": session_id,
    }))
    .unwrap_or_else(|_| "{}".to_string()))
}

async fn execute_advance_debate_round(
    args: &serde_json::Value,
    agent: &AgentEngine,
) -> Result<String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'session_id' argument"))?;

    match agent.advance_debate_round(session_id).await {
        Ok(session) => Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "advanced",
            "session_id": session.id,
            "current_round": session.current_round,
            "roles": session.roles,
        }))
        .unwrap_or_else(|_| "{}".to_string())),
        Err(error) => Ok(format!("Failed to advance debate round: {error}")),
    }
}

async fn execute_complete_debate_session(
    args: &serde_json::Value,
    agent: &AgentEngine,
) -> Result<String> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'session_id' argument"))?;

    match agent.complete_debate_session(session_id).await {
        Ok(payload) => {
            Ok(serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()))
        }
        Err(error) => Ok(format!("Failed to complete debate session: {error}")),
    }
}

async fn execute_handoff_thread_agent(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: &str,
) -> Result<(String, Option<ToolPendingApproval>)> {
    if thread_id.trim().is_empty() {
        anyhow::bail!("handoff_thread_agent requires a thread context");
    }

    let action = args
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'action' argument"))?;
    let reason = args
        .get("reason")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'reason' argument"))?;
    let summary = args
        .get("summary")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'summary' argument"))?;
    let requested_by = match args
        .get("requested_by")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .unwrap_or("agent")
    {
        "user" => crate::agent::ThreadHandoffRequestedBy::User,
        "agent" => crate::agent::ThreadHandoffRequestedBy::Agent,
        other => anyhow::bail!("invalid 'requested_by' argument: {other}"),
    };

    let request = match action {
        "push_handoff" => crate::agent::PendingThreadHandoffActivation {
            thread_id: thread_id.to_string(),
            kind: crate::agent::ThreadHandoffKind::Push,
            target_agent_id: Some(
                args.get("target_agent_id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("push_handoff requires 'target_agent_id'"))?
                    .to_string(),
            ),
            requested_by,
            reason: reason.to_string(),
            summary: summary.to_string(),
        },
        "return_handoff" => crate::agent::PendingThreadHandoffActivation {
            thread_id: thread_id.to_string(),
            kind: crate::agent::ThreadHandoffKind::Return,
            target_agent_id: None,
            requested_by,
            reason: reason.to_string(),
            summary: summary.to_string(),
        },
        other => anyhow::bail!("unsupported handoff action: {other}"),
    };

    let requires_approval = matches!(
        (request.kind, request.requested_by),
        (
            crate::agent::ThreadHandoffKind::Push,
            crate::agent::ThreadHandoffRequestedBy::Agent
        )
    ) && !matches!(
        agent.config.read().await.managed_execution.security_level,
        SecurityLevel::Yolo
    );

    if requires_approval {
        let pending_approval = agent.thread_handoff_pending_approval(&request, "medium")?;
        agent
            .queue_thread_handoff_approval(&request, &pending_approval)
            .await?;
        let target_name = request
            .target_agent_id
            .as_deref()
            .map(canonical_agent_name)
            .unwrap_or(MAIN_AGENT_NAME);
        return Ok((
            format!("Queued operator approval to hand off this thread to {target_name}."),
            Some(pending_approval),
        ));
    }

    let event = agent
        .apply_thread_handoff_activation(&request, None)
        .await?;
    Ok((
        format!(
            "Thread handoff complete: {} -> {}.",
            canonical_agent_name(&event.from_agent_id),
            canonical_agent_name(&event.to_agent_id)
        ),
        None,
    ))
}

async fn execute_message_agent_visible_thread_continuation(
    agent: &AgentEngine,
    thread_id: &str,
    sender: &str,
    resolved_target_id: &str,
    resolved_target_name: &str,
    message: &str,
    preferred_session_hint: Option<String>,
) -> Result<serde_json::Value> {
    let payload = agent
        .build_internal_delegate_payload(Some(thread_id), message, true)
        .await;
    let continuation_prompt = agent
        .build_visible_thread_continuation_prompt(
            thread_id,
            sender,
            resolved_target_id,
            message,
        )
        .await;
    agent
        .enqueue_visible_thread_continuation(
            thread_id,
            crate::agent::DeferredVisibleThreadContinuation {
                agent_id: resolved_target_id.to_string(),
                preferred_session_hint: preferred_session_hint.clone(),
                llm_user_content: continuation_prompt,
                force_compaction: false,
                rerun_participant_observers_after_turn: true,
                internal_delegate_sender: Some(sender.to_string()),
                internal_delegate_message: Some(payload),
            },
        )
        .await;
    Ok(serde_json::json!({
        "target": resolved_target_name,
        "thread_id": crate::agent::agent_identity::internal_dm_thread_id(
            sender,
            resolved_target_id,
        ),
        "response": "Visible-thread continuation queued; internal discussion will run after the current turn finishes.",
        "upstream_message": serde_json::Value::Null,
        "visible_thread_continuation_requested": true,
    }))
}

async fn execute_message_agent_internal_dm(
    agent: &AgentEngine,
    sender: &str,
    resolved_target_id: &str,
    resolved_target_name: &str,
    message: &str,
    preferred_session_hint: Option<&str>,
) -> Result<serde_json::Value> {
    let result = Box::pin(agent.send_internal_agent_message(
        sender,
        resolved_target_id,
        message,
        preferred_session_hint,
    ))
    .await?;
    Ok(serde_json::json!({
        "target": resolved_target_name,
        "thread_id": result.thread_id,
        "response": result.response,
        "upstream_message": result.upstream_message,
        "visible_thread_continuation_requested": false,
    }))
}

async fn execute_message_agent(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: &str,
    task_id: Option<&str>,
    preferred_session_id: Option<SessionId>,
) -> Result<String> {
    let target = args
        .get("target")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'target' argument"))?;
    let message = args
        .get("message")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'message' argument"))?;
    let request_visible_thread_continuation = args
        .get("request_visible_thread_continuation")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let sender = if let Some(current_task_id) = task_id {
        let tasks = agent.tasks.lock().await;
        sender_name_for_task(tasks.iter().find(|task| task.id == current_task_id))
    } else {
        canonical_agent_name(&current_agent_scope_id()).to_string()
    };
    let (resolved_target_id, resolved_target_name) =
        agent.resolve_thread_participant_target(target).await?;

    if canonical_agent_id(&sender) == canonical_agent_id(&resolved_target_id) {
        anyhow::bail!("message_agent cannot target the current active responder");
    }
    if request_visible_thread_continuation
        && (thread_id.trim().is_empty()
            || crate::agent::agent_identity::is_internal_dm_thread(thread_id)
            || crate::agent::agent_identity::is_participant_playground_thread(thread_id)
            || crate::agent::is_internal_handoff_thread(thread_id))
    {
        anyhow::bail!(
            "request_visible_thread_continuation requires a visible operator thread, not an internal thread"
        );
    }

    let preferred_session_hint = preferred_session_id.as_ref().map(|value| value.to_string());
    let result = if request_visible_thread_continuation {
        Box::pin(execute_message_agent_visible_thread_continuation(
            agent,
            thread_id,
            &sender,
            &resolved_target_id,
            &resolved_target_name,
            message,
            preferred_session_hint.clone(),
        ))
        .await?
    } else {
        Box::pin(execute_message_agent_internal_dm(
            agent,
            &sender,
            &resolved_target_id,
            &resolved_target_name,
            message,
            preferred_session_hint.as_deref(),
        ))
        .await?
    };
    Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string()))
}
