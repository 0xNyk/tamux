use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use amux_protocol::{
    DaemonMessage, ManagedCommandRequest, ManagedCommandSource, SecurityLevel, SessionId,
};
use anyhow::{Context, Result};
use base64::Engine;
use tokio::io::AsyncReadExt;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::history::{HistoryStore, SkillVariantRecord};
use crate::scrub::scrub_sensitive;
use crate::session_manager::SessionManager;

use super::agent_identity::{
    build_spawned_persona_prompt, canonical_agent_id, canonical_agent_name,
    current_agent_scope_id, extract_persona_name, sender_name_for_task, CONCIERGE_AGENT_ID, CONCIERGE_AGENT_NAME,
    MAIN_AGENT_ID, MAIN_AGENT_NAME,
};
use super::memory::{apply_memory_update, MemoryTarget, MemoryUpdateMode, MemoryWriteContext};
use super::semantic_env::{execute_semantic_query, infer_workspace_context_tags};
use super::session_recall::execute_session_search as run_session_search;
use super::tool_synthesis::{
    activate_generated_tool, detect_cli_wrapper_synthesis_proposal,
    detect_cli_wrapper_synthesis_proposal_from_command, execute_generated_tool,
    find_equivalent_generated_cli_tool, find_equivalent_generated_openapi_tool,
    generated_tool_definitions,
    list_generated_tools, promote_generated_tool, synthesize_tool,
};

use super::types::{
    AgentConfig, AgentEvent, NotificationSeverity, TodoItem, TodoStatus, ToolCall, ToolDefinition,
    ToolFunctionDef, ToolPendingApproval, ToolResult,
};
use super::AgentEngine;

const ONECONTEXT_TOOL_QUERY_MAX_CHARS: usize = 300;
const ONECONTEXT_TOOL_OUTPUT_MAX_CHARS: usize = 12_000;
const DEFAULT_DAEMON_TOOL_TIMEOUT_SECS: u64 = 120;
const MAX_DAEMON_TOOL_TIMEOUT_SECS: u64 = 600;
const SESSION_SEARCH_OUTPUT_MAX_CHARS: usize = 12_000;
const SEARCH_FILES_MAX_RESULTS_CAP: u64 = 200;
const SEARCH_FILES_PROGRAM: &str = "rg";
const SEARCH_FILES_MAX_LINE_BYTES: usize = 64 * 1024;
const SEARCH_FILES_MAX_STDERR_BYTES: usize = 64 * 1024;

#[derive(Clone)]
struct OnecontextSearchRequest {
    bounded_query: String,
    scope: String,
    timeout_seconds: u64,
}

#[derive(Clone)]
struct SearchFilesRequest {
    pattern: String,
    path: String,
    file_pattern: Option<String>,
    max_results: u64,
    timeout_seconds: u64,
}

struct SearchFilesCommandOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
}

#[derive(Clone)]
struct WebSearchRequest {
    query: String,
    max_results: u64,
    timeout_seconds: u64,
}

#[derive(Clone)]
struct FetchUrlRequest {
    url: String,
    max_length: usize,
    timeout_seconds: u64,
}

fn default_timeout_seconds_for_tool(tool_name: &str) -> u64 {
    match tool_name {
        "onecontext_search" | "fetch_url" | "web_search" => 300,
        _ => DEFAULT_DAEMON_TOOL_TIMEOUT_SECS,
    }
}

fn daemon_tool_timeout_seconds(tool_name: &str, args: &serde_json::Value) -> u64 {
    args.get("timeout_seconds")
        .and_then(|value| value.as_u64())
        .unwrap_or_else(|| default_timeout_seconds_for_tool(tool_name))
        .min(MAX_DAEMON_TOOL_TIMEOUT_SECS)
}

fn normalize_onecontext_simple_query(query: &str) -> String {
    let mut normalized = String::with_capacity(query.len());
    let mut last_was_space = true;

    for ch in query.chars() {
        let mapped = if ch.is_alphanumeric() || ch == '_' {
            ch
        } else if ch.is_whitespace() {
            ' '
        } else {
            ' '
        };

        if mapped == ' ' {
            if !last_was_space {
                normalized.push(' ');
            }
            last_was_space = true;
        } else {
            normalized.push(mapped);
            last_was_space = false;
        }
    }

    normalized.trim().to_string()
}

fn plain_text_onecontext_regex(query: &str) -> String {
    normalize_onecontext_simple_query(query)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(".*")
}

pub(super) fn prepare_onecontext_search_query(
    query: &str,
    no_regex: bool,
    max_chars: usize,
) -> Option<String> {
    let prepared = if no_regex {
        plain_text_onecontext_regex(query)
    } else {
        query.trim().to_string()
    };

    let bounded_query = prepared.chars().take(max_chars).collect::<String>();
    if bounded_query.trim().is_empty() {
        return None;
    }

    Some(bounded_query)
}

fn onecontext_search_request(args: &serde_json::Value) -> Result<OnecontextSearchRequest> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'query' argument"))?
        .trim();

    if query.is_empty() {
        return Err(anyhow::anyhow!("'query' must not be empty"));
    }

    let scope = args
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("session");
    if !matches!(scope, "session" | "event" | "turn") {
        return Err(anyhow::anyhow!(
            "invalid 'scope': {scope} (expected session, event, or turn)"
        ));
    }

    if args
        .get("timeout_seconds")
        .is_some_and(|value| value.as_u64().is_none())
    {
        return Err(anyhow::anyhow!(
            "'timeout_seconds' must be a non-negative integer"
        ));
    }

    let no_regex = args
        .get("no_regex")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let Some(bounded_query) = prepare_onecontext_search_query(
        query,
        no_regex,
        ONECONTEXT_TOOL_QUERY_MAX_CHARS,
    ) else {
        return Err(anyhow::anyhow!(
            "'query' must contain at least one searchable keyword"
        ));
    };

    Ok(OnecontextSearchRequest {
        bounded_query,
        scope: scope.to_string(),
        timeout_seconds: daemon_tool_timeout_seconds("onecontext_search", args),
    })
}

fn search_files_request(args: &serde_json::Value) -> Result<SearchFilesRequest> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'pattern' argument"))?
        .trim();

    if pattern.is_empty() {
        return Err(anyhow::anyhow!("'pattern' must not be empty"));
    }

    if args
        .get("timeout_seconds")
        .is_some_and(|value| value.as_u64().is_none())
    {
        return Err(anyhow::anyhow!(
            "'timeout_seconds' must be a non-negative integer"
        ));
    }

    if args
        .get("max_results")
        .is_some_and(|value| value.as_u64().is_none())
    {
        return Err(anyhow::anyhow!(
            "'max_results' must be a non-negative integer"
        ));
    }

    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".")
        .to_string();
    validate_read_path(&path)?;
    let path = resolve_search_files_path(&path)?;

    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .max(1)
        .min(SEARCH_FILES_MAX_RESULTS_CAP);

    Ok(SearchFilesRequest {
        pattern: pattern.to_string(),
        path,
        file_pattern: args
            .get("file_pattern")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        max_results,
        timeout_seconds: daemon_tool_timeout_seconds("search_files", args),
    })
}
