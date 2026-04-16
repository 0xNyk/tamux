use serde::Serialize;

const MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER: usize = 64;

#[derive(Debug, Clone, Copy)]
enum MemoryReadScope {
    Memory,
    User,
    Soul,
}

impl MemoryReadScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::User => "user",
            Self::Soul => "soul",
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Self::Memory => "MEMORY.md",
            Self::User => "USER.md",
            Self::Soul => "SOUL.md",
        }
    }

    fn default_include_thread_structural_memory(self) -> bool {
        matches!(self, Self::Memory)
    }
}

#[derive(Debug, Clone)]
struct MemoryReadRequest {
    include_already_injected: bool,
    include_base_markdown: bool,
    include_operator_profile_json: bool,
    include_operator_model_summary: bool,
    include_thread_structural_memory: bool,
    limit_per_layer: usize,
}

#[derive(Debug, Clone)]
struct MemorySearchRequest {
    query: String,
    include_already_injected: bool,
    include_base_markdown: bool,
    include_operator_profile_json: bool,
    include_operator_model_summary: bool,
    include_thread_structural_memory: bool,
    limit: usize,
}

#[derive(Debug, Serialize)]
struct MemoryReadEnvelope {
    scope: String,
    injection_state: MemoryReadInjectionState,
    layers_consulted: Vec<String>,
    layers_skipped: Vec<MemoryReadSkippedLayer>,
    results: serde_json::Value,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct MemoryReadInjectionState {
    include_already_injected: bool,
    base_layer_injected: bool,
    base_layer_stale: bool,
    injected_base_markdown_hash: Option<String>,
    injected_base_markdown_updated_at_ms: Option<u64>,
    current_base_markdown_hash: Option<String>,
    current_base_markdown_updated_at_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct MemoryReadSkippedLayer {
    layer: String,
    reason: String,
}

#[derive(Debug, Serialize)]
struct BaseMarkdownResult {
    file: String,
    content: String,
    updated_at_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct MemorySearchEnvelope {
    scope: String,
    query: String,
    injection_state: MemoryReadInjectionState,
    layers_consulted: Vec<String>,
    layers_skipped: Vec<MemoryReadSkippedLayer>,
    matches: Vec<MemorySearchMatch>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct MemorySearchMatch {
    layer: String,
    source: String,
    snippet: String,
    score: u32,
    #[serde(skip_serializing)]
    rank_weight: Option<f64>,
    freshness: MemorySearchFreshness,
}

#[derive(Debug, Serialize)]
struct MemorySearchFreshness {
    status: String,
    updated_at_ms: Option<u64>,
    injected_updated_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct MemorySearchCandidate {
    layer: &'static str,
    source: String,
    snippet: String,
    haystack: String,
    rank_weight: Option<f64>,
    updated_at_ms: Option<u64>,
    injected_updated_at_ms: Option<u64>,
    freshness_status: &'static str,
}

fn parse_bool_arg(args: &serde_json::Value, key: &str, default: bool) -> Result<bool> {
    match args.get(key) {
        Some(value) => value
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("'{key}' must be a boolean")),
        None => Ok(default),
    }
}

fn ensure_object_args(args: &serde_json::Value) -> Result<()> {
    if args.is_object() {
        Ok(())
    } else {
        anyhow::bail!("memory tool arguments must be a JSON object")
    }
}

fn parse_clamped_usize_arg(
    args: &serde_json::Value,
    key: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize> {
    let value = match args.get(key) {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("'{key}' must be a non-negative integer"))?
            as usize,
        None => default,
    };
    Ok(value.clamp(min, max))
}

fn parse_memory_read_request(
    args: &serde_json::Value,
    scope: MemoryReadScope,
) -> Result<MemoryReadRequest> {
    ensure_object_args(args)?;
    let limit_per_layer = parse_clamped_usize_arg(args, "limit_per_layer", 5, 1, 25)?;

    Ok(MemoryReadRequest {
        include_already_injected: parse_bool_arg(args, "include_already_injected", false)?,
        include_base_markdown: parse_bool_arg(args, "include_base_markdown", true)?,
        include_operator_profile_json: parse_bool_arg(args, "include_operator_profile_json", true)?,
        include_operator_model_summary: parse_bool_arg(args, "include_operator_model_summary", true)?,
        include_thread_structural_memory: parse_bool_arg(
            args,
            "include_thread_structural_memory",
            scope.default_include_thread_structural_memory(),
        )?,
        limit_per_layer,
    })
}

fn parse_memory_search_request(
    args: &serde_json::Value,
    scope: MemoryReadScope,
) -> Result<MemorySearchRequest> {
    ensure_object_args(args)?;
    let query = args
        .get("query")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing 'query' argument"))?
        .to_string();

    Ok(MemorySearchRequest {
        query,
        include_already_injected: parse_bool_arg(args, "include_already_injected", false)?,
        include_base_markdown: parse_bool_arg(args, "include_base_markdown", true)?,
        include_operator_profile_json: parse_bool_arg(args, "include_operator_profile_json", true)?,
        include_operator_model_summary: parse_bool_arg(args, "include_operator_model_summary", true)?,
        include_thread_structural_memory: parse_bool_arg(
            args,
            "include_thread_structural_memory",
            scope.default_include_thread_structural_memory(),
        )?,
        limit: parse_clamped_usize_arg(args, "limit", 5, 1, 25)?,
    })
}

fn requested_thread_id(args: &serde_json::Value) -> Result<Option<String>> {
    match args.get("thread_id") {
        Some(value) => match value.as_str() {
            Some(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed.to_string()))
                }
            }
            None => Err(anyhow::anyhow!("'thread_id' must be a string")),
        },
        None => Ok(None),
    }
}

fn requested_task_id(args: &serde_json::Value) -> Result<Option<String>> {
    match args.get("task_id") {
        Some(value) => match value.as_str() {
            Some(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(trimmed.to_string()))
                }
            }
            None => Err(anyhow::anyhow!("'task_id' must be a string")),
        },
        None => Ok(None),
    }
}

fn file_updated_at_ms(path: &std::path::Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let duration = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(duration.as_millis() as u64)
}

fn base_layer_is_stale(
    injected_hash: Option<&str>,
    injected_updated_at_ms: Option<u64>,
    current_hash: Option<&str>,
    current_updated_at_ms: Option<u64>,
) -> bool {
    injected_hash != current_hash || injected_updated_at_ms != current_updated_at_ms
}

fn current_scope_base_layer_state(
    scope: MemoryReadScope,
    summary: &crate::agent::memory_context::StructuredMemorySummary,
) -> (Option<String>, Option<u64>) {
    match scope {
        MemoryReadScope::Memory => (
            summary.memory_markdown_hash.clone(),
            summary.memory_markdown_updated_at_ms,
        ),
        MemoryReadScope::User => (
            summary.user_markdown_hash.clone(),
            summary.user_markdown_updated_at_ms,
        ),
        MemoryReadScope::Soul => (
            summary.soul_markdown_hash.clone(),
            summary.soul_markdown_updated_at_ms,
        ),
    }
}

fn injected_scope_base_layer_state(
    scope: MemoryReadScope,
    injection_state: Option<&crate::agent::memory_context::PromptMemoryInjectionState>,
) -> (Option<String>, Option<u64>) {
    match (scope, injection_state) {
        (MemoryReadScope::Memory, Some(state)) => (
            state.memory_markdown_hash.clone(),
            state.memory_markdown_updated_at_ms,
        ),
        (MemoryReadScope::User, Some(state)) => {
            (state.user_markdown_hash.clone(), state.user_markdown_updated_at_ms)
        }
        (MemoryReadScope::Soul, Some(state)) => {
            (state.soul_markdown_hash.clone(), state.soul_markdown_updated_at_ms)
        }
        (_, None) => (None, None),
    }
}

fn scope_markdown_content_and_path<'a>(
    scope: MemoryReadScope,
    memory: &'a crate::agent::types::AgentMemory,
    memory_paths: &'a crate::agent::task_prompt::MemoryPaths,
) -> (&'a str, &'a std::path::Path) {
    match scope {
        MemoryReadScope::Memory => (&memory.memory, &memory_paths.memory_path),
        MemoryReadScope::User => (&memory.user_profile, &memory_paths.user_path),
        MemoryReadScope::Soul => (&memory.soul, &memory_paths.soul_path),
    }
}

fn build_memory_injection_state(
    include_already_injected: bool,
    base_layer_injected: bool,
    base_layer_stale: bool,
    injected_base_markdown_hash: Option<String>,
    injected_base_markdown_updated_at_ms: Option<u64>,
    current_base_markdown_hash: Option<String>,
    current_base_markdown_updated_at_ms: Option<u64>,
) -> MemoryReadInjectionState {
    MemoryReadInjectionState {
        include_already_injected,
        base_layer_injected,
        base_layer_stale,
        injected_base_markdown_hash,
        injected_base_markdown_updated_at_ms,
        current_base_markdown_hash,
        current_base_markdown_updated_at_ms,
    }
}

fn search_freshness_status(
    current_hash: Option<&str>,
    current_updated_at_ms: Option<u64>,
    injected_hash: Option<&str>,
    injected_updated_at_ms: Option<u64>,
) -> &'static str {
    if current_hash.is_none() && current_updated_at_ms.is_none() {
        return "unknown";
    }
    if injected_hash.is_some() || injected_updated_at_ms.is_some() {
        if base_layer_is_stale(
            injected_hash,
            injected_updated_at_ms,
            current_hash,
            current_updated_at_ms,
        ) {
            return "stale";
        }
    }
    "fresh"
}

fn score_memory_search_candidate(query_lower: &str, tokens: &[String], haystack: &str) -> Option<u32> {
    let normalized = haystack.to_ascii_lowercase();
    let mut score = 0u32;

    if normalized == query_lower {
        score += 160;
    } else if normalized.starts_with(query_lower) {
        score += 120;
    } else if normalized.contains(query_lower) {
        score += 90;
    }

    let matched_tokens = tokens
        .iter()
        .filter(|token| normalized.contains(token.as_str()))
        .count() as u32;
    if matched_tokens == 0 && score == 0 {
        return None;
    }

    score += matched_tokens * 15;
    if !tokens.is_empty() && matched_tokens as usize == tokens.len() {
        score += 20;
    }
    let word_count = normalized.split_whitespace().count() as u32;
    score += 12u32.saturating_sub(word_count.min(12));
    Some(score)
}

fn collect_base_markdown_candidates(
    candidates: &mut Vec<MemorySearchCandidate>,
    scope: MemoryReadScope,
    content: &str,
    updated_at_ms: Option<u64>,
    current_hash: Option<&str>,
    injected_hash: Option<&str>,
    injected_updated_at_ms: Option<u64>,
) -> bool {
    let mut truncated = false;
    for (index, line) in content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then_some((index, trimmed))
        })
    {
        if candidates.len() >= MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER {
            truncated = true;
            break;
        }
        candidates.push(MemorySearchCandidate {
            layer: "base_markdown",
            source: format!("{}:{}", scope.file_name(), index + 1),
            snippet: line.to_string(),
            haystack: line.to_string(),
            rank_weight: None,
            updated_at_ms,
            injected_updated_at_ms,
            freshness_status: search_freshness_status(
                current_hash,
                updated_at_ms,
                injected_hash,
                injected_updated_at_ms,
            ),
        });
    }
    truncated
}

fn collect_operator_model_candidates(
    candidates: &mut Vec<MemorySearchCandidate>,
    content: &str,
    updated_at_ms: Option<u64>,
) -> bool {
    let mut truncated = false;
    for (index, line) in content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then_some((index, trimmed))
        })
    {
        if candidates.len() >= MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER {
            truncated = true;
            break;
        }
        candidates.push(MemorySearchCandidate {
            layer: "operator_model_summary",
            source: format!("operator_model_summary:{}", index + 1),
            snippet: line.to_string(),
            haystack: line.to_string(),
            rank_weight: None,
            updated_at_ms,
            injected_updated_at_ms: None,
            freshness_status: search_freshness_status(None, updated_at_ms, None, None),
        });
    }
    truncated
}

fn collect_operator_profile_candidates(
    value: &serde_json::Value,
    path: &str,
    candidates: &mut Vec<MemorySearchCandidate>,
) -> bool {
    if candidates.len() >= MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER {
        return true;
    }

    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let next_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                if collect_operator_profile_candidates(value, &next_path, candidates) {
                    return true;
                }
            }
            false
        }
        serde_json::Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                let next_path = format!("{path}[{index}]");
                if collect_operator_profile_candidates(value, &next_path, candidates) {
                    return true;
                }
            }
            false
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return false;
            }
            if candidates.len() >= MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER {
                return true;
            }
            candidates.push(MemorySearchCandidate {
                layer: "operator_profile_json",
                source: path.to_string(),
                snippet: format!("{path}: {trimmed}"),
                haystack: format!("{path} {trimmed}"),
                rank_weight: None,
                updated_at_ms: None,
                injected_updated_at_ms: None,
                freshness_status: "unknown",
            });
            false
        }
        serde_json::Value::Number(number) => {
            if candidates.len() >= MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER {
                return true;
            }
            candidates.push(MemorySearchCandidate {
                layer: "operator_profile_json",
                source: path.to_string(),
                snippet: format!("{path}: {number}"),
                haystack: format!("{path} {number}"),
                rank_weight: None,
                updated_at_ms: None,
                injected_updated_at_ms: None,
                freshness_status: "unknown",
            });
            false
        }
        serde_json::Value::Bool(boolean) => {
            if candidates.len() >= MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER {
                return true;
            }
            candidates.push(MemorySearchCandidate {
                layer: "operator_profile_json",
                source: path.to_string(),
                snippet: format!("{path}: {boolean}"),
                haystack: format!("{path} {boolean}"),
                rank_weight: None,
                updated_at_ms: None,
                injected_updated_at_ms: None,
                freshness_status: "unknown",
            });
            false
        }
        serde_json::Value::Null => false,
    }
}

fn collect_thread_structural_candidates(
    candidates: &mut Vec<MemorySearchCandidate>,
    structural_memory: crate::agent::context::structural_memory::ThreadStructuralMemory,
) -> bool {
    let language_hints_truncated =
        structural_memory.language_hints.len() > MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER;
    for hint in structural_memory
        .language_hints
        .iter()
        .take(MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER)
    {
        candidates.push(MemorySearchCandidate {
            layer: "thread_structural_memory",
            source: format!("language_hint:{hint}"),
            snippet: format!("Language hint: {hint}"),
            haystack: hint.clone(),
            rank_weight: None,
            updated_at_ms: None,
            injected_updated_at_ms: None,
            freshness_status: "unknown",
        });
    }

    let entries = structural_memory.concise_context_entries(&[], usize::MAX);
    let entries_truncated = entries.len() > MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER;
    for entry in entries.into_iter().take(MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER) {
        candidates.push(MemorySearchCandidate {
            layer: "thread_structural_memory",
            source: entry.node_id,
            snippet: entry.summary.clone(),
            haystack: entry.summary,
            rank_weight: None,
            updated_at_ms: None,
            injected_updated_at_ms: None,
            freshness_status: "unknown",
        });
    }
    language_hints_truncated || entries_truncated
}

fn preferred_thread_structural_refs_for_query(
    structural_memory: &crate::agent::context::structural_memory::ThreadStructuralMemory,
    query: &str,
    limit: usize,
) -> Vec<String> {
    let query_lower = query.to_ascii_lowercase();
    let tokens = query_tokens(query);
    structural_memory
        .concise_context_entries(&[], usize::MAX)
        .into_iter()
        .filter(|entry| {
            let haystack = format!("{} {}", entry.node_id, entry.summary).to_ascii_lowercase();
            haystack.contains(&query_lower)
                || tokens.iter().all(|token| haystack.contains(token))
        })
        .map(|entry| entry.node_id)
        .take(limit)
        .collect()
}

fn collect_thread_structural_graph_lookup_candidates(
    candidates: &mut Vec<MemorySearchCandidate>,
    structural_memory: &crate::agent::context::structural_memory::ThreadStructuralMemory,
    preferred_refs: &[String],
    limit: usize,
) {
    let mut seen = std::collections::HashSet::new();
    for preferred_ref in preferred_refs.iter().take(limit) {
        for neighbor in structural_memory.graph_neighbors(preferred_ref, limit) {
            if !seen.insert((neighbor.node_id.clone(), neighbor.relation_kind.clone())) {
                continue;
            }
            candidates.push(MemorySearchCandidate {
                layer: "thread_structural_memory",
                source: neighbor.node_id.clone(),
                snippet: format!("Graph lookup from {preferred_ref}: {}", neighbor.summary),
                haystack: format!(
                    "{preferred_ref} {} {} {}",
                    neighbor.node_id, neighbor.relation_kind, neighbor.summary
                ),
                rank_weight: None,
                updated_at_ms: None,
                injected_updated_at_ms: None,
                freshness_status: "unknown",
            });
            if candidates.len() >= MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER {
                return;
            }
        }
    }
}

async fn load_thread_memory_graph_neighbors(
    agent: &AgentEngine,
    structural_memory: &crate::agent::context::structural_memory::ThreadStructuralMemory,
    limit: usize,
) -> Result<Vec<crate::history::MemoryGraphNeighborRow>> {
    let structural_refs = structural_memory
        .concise_context_entries(&[], limit)
        .into_iter()
        .map(|entry| entry.node_id)
        .collect::<Vec<_>>();
    let mut neighbors = Vec::new();
    let mut frontier = structural_refs.clone();

    for _depth in 0..3 {
        let current_frontier = frontier;
        frontier = Vec::new();
        for node_id in current_frontier.into_iter().take(limit) {
            let remaining = limit.saturating_sub(neighbors.len());
            if remaining == 0 {
                break;
            }
            let rows = agent
                .history
                .list_memory_graph_neighbors(&node_id, remaining)
                .await?;
            for row in rows {
                if structural_refs.iter().any(|existing| existing == &row.node.id) {
                    continue;
                }
                if let Some(existing) = neighbors
                    .iter_mut()
                    .find(|existing: &&mut crate::history::MemoryGraphNeighborRow| {
                        existing.node.id == row.node.id
                    })
                {
                    if row.via_edge.weight > existing.via_edge.weight {
                        *existing = row;
                    }
                    continue;
                }
                if !frontier.iter().any(|existing| existing == &row.node.id) {
                    frontier.push(row.node.id.clone());
                }
                neighbors.push(row);
                if neighbors.len() >= limit {
                    break;
                }
            }
        }
        if neighbors.len() >= limit || frontier.is_empty() {
            break;
        }
    }
    neighbors.sort_by(|left, right| {
        right
            .via_edge
            .weight
            .partial_cmp(&left.via_edge.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(right.via_edge.last_updated_ms.cmp(&left.via_edge.last_updated_ms))
            .then(left.node.id.cmp(&right.node.id))
    });
    Ok(neighbors)
}

fn memory_graph_neighbor_snippet(row: &crate::history::MemoryGraphNeighborRow) -> String {
    let relation = row.via_edge.relation_type.replace('_', " ");
    let summary = row
        .node
        .summary_text
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!(" — {}", value))
        .unwrap_or_default();
    format!(
        "Graph neighbor: {} ({}) via {}{}",
        row.node.label, row.node.node_type, relation, summary
    )
}

fn rank_memory_search_matches(
    request: &MemorySearchRequest,
    candidates: Vec<MemorySearchCandidate>,
) -> (Vec<MemorySearchMatch>, bool) {
    let query_lower = request.query.to_ascii_lowercase();
    let tokens = query_tokens(&request.query);
    let mut matches = candidates
        .into_iter()
        .filter_map(|candidate| {
            score_memory_search_candidate(&query_lower, &tokens, &candidate.haystack).map(|score| {
                MemorySearchMatch {
                    layer: candidate.layer.to_string(),
                    source: candidate.source,
                    snippet: candidate.snippet,
                    score,
                    rank_weight: candidate.rank_weight,
                    freshness: MemorySearchFreshness {
                        status: candidate.freshness_status.to_string(),
                        updated_at_ms: candidate.updated_at_ms,
                        injected_updated_at_ms: candidate.injected_updated_at_ms,
                    },
                }
            })
        })
        .collect::<Vec<_>>();

    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| {
                right
                    .rank_weight
                    .unwrap_or(0.0)
                    .partial_cmp(&left.rank_weight.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then(left.layer.cmp(&right.layer))
            .then(left.source.cmp(&right.source))
            .then(left.snippet.cmp(&right.snippet))
    });

    let truncated = matches.len() > request.limit;
    matches.truncate(request.limit);
    (matches, truncated)
}

async fn task_or_current_scope_id(
    agent: &AgentEngine,
    thread_id: Option<&str>,
    task_id: Option<&str>,
) -> Result<String> {
    if let Some(current_task_id) = task_id {
        let tasks = agent.tasks.lock().await;
        let task = tasks
            .iter()
            .find(|task| task.id == current_task_id)
            .ok_or_else(|| anyhow::anyhow!("unknown task_id: {current_task_id}"))?;
        return Ok(crate::agent::agent_scope_id_for_task(Some(task)));
    }
    if thread_id.is_some() {
        if thread_id == Some(crate::agent::concierge::CONCIERGE_THREAD_ID) {
            return Ok(crate::agent::agent_identity::CONCIERGE_AGENT_ID.to_string());
        }
        let thread_id = thread_id.expect("checked is_some");
        if let Some(active_agent_id) = agent.active_agent_id_for_thread(thread_id).await {
            return Ok(active_agent_id);
        }
        if agent.get_thread_memory_injection_state(thread_id).await.is_some() {
            return Ok(crate::agent::agent_identity::MAIN_AGENT_ID.to_string());
        }
        if agent.get_thread(thread_id).await.is_some() {
            return Ok(crate::agent::agent_identity::MAIN_AGENT_ID.to_string());
        }
        anyhow::bail!("unknown thread_id: {thread_id}");
    }
    Ok(current_agent_scope_id())
}

async fn execute_memory_read_tool(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: Option<&str>,
    task_id: Option<&str>,
    agent_data_dir: &std::path::Path,
    scope: MemoryReadScope,
) -> Result<String> {
    let request = parse_memory_read_request(args, scope)?;
    let effective_thread_id = if task_id.is_some() { None } else { thread_id };
    let scope_id = task_or_current_scope_id(agent, thread_id, task_id).await?;
    let memory = crate::agent::memory::load_memory_for_scope(agent_data_dir, &scope_id).await?;
    let memory_paths = crate::agent::task_prompt::memory_paths_for_scope(agent_data_dir, &scope_id);
    let summary =
        crate::agent::memory_context::build_structured_memory_summary(&memory, &memory_paths, None, None);
    let injection_state = match effective_thread_id {
        Some(thread_id) => agent.get_thread_memory_injection_state(thread_id).await,
        None => None,
    };
    let (current_base_markdown_hash, current_base_markdown_updated_at_ms) =
        current_scope_base_layer_state(scope, &summary);
    let (injected_base_markdown_hash, injected_base_markdown_updated_at_ms) =
        injected_scope_base_layer_state(scope, injection_state.as_ref());
    let base_layer_stale = injection_state.as_ref().is_some_and(|state| {
        state.is_base_layer_injected()
            && base_layer_is_stale(
                injected_base_markdown_hash.as_deref(),
                injected_base_markdown_updated_at_ms,
                current_base_markdown_hash.as_deref(),
                current_base_markdown_updated_at_ms,
            )
    });
    let base_layer_injected = injection_state
        .as_ref()
        .is_some_and(|state| state.is_base_layer_injected());

    let mut layers_consulted = Vec::new();
    let mut layers_skipped = Vec::new();
    let mut truncated = false;
    let mut results = serde_json::Map::new();

    if request.include_base_markdown {
        let should_skip_base_markdown = base_layer_injected
            && !base_layer_stale
            && !request.include_already_injected;
        if should_skip_base_markdown {
            layers_skipped.push(MemoryReadSkippedLayer {
                layer: "base_markdown".to_string(),
                reason: "already_injected_fresh".to_string(),
            });
        } else {
            let (content, path) = scope_markdown_content_and_path(scope, &memory, &memory_paths);
            results.insert(
                "base_markdown".to_string(),
                serde_json::to_value(BaseMarkdownResult {
                    file: scope.file_name().to_string(),
                    content: content.to_string(),
                    updated_at_ms: file_updated_at_ms(path),
                })?,
            );
            layers_consulted.push("base_markdown".to_string());
        }
    } else {
        layers_skipped.push(MemoryReadSkippedLayer {
            layer: "base_markdown".to_string(),
            reason: "disabled".to_string(),
        });
    }

    if request.include_operator_profile_json {
        let profile_json = agent.get_operator_profile_summary_json().await?;
        let profile_value: serde_json::Value = serde_json::from_str(&profile_json)?;
        results.insert("operator_profile_json".to_string(), profile_value);
        layers_consulted.push("operator_profile_json".to_string());
    } else {
        layers_skipped.push(MemoryReadSkippedLayer {
            layer: "operator_profile_json".to_string(),
            reason: "disabled".to_string(),
        });
    }

    if request.include_operator_model_summary {
        if let Some(summary) = agent.build_operator_model_prompt_summary().await {
            results.insert(
                "operator_model_summary".to_string(),
                serde_json::json!({ "content": summary }),
            );
            layers_consulted.push("operator_model_summary".to_string());
        } else {
            layers_skipped.push(MemoryReadSkippedLayer {
                layer: "operator_model_summary".to_string(),
                reason: "empty".to_string(),
            });
        }
    } else {
        layers_skipped.push(MemoryReadSkippedLayer {
            layer: "operator_model_summary".to_string(),
            reason: "disabled".to_string(),
        });
    }

    if request.include_thread_structural_memory {
        if let Some(structural_memory) = match effective_thread_id {
            Some(thread_id) => agent.get_thread_structural_memory(thread_id).await,
            None => None,
        } {
            let entries = structural_memory.concise_context_entries(&[], request.limit_per_layer);
            let total_structural_entries = structural_memory.concise_context_entries(&[], usize::MAX).len();
            let total_language_hints = structural_memory.language_hints.len();
            let language_hints = structural_memory
                .language_hints
                .iter()
                .take(request.limit_per_layer)
                .cloned()
                .collect::<Vec<_>>();
            let graph_lookup = structural_memory.graph_lookup(&[], request.limit_per_layer);
            let graph_neighbors = load_thread_memory_graph_neighbors(
                agent,
                &structural_memory,
                request.limit_per_layer,
            )
            .await?;
            truncated |= entries.len() < total_structural_entries
                || language_hints.len() < total_language_hints;
            results.insert(
                "thread_structural_memory".to_string(),
                serde_json::json!({
                    "language_hints": language_hints,
                    "entries": entries
                        .into_iter()
                        .map(|entry| serde_json::json!({
                            "node_id": entry.node_id,
                            "summary": entry.summary,
                        }))
                        .collect::<Vec<_>>(),
                    "graph_lookup": graph_lookup
                        .into_iter()
                        .map(|neighbor| serde_json::json!({
                            "node_id": neighbor.node_id,
                            "relation_kind": neighbor.relation_kind,
                            "direction": neighbor.direction,
                            "summary": neighbor.summary,
                        }))
                        .collect::<Vec<_>>(),
                    "graph_neighbors": graph_neighbors
                        .into_iter()
                        .map(|row| serde_json::json!({
                            "node_id": row.node.id,
                            "label": row.node.label,
                            "node_type": row.node.node_type,
                            "relation_type": row.via_edge.relation_type,
                            "summary": memory_graph_neighbor_snippet(&row),
                        }))
                        .collect::<Vec<_>>(),
                }),
            );
            layers_consulted.push("thread_structural_memory".to_string());
        } else {
            layers_skipped.push(MemoryReadSkippedLayer {
                layer: "thread_structural_memory".to_string(),
                reason: "empty".to_string(),
            });
        }
    } else {
        layers_skipped.push(MemoryReadSkippedLayer {
            layer: "thread_structural_memory".to_string(),
            reason: "disabled".to_string(),
        });
    }

    let envelope = MemoryReadEnvelope {
        scope: scope.as_str().to_string(),
        injection_state: build_memory_injection_state(
            request.include_already_injected,
            base_layer_injected,
            base_layer_stale,
            injected_base_markdown_hash,
            injected_base_markdown_updated_at_ms,
            current_base_markdown_hash,
            current_base_markdown_updated_at_ms,
        ),
        layers_consulted,
        layers_skipped,
        results: serde_json::Value::Object(results),
        truncated,
    };

    Ok(serde_json::to_string(&envelope)?)
}

async fn execute_read_memory(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: Option<&str>,
    task_id: Option<&str>,
    agent_data_dir: &std::path::Path,
) -> Result<String> {
    execute_memory_read_tool(
        args,
        agent,
        thread_id,
        task_id,
        agent_data_dir,
        MemoryReadScope::Memory,
    )
    .await
}

async fn execute_read_user(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: Option<&str>,
    task_id: Option<&str>,
    agent_data_dir: &std::path::Path,
) -> Result<String> {
    execute_memory_read_tool(
        args,
        agent,
        thread_id,
        task_id,
        agent_data_dir,
        MemoryReadScope::User,
    )
    .await
}

async fn execute_read_soul(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: Option<&str>,
    task_id: Option<&str>,
    agent_data_dir: &std::path::Path,
) -> Result<String> {
    execute_memory_read_tool(
        args,
        agent,
        thread_id,
        task_id,
        agent_data_dir,
        MemoryReadScope::Soul,
    )
    .await
}

async fn execute_memory_search_tool(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: Option<&str>,
    task_id: Option<&str>,
    agent_data_dir: &std::path::Path,
    scope: MemoryReadScope,
) -> Result<String> {
    let request = parse_memory_search_request(args, scope)?;
    let effective_thread_id = if task_id.is_some() { None } else { thread_id };
    let scope_id = task_or_current_scope_id(agent, thread_id, task_id).await?;
    let memory = crate::agent::memory::load_memory_for_scope(agent_data_dir, &scope_id).await?;
    let memory_paths = crate::agent::task_prompt::memory_paths_for_scope(agent_data_dir, &scope_id);
    let summary =
        crate::agent::memory_context::build_structured_memory_summary(&memory, &memory_paths, None, None);
    let injection_state = match effective_thread_id {
        Some(thread_id) => agent.get_thread_memory_injection_state(thread_id).await,
        None => None,
    };
    let (current_base_markdown_hash, current_base_markdown_updated_at_ms) =
        current_scope_base_layer_state(scope, &summary);
    let (injected_base_markdown_hash, injected_base_markdown_updated_at_ms) =
        injected_scope_base_layer_state(scope, injection_state.as_ref());
    let base_layer_stale = injection_state.as_ref().is_some_and(|state| {
        state.is_base_layer_injected()
            && base_layer_is_stale(
                injected_base_markdown_hash.as_deref(),
                injected_base_markdown_updated_at_ms,
                current_base_markdown_hash.as_deref(),
                current_base_markdown_updated_at_ms,
            )
    });
    let base_layer_injected = injection_state
        .as_ref()
        .is_some_and(|state| state.is_base_layer_injected());

    let mut layers_consulted = Vec::new();
    let mut layers_skipped = Vec::new();
    let mut candidates = Vec::new();
    let mut collection_truncated = false;

    if request.include_base_markdown {
        let should_skip_base_markdown = base_layer_injected
            && !base_layer_stale
            && !request.include_already_injected;
        if should_skip_base_markdown {
            layers_skipped.push(MemoryReadSkippedLayer {
                layer: "base_markdown".to_string(),
                reason: "already_injected_fresh".to_string(),
            });
        } else {
            let (content, path) = scope_markdown_content_and_path(scope, &memory, &memory_paths);
            let mut base_markdown_candidates = Vec::new();
            collection_truncated |= collect_base_markdown_candidates(
                &mut base_markdown_candidates,
                scope,
                content,
                file_updated_at_ms(path),
                current_base_markdown_hash.as_deref(),
                injected_base_markdown_hash.as_deref(),
                injected_base_markdown_updated_at_ms,
            );
            candidates.extend(base_markdown_candidates);
            layers_consulted.push("base_markdown".to_string());
        }
    } else {
        layers_skipped.push(MemoryReadSkippedLayer {
            layer: "base_markdown".to_string(),
            reason: "disabled".to_string(),
        });
    }

    if request.include_operator_profile_json {
        let profile_json = agent.get_operator_profile_summary_json().await?;
        let profile_value: serde_json::Value = serde_json::from_str(&profile_json)?;
        let mut profile_candidates = Vec::new();
        collection_truncated |=
            collect_operator_profile_candidates(&profile_value, "", &mut profile_candidates);
        candidates.extend(profile_candidates);
        layers_consulted.push("operator_profile_json".to_string());
    } else {
        layers_skipped.push(MemoryReadSkippedLayer {
            layer: "operator_profile_json".to_string(),
            reason: "disabled".to_string(),
        });
    }

    if request.include_operator_model_summary {
        if let Some(summary) = agent.build_operator_model_prompt_summary().await {
            let mut operator_model_candidates = Vec::new();
            collection_truncated |= collect_operator_model_candidates(
                &mut operator_model_candidates,
                &summary,
                file_updated_at_ms(&crate::agent::operator_model_path(agent_data_dir)),
            );
            candidates.extend(operator_model_candidates);
            layers_consulted.push("operator_model_summary".to_string());
        } else {
            layers_skipped.push(MemoryReadSkippedLayer {
                layer: "operator_model_summary".to_string(),
                reason: "empty".to_string(),
            });
        }
    } else {
        layers_skipped.push(MemoryReadSkippedLayer {
            layer: "operator_model_summary".to_string(),
            reason: "disabled".to_string(),
        });
    }

    if request.include_thread_structural_memory {
        if let Some(structural_memory) = match effective_thread_id {
            Some(thread_id) => agent.get_thread_structural_memory(thread_id).await,
            None => None,
        } {
            let mut structural_candidates = Vec::new();
            collection_truncated |=
                collect_thread_structural_candidates(&mut structural_candidates, structural_memory.clone());
            let preferred_refs = preferred_thread_structural_refs_for_query(
                &structural_memory,
                &request.query,
                MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER,
            );
            collect_thread_structural_graph_lookup_candidates(
                &mut structural_candidates,
                &structural_memory,
                &preferred_refs,
                MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER,
            );
            let graph_neighbors = load_thread_memory_graph_neighbors(
                agent,
                &structural_memory,
                MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER,
            )
            .await?;
            for row in graph_neighbors.into_iter().take(MEMORY_SEARCH_MAX_CANDIDATES_PER_LAYER) {
                structural_candidates.push(MemorySearchCandidate {
                    layer: "thread_structural_memory",
                    source: row.node.id.clone(),
                    snippet: memory_graph_neighbor_snippet(&row),
                    haystack: format!(
                        "{} {} {} {}",
                        row.node.label,
                        row.node.node_type,
                        row.via_edge.relation_type,
                        row.node.summary_text.unwrap_or_default()
                    ),
                    rank_weight: Some(row.via_edge.weight),
                    updated_at_ms: Some(row.node.last_accessed_ms),
                    injected_updated_at_ms: None,
                    freshness_status: "current",
                });
            }
            candidates.extend(structural_candidates);
            layers_consulted.push("thread_structural_memory".to_string());
        } else {
            layers_skipped.push(MemoryReadSkippedLayer {
                layer: "thread_structural_memory".to_string(),
                reason: "empty".to_string(),
            });
        }
    } else {
        layers_skipped.push(MemoryReadSkippedLayer {
            layer: "thread_structural_memory".to_string(),
            reason: "disabled".to_string(),
        });
    }

    let (matches, ranking_truncated) = rank_memory_search_matches(&request, candidates);
    let envelope = MemorySearchEnvelope {
        scope: scope.as_str().to_string(),
        query: request.query,
        injection_state: build_memory_injection_state(
            request.include_already_injected,
            base_layer_injected,
            base_layer_stale,
            injected_base_markdown_hash,
            injected_base_markdown_updated_at_ms,
            current_base_markdown_hash,
            current_base_markdown_updated_at_ms,
        ),
        layers_consulted,
        layers_skipped,
        matches,
        truncated: collection_truncated || ranking_truncated,
    };

    Ok(serde_json::to_string(&envelope)?)
}

async fn execute_search_memory(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: Option<&str>,
    task_id: Option<&str>,
    agent_data_dir: &std::path::Path,
) -> Result<String> {
    execute_memory_search_tool(
        args,
        agent,
        thread_id,
        task_id,
        agent_data_dir,
        MemoryReadScope::Memory,
    )
    .await
}

async fn execute_search_user(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: Option<&str>,
    task_id: Option<&str>,
    agent_data_dir: &std::path::Path,
) -> Result<String> {
    execute_memory_search_tool(
        args,
        agent,
        thread_id,
        task_id,
        agent_data_dir,
        MemoryReadScope::User,
    )
    .await
}

async fn execute_search_soul(
    args: &serde_json::Value,
    agent: &AgentEngine,
    thread_id: Option<&str>,
    task_id: Option<&str>,
    agent_data_dir: &std::path::Path,
) -> Result<String> {
    execute_memory_search_tool(
        args,
        agent,
        thread_id,
        task_id,
        agent_data_dir,
        MemoryReadScope::Soul,
    )
    .await
}

pub(crate) async fn execute_memory_tool_for_mcp(
    tool_name: &str,
    args: &serde_json::Value,
    agent: &AgentEngine,
    agent_data_dir: &std::path::Path,
) -> Result<String> {
    if !args.is_object() {
        anyhow::bail!("memory tool arguments must be a JSON object");
    }
    let thread_id = requested_thread_id(args)?;
    let task_id = requested_task_id(args)?;

    match tool_name {
        "read_memory" => {
            execute_read_memory(
                args,
                agent,
                thread_id.as_deref(),
                task_id.as_deref(),
                agent_data_dir,
            )
            .await
        }
        "read_user" => {
            execute_read_user(
                args,
                agent,
                thread_id.as_deref(),
                task_id.as_deref(),
                agent_data_dir,
            )
            .await
        }
        "read_soul" => {
            execute_read_soul(
                args,
                agent,
                thread_id.as_deref(),
                task_id.as_deref(),
                agent_data_dir,
            )
            .await
        }
        "search_memory" => {
            execute_search_memory(
                args,
                agent,
                thread_id.as_deref(),
                task_id.as_deref(),
                agent_data_dir,
            )
            .await
        }
        "search_user" => {
            execute_search_user(
                args,
                agent,
                thread_id.as_deref(),
                task_id.as_deref(),
                agent_data_dir,
            )
            .await
        }
        "search_soul" => {
            execute_search_soul(
                args,
                agent,
                thread_id.as_deref(),
                task_id.as_deref(),
                agent_data_dir,
            )
            .await
        }
        _ => anyhow::bail!("unsupported memory tool for MCP: {tool_name}"),
    }
}
