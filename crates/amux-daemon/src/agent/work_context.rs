//! Work context tracking — TODOs, file artifacts, repo watching, and event emission.

use std::collections::HashSet;

use super::*;

const RAPID_REVERT_WINDOW_MS: u64 = 30_000;

/// Map a `GoalRunStatus` to an event-kind string for autonomy-level filtering.
fn goal_run_status_to_event_kind(status: GoalRunStatus) -> &'static str {
    match status {
        GoalRunStatus::Completed => "completed",
        GoalRunStatus::Failed | GoalRunStatus::Cancelled => "failed",
        GoalRunStatus::Planning => "planning",
        GoalRunStatus::Running => "step_started",
        GoalRunStatus::AwaitingApproval => "step_started",
        GoalRunStatus::Queued | GoalRunStatus::Paused => "step_detail",
    }
}

impl AgentEngine {
    pub(super) fn emit_task_update(&self, task: &AgentTask, message: Option<String>) {
        let _ = self.event_tx.send(AgentEvent::TaskUpdate {
            task_id: task.id.clone(),
            status: task.status,
            progress: task.progress,
            message,
            task: Some(task.clone()),
        });
    }

    pub(super) fn emit_goal_run_update(&self, goal_run: &GoalRun, message: Option<String>) {
        let event_kind = goal_run_status_to_event_kind(goal_run.status);
        if !super::autonomy::should_emit_event(goal_run.autonomy_level, event_kind) {
            return;
        }
        let _ = self.event_tx.send(AgentEvent::GoalRunUpdate {
            goal_run_id: goal_run.id.clone(),
            status: goal_run.status,
            current_step_index: Some(goal_run.current_step_index),
            message,
            goal_run: Some(goal_run.clone()),
        });
    }

    pub(super) fn emit_todo_update(
        &self,
        thread_id: &str,
        goal_run_id: Option<String>,
        step_index: Option<usize>,
        items: Vec<TodoItem>,
    ) {
        let _ = self.event_tx.send(AgentEvent::TodoUpdate {
            thread_id: thread_id.to_string(),
            goal_run_id,
            step_index,
            items,
        });
    }

    pub(super) fn emit_work_context_update(&self, thread_id: &str, context: ThreadWorkContext) {
        let _ = self.event_tx.send(AgentEvent::WorkContextUpdate {
            thread_id: thread_id.to_string(),
            context,
        });
    }

    pub(super) fn emit_workflow_notice(
        &self,
        thread_id: &str,
        kind: &str,
        message: impl Into<String>,
        details: Option<String>,
    ) {
        let _ = self.event_tx.send(AgentEvent::WorkflowNotice {
            thread_id: thread_id.to_string(),
            kind: kind.to_string(),
            message: message.into(),
            details,
        });
    }

    pub async fn replace_thread_todos(
        &self,
        thread_id: &str,
        mut items: Vec<TodoItem>,
        task_id: Option<&str>,
    ) {
        let now = now_millis();
        for (index, item) in items.iter_mut().enumerate() {
            item.position = index;
            if item.created_at == 0 {
                item.created_at = now;
            }
            item.updated_at = now;
        }

        {
            let mut todos = self.thread_todos.write().await;
            todos.insert(thread_id.to_string(), items.clone());
        }
        self.persist_todos().await;

        let mut goal_run_update = None;
        let mut goal_run_id = None;
        let mut step_index = None;
        if let Some(task_id) = task_id {
            goal_run_update = self.record_goal_run_todo_snapshot(task_id, &items).await;
            if let Some(goal_run) = goal_run_update.as_ref() {
                goal_run_id = Some(goal_run.id.clone());
                step_index = Some(goal_run.current_step_index);
            }
        }

        self.emit_todo_update(thread_id, goal_run_id, step_index, items);

        if let Some(goal_run) = goal_run_update {
            self.persist_goal_runs().await;
            self.emit_goal_run_update(&goal_run, Some("Goal todo updated".into()));
        }
    }

    pub(super) async fn capture_tool_work_context(
        &self,
        thread_id: &str,
        task_id: Option<&str>,
        tool_name: &str,
        args_json: &str,
    ) {
        match tool_name {
            "create_file" | "write_file" | "append_to_file" | "replace_in_file"
            | "apply_file_patch" | "apply_patch" => {
                let Ok(args) = serde_json::from_str::<serde_json::Value>(args_json) else {
                    return;
                };
                if tool_name == "apply_patch" {
                    if let Some(input) = super::tool_executor::get_apply_patch_text_arg(&args) {
                        if let Ok(paths) = super::tool_executor::extract_apply_patch_paths(input) {
                            for path in paths {
                                self.record_file_work_context(thread_id, task_id, tool_name, &path)
                                    .await;
                            }
                            return;
                        }
                    }
                }
                let Some(path) = args
                    .get("path")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return;
                };
                self.record_file_work_context(thread_id, task_id, tool_name, path)
                    .await;
            }
            "run_terminal_command" | "run_bash" | "bash_command" => {
                self.refresh_thread_repo_context(thread_id).await;
            }
            _ => {}
        }
    }

    pub(super) async fn record_generated_skill_work_context(&self, goal_run: &GoalRun) {
        let Some(path) = goal_run.generated_skill_path.as_deref() else {
            return;
        };

        let context = ThreadWorkContext {
            thread_id: goal_run.thread_id.clone().unwrap_or_default(),
            entries: vec![WorkContextEntry {
                path: path.to_string(),
                previous_path: None,
                kind: WorkContextEntryKind::GeneratedSkill,
                source: "generated_skill".to_string(),
                change_kind: None,
                repo_root: crate::git::find_git_root(path),
                goal_run_id: Some(goal_run.id.clone()),
                step_index: Some(goal_run.current_step_index),
                session_id: goal_run.session_id.clone(),
                is_text: true,
                updated_at: now_millis(),
            }],
        };
        if context.thread_id.is_empty() {
            return;
        }
        self.merge_work_context_entries(&context.thread_id, context.entries)
            .await;
    }

    pub(super) async fn record_file_work_context(
        &self,
        thread_id: &str,
        task_id: Option<&str>,
        source: &str,
        path: &str,
    ) {
        let normalized = std::fs::canonicalize(path)
            .unwrap_or_else(|_| std::path::PathBuf::from(path))
            .to_string_lossy()
            .to_string();
        let repo_root = crate::git::find_git_root(&normalized);
        let (goal_run_id, step_index, session_id) = self.goal_context_for_task(task_id).await;
        let (entry_path, kind) = if let Some(repo_root) = repo_root.as_deref() {
            let relative = std::path::Path::new(&normalized)
                .strip_prefix(repo_root)
                .ok()
                .map(|value| value.to_string_lossy().trim_start_matches('/').to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| normalized.clone());
            (relative, WorkContextEntryKind::RepoChange)
        } else {
            (normalized.clone(), WorkContextEntryKind::Artifact)
        };

        self.merge_work_context_entries(
            thread_id,
            vec![WorkContextEntry {
                path: entry_path,
                previous_path: None,
                kind,
                source: source.to_string(),
                change_kind: None,
                repo_root,
                goal_run_id,
                step_index,
                session_id,
                is_text: true,
                updated_at: now_millis(),
            }],
        )
        .await;
        self.refresh_thread_repo_context(thread_id).await;
    }

    pub(super) async fn goal_context_for_task(
        &self,
        task_id: Option<&str>,
    ) -> (Option<String>, Option<usize>, Option<String>) {
        let Some(task_id) = task_id else {
            return (None, None, None);
        };

        let task = {
            let tasks = self.tasks.lock().await;
            tasks.iter().find(|item| item.id == task_id).cloned()
        };
        let Some(task) = task else {
            return (None, None, None);
        };

        let goal_run = if let Some(goal_run_id) = task.goal_run_id.as_deref() {
            let goal_runs = self.goal_runs.lock().await;
            goal_runs
                .iter()
                .find(|item| item.id == goal_run_id)
                .cloned()
        } else {
            None
        };
        let step_index = goal_run.as_ref().map(|item| item.current_step_index);
        (
            task.goal_run_id.clone(),
            step_index,
            task.session_id
                .clone()
                .or_else(|| goal_run.and_then(|item| item.session_id)),
        )
    }

    pub(super) async fn resolve_thread_repo_root(
        &self,
        thread_id: &str,
    ) -> Option<(String, Option<String>, Option<String>, Option<usize>)> {
        let goal_runs = self.goal_runs.lock().await;
        let run = goal_runs
            .iter()
            .filter(|run| run.thread_id.as_deref() == Some(thread_id))
            .max_by_key(|run| run.updated_at)
            .cloned();
        drop(goal_runs);

        let session_id =
            if let Some(run_session_id) = run.as_ref().and_then(|item| item.session_id.clone()) {
                Some(run_session_id)
            } else {
                let tasks = self.tasks.lock().await;
                tasks
                    .iter()
                    .filter(|task| task.thread_id.as_deref() == Some(thread_id))
                    .find_map(|task| task.session_id.clone())
            };

        if let Some(session_id) = session_id.as_deref() {
            if let Some(cwd) = self
                .session_manager
                .list()
                .await
                .into_iter()
                .find(|session| session.id.to_string() == session_id)
                .and_then(|session| session.cwd)
            {
                if let Some(repo_root) = crate::git::find_git_root(&cwd) {
                    return Some((
                        repo_root,
                        run.as_ref().map(|item| item.id.clone()),
                        Some(session_id.to_string()),
                        run.as_ref().map(|item| item.current_step_index),
                    ));
                }
            }
        }

        let existing = self.thread_work_contexts.read().await;
        let repo_root = existing.get(thread_id).and_then(|context| {
            context
                .entries
                .iter()
                .find_map(|entry| entry.repo_root.clone())
        });
        let goal_run_id = run.as_ref().map(|item| item.id.clone());
        let step_index = run.as_ref().map(|item| item.current_step_index);
        repo_root.map(|root| (root, goal_run_id, session_id, step_index))
    }

    pub(super) async fn refresh_thread_repo_context(&self, thread_id: &str) {
        let Some((repo_root, goal_run_id, session_id, step_index)) =
            self.resolve_thread_repo_root(thread_id).await
        else {
            self.remove_repo_watcher(thread_id).await;
            return;
        };

        self.ensure_repo_watcher(thread_id, &repo_root).await;
        let changes = crate::git::list_git_changes(&repo_root);
        let now = now_millis();
        let entries = changes
            .into_iter()
            .map(|entry| WorkContextEntry {
                path: entry.path,
                previous_path: entry.previous_path,
                kind: WorkContextEntryKind::RepoChange,
                source: "repo_scan".to_string(),
                change_kind: Some(entry.kind),
                repo_root: Some(repo_root.clone()),
                goal_run_id: goal_run_id.clone(),
                step_index,
                session_id: session_id.clone(),
                is_text: true,
                updated_at: now,
            })
            .collect::<Vec<_>>();
        self.detect_and_record_rapid_reverts(thread_id, &repo_root, &entries, now)
            .await;
        self.merge_repo_scan_entries(thread_id, &repo_root, entries)
            .await;
        self.maybe_run_aline_startup_reconciliation_for_repo(&repo_root)
            .await;
    }

    async fn detect_and_record_rapid_reverts(
        &self,
        thread_id: &str,
        repo_root: &str,
        fresh_entries: &[WorkContextEntry],
        detected_at: u64,
    ) {
        let context = {
            let contexts = self.thread_work_contexts.read().await;
            contexts.get(thread_id).cloned()
        };
        let Some(context) = context else {
            return;
        };

        let changed_paths = fresh_entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<HashSet<_>>();

        let mut already_recorded = HashSet::new();
        for signal in self
            .history
            .list_implicit_signals(thread_id, 50)
            .await
            .unwrap_or_default()
        {
            if signal.signal_type != "rapid_revert" {
                continue;
            }
            let Some(snapshot) = signal.context_snapshot_json.as_deref() else {
                continue;
            };
            let Ok(snapshot) = serde_json::from_str::<serde_json::Value>(snapshot) else {
                continue;
            };
            if let Some(path) = snapshot.get("path").and_then(|value| value.as_str()) {
                already_recorded.insert(path.to_string());
            }
        }

        for entry in context.entries.iter().filter(|entry| {
            entry.kind == WorkContextEntryKind::RepoChange
                && entry.repo_root.as_deref() == Some(repo_root)
                && entry.source != "repo_scan"
        }) {
            if detected_at.saturating_sub(entry.updated_at) > RAPID_REVERT_WINDOW_MS {
                continue;
            }
            if changed_paths.contains(entry.path.as_str()) {
                continue;
            }
            if !already_recorded.insert(entry.path.clone()) {
                continue;
            }
            if let Err(error) = self
                .record_rapid_revert_feedback(
                    thread_id,
                    &entry.path,
                    &entry.source,
                    entry.repo_root.as_deref(),
                    entry.updated_at,
                    detected_at,
                )
                .await
            {
                tracing::warn!(
                    thread_id = %thread_id,
                    path = %entry.path,
                    error = %error,
                    "failed to record rapid revert implicit feedback"
                );
            }
        }
    }

    async fn merge_repo_scan_entries(
        &self,
        thread_id: &str,
        repo_root: &str,
        fresh_entries: Vec<WorkContextEntry>,
    ) {
        let mut contexts = self.thread_work_contexts.write().await;
        let context = contexts
            .entry(thread_id.to_string())
            .or_insert_with(|| ThreadWorkContext {
                thread_id: thread_id.to_string(),
                entries: Vec::new(),
            });

        context.entries.retain(|entry| {
            !(entry.repo_root.as_deref() == Some(repo_root) && entry.source == "repo_scan")
        });

        for fresh in fresh_entries {
            if let Some(existing) = context
                .entries
                .iter_mut()
                .find(|entry| entry.path == fresh.path && entry.repo_root == fresh.repo_root)
            {
                existing.change_kind = fresh.change_kind.clone();
                existing.previous_path = fresh.previous_path.clone();
                existing.updated_at = fresh.updated_at;
                if existing.source == "repo_scan" {
                    *existing = fresh.clone();
                }
            } else {
                context.entries.push(fresh);
            }
        }
        context
            .entries
            .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let snapshot = context.clone();
        drop(contexts);

        self.persist_work_context().await;
        self.emit_work_context_update(thread_id, snapshot);
    }

    async fn merge_work_context_entries(
        &self,
        thread_id: &str,
        fresh_entries: Vec<WorkContextEntry>,
    ) {
        let mut contexts = self.thread_work_contexts.write().await;
        let context = contexts
            .entry(thread_id.to_string())
            .or_insert_with(|| ThreadWorkContext {
                thread_id: thread_id.to_string(),
                entries: Vec::new(),
            });

        for fresh in fresh_entries {
            if let Some(existing) = context
                .entries
                .iter_mut()
                .find(|entry| entry.path == fresh.path && entry.repo_root == fresh.repo_root)
            {
                *existing = fresh;
            } else {
                context.entries.push(fresh);
            }
        }
        context
            .entries
            .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let snapshot = context.clone();
        drop(contexts);

        self.persist_work_context().await;
        self.emit_work_context_update(thread_id, snapshot);
    }

    pub(super) async fn record_goal_run_todo_snapshot(
        &self,
        task_id: &str,
        items: &[TodoItem],
    ) -> Option<GoalRun> {
        let goal_run_id = {
            let tasks = self.tasks.lock().await;
            tasks
                .iter()
                .find(|task| task.id == task_id)
                .and_then(|task| task.goal_run_id.clone())
        }?;

        let mut goal_runs = self.goal_runs.lock().await;
        let goal_run = goal_runs
            .iter_mut()
            .find(|goal_run| goal_run.id == goal_run_id)?;
        goal_run.updated_at = now_millis();
        goal_run.events.push(make_goal_run_event_with_todos(
            "todo",
            "goal todo updated",
            None,
            Some(goal_run.current_step_index),
            items.to_vec(),
        ));
        Some(goal_run.clone())
    }

    pub(super) async fn mark_task_awaiting_approval(
        &self,
        task_id: &str,
        thread_id: &str,
        pending_approval: &ToolPendingApproval,
    ) {
        let updated = {
            let mut tasks = self.tasks.lock().await;
            let Some(task) = tasks.iter_mut().find(|entry| entry.id == task_id) else {
                return;
            };

            let reason = format!(
                "waiting for operator approval: {}",
                pending_approval.command
            );
            task.status = TaskStatus::AwaitingApproval;
            task.thread_id = Some(thread_id.to_string());
            if task.session_id.is_none() {
                task.session_id = pending_approval.session_id.clone();
            }
            task.awaiting_approval_id = Some(pending_approval.approval_id.clone());
            task.blocked_reason = Some(reason.clone());
            task.error = None;
            task.last_error = None;
            task.progress = task.progress.max(35);
            task.logs.push(make_task_log_entry(
                task.retry_count,
                TaskLogLevel::Warn,
                "approval",
                "managed command paused for operator approval",
                Some(reason),
            ));
            task.clone()
        };

        self.persist_tasks().await;
        self.emit_task_update(&updated, Some("Task awaiting approval".into()));
    }
}
