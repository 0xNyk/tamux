#![allow(dead_code)]

//! Thread CRUD operations — list, get, delete, planner detection.

use super::*;
use serde::{Deserialize, Serialize};

const SESSION_ABANDON_WINDOW_MS: u64 = 30_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ThreadListFilter {
    pub created_after: Option<u64>,
    pub created_before: Option<u64>,
    pub updated_after: Option<u64>,
    pub updated_before: Option<u64>,
    pub agent_name: Option<String>,
    pub title_query: Option<String>,
    pub pinned: Option<bool>,
    pub include_internal: bool,
    pub limit: Option<usize>,
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ThreadDetailResult {
    pub thread: AgentThread,
    pub messages_truncated: bool,
    pub total_message_count: usize,
    pub loaded_message_start: usize,
    pub loaded_message_end: usize,
}

fn thread_detail_frame_fits_ipc(thread: &Option<AgentThread>) -> bool {
    let Ok(thread_json) = serde_json::to_string(thread) else {
        return false;
    };

    amux_protocol::daemon_message_fits_ipc(&amux_protocol::DaemonMessage::AgentThreadDetail {
        thread_json,
    })
}

impl AgentEngine {
    pub async fn set_thread_client_surface(
        &self,
        thread_id: &str,
        client_surface: amux_protocol::ClientSurface,
    ) {
        self.thread_client_surfaces
            .write()
            .await
            .insert(thread_id.to_string(), client_surface);
        let thread_exists = self.threads.read().await.contains_key(thread_id);
        if thread_exists {
            self.persist_thread_by_id(thread_id).await;
        }
    }

    pub async fn get_thread_client_surface(
        &self,
        thread_id: &str,
    ) -> Option<amux_protocol::ClientSurface> {
        self.thread_client_surfaces
            .read()
            .await
            .get(thread_id)
            .copied()
    }

    pub async fn clear_thread_client_surface(&self, thread_id: &str) {
        self.thread_client_surfaces.write().await.remove(thread_id);
    }

    pub async fn set_thread_skill_discovery_state(
        &self,
        thread_id: &str,
        state: LatestSkillDiscoveryState,
    ) {
        self.thread_skill_discovery_states
            .write()
            .await
            .insert(thread_id.to_string(), state);
        let thread_exists = self.threads.read().await.contains_key(thread_id);
        if thread_exists {
            self.persist_thread_by_id(thread_id).await;
        }
    }

    pub async fn get_thread_skill_discovery_state(
        &self,
        thread_id: &str,
    ) -> Option<LatestSkillDiscoveryState> {
        self.thread_skill_discovery_states
            .read()
            .await
            .get(thread_id)
            .cloned()
    }

    pub async fn clear_thread_skill_discovery_state(&self, thread_id: &str) {
        self.thread_skill_discovery_states
            .write()
            .await
            .remove(thread_id);
        let thread_exists = self.threads.read().await.contains_key(thread_id);
        if thread_exists {
            self.persist_thread_by_id(thread_id).await;
        }
    }

    pub async fn set_thread_memory_injection_state(
        &self,
        thread_id: &str,
        state: PromptMemoryInjectionState,
    ) {
        self.thread_memory_injection_state_map()
            .write()
            .await
            .insert(thread_id.to_string(), state);
        let thread_exists = self.threads.read().await.contains_key(thread_id);
        if thread_exists {
            self.persist_thread_by_id(thread_id).await;
        }
    }

    pub async fn get_thread_memory_injection_state(
        &self,
        thread_id: &str,
    ) -> Option<PromptMemoryInjectionState> {
        self.thread_memory_injection_state_map()
            .read()
            .await
            .get(thread_id)
            .cloned()
    }

    pub async fn clear_thread_memory_injection_state(&self, thread_id: &str) {
        self.thread_memory_injection_state_map()
            .write()
            .await
            .remove(thread_id);
        let thread_exists = self.threads.read().await.contains_key(thread_id);
        if thread_exists {
            self.persist_thread_by_id(thread_id).await;
        }
    }

    pub async fn set_goal_run_client_surface(
        &self,
        goal_run_id: &str,
        client_surface: amux_protocol::ClientSurface,
    ) {
        self.goal_run_client_surfaces
            .write()
            .await
            .insert(goal_run_id.to_string(), client_surface);
    }

    pub async fn get_goal_run_client_surface(
        &self,
        goal_run_id: &str,
    ) -> Option<amux_protocol::ClientSurface> {
        self.goal_run_client_surfaces
            .read()
            .await
            .get(goal_run_id)
            .copied()
    }

    pub async fn list_threads(&self) -> Vec<AgentThread> {
        self.list_threads_filtered(&ThreadListFilter::default())
            .await
    }

    pub(crate) async fn list_threads_filtered(
        &self,
        filter: &ThreadListFilter,
    ) -> Vec<AgentThread> {
        let threads = self.threads.read().await;
        let mut list: Vec<AgentThread> = threads
            .values()
            .filter(|thread| thread_matches_list_filter(thread, filter))
            .map(summarize_thread_for_list)
            .collect();

        list.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });

        let limit = filter.limit.unwrap_or(usize::MAX);
        list.into_iter().skip(filter.offset).take(limit).collect()
    }

    pub async fn get_thread(&self, thread_id: &str) -> Option<AgentThread> {
        self.get_thread_filtered(thread_id, false, None, 0)
            .await
            .map(|result| result.thread)
    }

    pub(crate) async fn get_thread_filtered(
        &self,
        thread_id: &str,
        include_internal: bool,
        message_limit: Option<usize>,
        message_offset: usize,
    ) -> Option<ThreadDetailResult> {
        let mut thread = self.threads.read().await.get(thread_id).cloned()?;
        if !thread_is_query_visible(&thread, include_internal) {
            return None;
        }

        let total_messages = thread.messages.len();
        let end = total_messages.saturating_sub(message_offset);
        let start = message_limit
            .map(|limit| end.saturating_sub(limit))
            .unwrap_or(0);
        let messages_truncated = start > 0 || end < total_messages;

        if messages_truncated {
            thread.messages = thread
                .messages
                .into_iter()
                .skip(start)
                .take(end.saturating_sub(start))
                .collect();
        }

        Some(ThreadDetailResult {
            thread,
            messages_truncated,
            total_message_count: total_messages,
            loaded_message_start: start,
            loaded_message_end: end,
        })
    }

    pub(crate) async fn get_thread_capped_for_ipc(
        &self,
        thread_id: &str,
        include_internal: bool,
    ) -> Option<ThreadDetailResult> {
        let detail = self
            .get_thread_filtered(thread_id, include_internal, None, 0)
            .await?;

        if thread_detail_frame_fits_ipc(&Some(detail.thread.clone())) {
            return Some(detail);
        }

        let mut low = 0usize;
        let mut high = detail.thread.messages.len();
        while low < high {
            let mid = low + (high - low) / 2;
            let mut candidate = detail.thread.clone();
            candidate.messages = candidate.messages[mid..].to_vec();
            if thread_detail_frame_fits_ipc(&Some(candidate)) {
                high = mid;
            } else {
                low = mid + 1;
            }
        }

        let mut thread = detail.thread;
        thread.messages = thread.messages[low..].to_vec();

        Some(ThreadDetailResult {
            thread,
            messages_truncated: detail.messages_truncated || low > 0,
            total_message_count: detail.total_message_count,
            loaded_message_start: detail.loaded_message_start + low,
            loaded_message_end: detail.loaded_message_end,
        })
    }

    pub async fn planner_required_for_thread(&self, thread_id: &str) -> bool {
        let threads = self.threads.read().await;
        let Some(thread) = threads.get(thread_id) else {
            return false;
        };
        let latest_user_message = thread
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .map(|message| message.content.as_str())
            .unwrap_or("");
        planner_required_for_message(latest_user_message)
    }

    pub async fn delete_thread(&self, thread_id: &str) -> bool {
        let thread_snapshot = self.threads.read().await.get(thread_id).cloned();
        let removed = self.threads.write().await.remove(thread_id).is_some();
        if removed {
            if let Some(thread) = thread_snapshot.as_ref() {
                self.maybe_record_session_abandon_on_thread_delete(thread)
                    .await;
            }
            self.clear_thread_client_surface(thread_id).await;
            self.clear_thread_skill_discovery_state(thread_id).await;
            self.clear_thread_memory_injection_state(thread_id).await;
            self.clear_thread_structural_memory(thread_id).await;
            self.thread_handoff_states.write().await.remove(thread_id);
            self.thread_participants.write().await.remove(thread_id);
            self.thread_participant_suggestions
                .write()
                .await
                .remove(thread_id);
            self.remove_repo_watcher(thread_id).await;
            self.thread_todos.write().await.remove(thread_id);
            self.thread_work_contexts.write().await.remove(thread_id);
            self.persist_threads().await;
            self.persist_todos().await;
            self.persist_work_context().await;
        }
        removed
    }

    async fn maybe_record_session_abandon_on_thread_delete(&self, thread: &AgentThread) {
        let now = now_millis();
        let Some(last_assistant) = thread
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::Assistant)
        else {
            return;
        };

        if now.saturating_sub(last_assistant.timestamp) > SESSION_ABANDON_WINDOW_MS {
            return;
        }

        let last_user_after_assistant = thread
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .is_some_and(|message| message.timestamp > last_assistant.timestamp);
        if last_user_after_assistant {
            return;
        }

        let recent_existing = self
            .history
            .list_implicit_signals(&thread.id, 10)
            .await
            .unwrap_or_default();
        if recent_existing
            .iter()
            .any(|signal| signal.signal_type == "session_abandon")
        {
            return;
        }

        if let Err(error) = self
            .record_session_abandon_feedback(
                &thread.id,
                last_assistant.content.trim(),
                last_assistant.timestamp,
                now,
            )
            .await
        {
            tracing::warn!(
                thread_id = %thread.id,
                error = %error,
                "failed to record session abandonment feedback on thread delete"
            );
        }
    }
}

fn thread_is_visible_by_default(thread: &AgentThread) -> bool {
    !crate::agent::concierge::is_user_visible_thread(thread)
        && !crate::agent::agent_identity::is_participant_playground_thread(&thread.id)
        && !crate::agent::is_internal_handoff_thread(&thread.id)
}

fn thread_is_query_visible(thread: &AgentThread, include_internal: bool) -> bool {
    include_internal || thread_is_visible_by_default(thread)
}

fn canonical_thread_agent_name(agent_name: Option<&str>) -> &'static str {
    let normalized = agent_name.unwrap_or("").trim();
    if normalized.is_empty() {
        return crate::agent::agent_identity::canonical_agent_name(
            crate::agent::agent_identity::MAIN_AGENT_ID,
        );
    }

    crate::agent::agent_identity::canonical_agent_name(normalized)
}

fn thread_matches_list_filter(thread: &AgentThread, filter: &ThreadListFilter) -> bool {
    if !thread_is_query_visible(thread, filter.include_internal) {
        return false;
    }

    if let Some(created_after) = filter.created_after {
        if thread.created_at < created_after {
            return false;
        }
    }

    if let Some(created_before) = filter.created_before {
        if thread.created_at > created_before {
            return false;
        }
    }

    if let Some(updated_after) = filter.updated_after {
        if thread.updated_at < updated_after {
            return false;
        }
    }

    if let Some(updated_before) = filter.updated_before {
        if thread.updated_at > updated_before {
            return false;
        }
    }

    if let Some(pinned) = filter.pinned {
        if thread.pinned != pinned {
            return false;
        }
    }

    if let Some(agent_name) = filter.agent_name.as_deref() {
        let expected = canonical_thread_agent_name(Some(agent_name));
        let actual = canonical_thread_agent_name(thread.agent_name.as_deref());
        if !actual.eq_ignore_ascii_case(expected) {
            return false;
        }
    }

    if let Some(title_query) = filter
        .title_query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
    {
        if !thread
            .title
            .to_ascii_lowercase()
            .contains(&title_query.to_ascii_lowercase())
        {
            return false;
        }
    }

    true
}

fn summarize_thread_for_list(thread: &AgentThread) -> AgentThread {
    AgentThread {
        id: thread.id.clone(),
        agent_name: thread.agent_name.clone(),
        title: thread.title.clone(),
        messages: Vec::new(),
        pinned: thread.pinned,
        upstream_thread_id: thread.upstream_thread_id.clone(),
        upstream_transport: thread.upstream_transport,
        upstream_provider: thread.upstream_provider.clone(),
        upstream_model: thread.upstream_model.clone(),
        upstream_assistant_id: thread.upstream_assistant_id.clone(),
        created_at: thread.created_at,
        updated_at: thread.updated_at,
        total_input_tokens: thread.total_input_tokens,
        total_output_tokens: thread.total_output_tokens,
    }
}

#[cfg(test)]
#[path = "tests/thread_crud.rs"]
mod tests;
