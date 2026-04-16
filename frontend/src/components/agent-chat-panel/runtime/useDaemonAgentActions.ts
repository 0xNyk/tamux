import { useCallback, useState, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import { useAgentStore } from "@/lib/agentStore";
import { getProviderDefinition } from "@/lib/agentStore/providers";
import type { AgentProviderId } from "@/lib/agentStore/types";
import { getAgentBridge, shouldUseDaemonRuntime } from "@/lib/agentDaemonConfig";
import { provisionAgentWorkspaceTerminals, provisionTerminalPaneInWorkspace, resolvePaneSessionId } from "@/lib/agentWorkspace";
import { startGoalRun, goalRunSupportAvailable, type GoalRun } from "@/lib/goalRuns";
import { useWorkspaceStore } from "@/lib/workspaceStore";
import { appendDaemonSystemMessage, normalizeBridgePayload } from "./daemonHelpers";
import { parseLeadingAgentDirective, type AgentDirective } from "./agentDirective";
import type { BuiltinAgentSetupState } from "./types";

const BUILTIN_PERSONA_ALIASES = [
  "swarozyc",
  "radogost",
  "domowoj",
  "swietowit",
  "perun",
  "mokosh",
  "dazhbog",
] as const;

type PendingBuiltinAgentSetup = BuiltinAgentSetupState & {
  directive: AgentDirective;
  threadId: string | null;
};

function isBuiltinPersonaAlias(agentAlias: string): boolean {
  return BUILTIN_PERSONA_ALIASES.includes(agentAlias.trim().toLowerCase() as (typeof BUILTIN_PERSONA_ALIASES)[number]);
}

function isBuiltinPersonaSetupError(error: string | undefined, targetAgentId: string): boolean {
  const normalizedError = (error ?? "").toLowerCase();
  return normalizedError.includes(`builtin agent '${targetAgentId.toLowerCase()}' is not configured`);
}

function builtinPersonaDisplayName(agentAlias: string): string {
  const normalized = agentAlias.trim().toLowerCase();
  return normalized ? `${normalized.charAt(0).toUpperCase()}${normalized.slice(1)}` : agentAlias;
}

export function useDaemonAgentActions({
  activePaneId,
  activeThreadId,
  activeWorkspace,
  addMessage,
  addNotification,
  agentSettings,
  createThread,
  daemonThreadIdRef,
  daemonLocalThreadRef,
  goalRunWorkspacesRef,
  goalRunsForTrace,
  latestDivergentSessionId,
  setActiveThread,
  setLatestDivergentSessionId,
  setView,
}: {
  activePaneId: string | null;
  activeThreadId: string | null;
  activeWorkspace: ReturnType<ReturnType<typeof useWorkspaceStore.getState>["activeWorkspace"]>;
  addMessage: ReturnType<typeof useAgentStore.getState>["addMessage"];
  addNotification: ReturnType<typeof import("@/lib/notificationStore").useNotificationStore.getState>["addNotification"];
  agentSettings: ReturnType<typeof useAgentStore.getState>["agentSettings"];
  createThread: ReturnType<typeof useAgentStore.getState>["createThread"];
  daemonThreadIdRef: MutableRefObject<string | null>;
  daemonLocalThreadRef: MutableRefObject<string | null>;
  goalRunWorkspacesRef: MutableRefObject<Record<string, string>>;
  goalRunsForTrace: GoalRun[];
  latestDivergentSessionId: string | null;
  setActiveThread: ReturnType<typeof useAgentStore.getState>["setActiveThread"];
  setLatestDivergentSessionId: Dispatch<SetStateAction<string | null>>;
  setView: Dispatch<SetStateAction<import("./types").AgentChatPanelView>>;
}) {
  const [pendingBuiltinAgentSetup, setPendingBuiltinAgentSetup] = useState<PendingBuiltinAgentSetup | null>(null);

  const runDirective = useCallback(async (
    directive: AgentDirective,
    currentThreadId: string | null,
    promptForSetup = true,
  ) => {
    const amux = getAgentBridge();
    const daemonThreadId = daemonThreadIdRef.current;
    const targetAgentId = directive.agentAlias.trim().toLowerCase();
    const defaultProviderId = agentSettings.active_provider;
    const defaultProviderConfig = agentSettings[defaultProviderId] as { model?: string } | undefined;
    const defaultModel = defaultProviderConfig?.model?.trim()
      || getProviderDefinition(defaultProviderId)?.defaultModel
      || "";

    if (directive.kind === "internal_delegate") {
      if (!amux?.agentInternalDelegate) {
        appendDaemonSystemMessage("Internal delegation is not available in this runtime.", currentThreadId);
        return true;
      }
      const response = await amux.agentInternalDelegate(
        daemonThreadId ?? null,
        directive.agentAlias,
        directive.body,
        null,
      );
      const payload = normalizeBridgePayload(response);
      if (payload?.ok === false && typeof payload?.error === "string" && promptForSetup && isBuiltinPersonaAlias(targetAgentId) && isBuiltinPersonaSetupError(payload.error, targetAgentId)) {
        setPendingBuiltinAgentSetup({
          targetAgentId,
          targetAgentName: builtinPersonaDisplayName(directive.agentAlias),
          providerId: defaultProviderId,
          model: defaultModel,
          error: null,
          directive,
          threadId: daemonThreadId ?? currentThreadId ?? null,
        });
        return true;
      }
      appendDaemonSystemMessage(
        payload?.ok === false && typeof payload?.error === "string"
          ? `Failed to delegate to ${directive.agentAlias}: ${payload.error}`
          : `Delegated internally to ${directive.agentAlias}.`,
        currentThreadId,
      );
      return true;
    }

    if (!daemonThreadId) {
      appendDaemonSystemMessage(
        "Participant commands require a daemon-linked thread.",
        currentThreadId,
      );
      return true;
    }
    if (!amux?.agentThreadParticipantCommand) {
      appendDaemonSystemMessage("Thread participants are not available in this runtime.", currentThreadId);
      return true;
    }
    const response = await amux.agentThreadParticipantCommand({
      threadId: daemonThreadId,
      targetAgentId: directive.agentAlias,
      action: directive.kind === "participant_deactivate" ? "deactivate" : "upsert",
      instruction: directive.kind === "participant_upsert" ? directive.body : null,
      sessionId: null,
    });
    const payload = normalizeBridgePayload(response);
    if (payload?.ok === false && typeof payload?.error === "string" && promptForSetup && isBuiltinPersonaAlias(targetAgentId) && isBuiltinPersonaSetupError(payload.error, targetAgentId)) {
      setPendingBuiltinAgentSetup({
        targetAgentId,
        targetAgentName: builtinPersonaDisplayName(directive.agentAlias),
        providerId: defaultProviderId,
        model: defaultModel,
        error: null,
        directive,
        threadId: daemonThreadId,
      });
      return true;
    }
    appendDaemonSystemMessage(
      payload?.ok === false && typeof payload?.error === "string"
        ? `Failed to update participant ${directive.agentAlias}: ${payload.error}`
        : directive.kind === "participant_deactivate"
          ? `Participant ${directive.agentAlias} stopped.`
          : `Participant ${directive.agentAlias} updated.`,
      currentThreadId,
    );
    return true;
  }, [
    agentSettings,
    daemonThreadIdRef,
  ]);

  const submitBuiltinAgentSetup = useCallback(async (providerId: AgentProviderId, model: string) => {
    const amux = getAgentBridge();
    const pending = pendingBuiltinAgentSetup;
    if (!pending || !amux?.agentSetTargetAgentProviderModel) {
      return;
    }
    const response = await amux.agentSetTargetAgentProviderModel(
      pending.targetAgentId,
      providerId,
      model,
    );
    const payload = normalizeBridgePayload(response);
    if (payload?.ok === false && typeof payload?.error === "string") {
      setPendingBuiltinAgentSetup({
        ...pending,
        providerId,
        model,
        error: payload.error,
      });
      return;
    }
    setPendingBuiltinAgentSetup(null);
    await runDirective(pending.directive, pending.threadId, false);
  }, [pendingBuiltinAgentSetup, runDirective]);

  const cancelBuiltinAgentSetup = useCallback(() => {
    setPendingBuiltinAgentSetup(null);
  }, []);

  const sendDaemonMessage = useCallback((text: string) => {
    const amux = getAgentBridge();
    if (!amux?.agentSendMessage) {
      return false;
    }
    const sendAgentMessage = amux.agentSendMessage;

    void (async () => {
      const trimmed = text.trim();
      const currentThreadId = daemonLocalThreadRef.current ?? activeThreadId;

      if (trimmed === "!explain") {
        const latestGoalRun = [...goalRunsForTrace].sort((a, b) => (b.created_at ?? 0) - (a.created_at ?? 0))[0];
        if (latestGoalRun?.id && amux.agentExplainAction) {
          const response = await amux.agentExplainAction(latestGoalRun.id, null);
          const payload = normalizeBridgePayload(response);
          appendDaemonSystemMessage(
            payload?.ok === false && typeof payload?.error === "string"
              ? `Failed to explain action: ${payload.error}`
              : `Explainability\n\n\`\`\`json\n${JSON.stringify(payload, null, 2)}\n\`\`\``,
            currentThreadId,
          );
        } else {
          appendDaemonSystemMessage("No goal run available to explain.", currentThreadId);
        }
        return;
      }

      if (trimmed.startsWith("!diverge ")) {
        const problemStatement = trimmed.slice("!diverge ".length).trim();
        const daemonThreadId = daemonThreadIdRef.current;
        if (problemStatement && daemonThreadId && amux.agentStartDivergentSession) {
          const response = await amux.agentStartDivergentSession({
            problemStatement,
            threadId: daemonThreadId,
            goalRunId: null,
          });
          const payload = normalizeBridgePayload(response);
          if (payload?.ok === false && typeof payload?.error === "string") {
            appendDaemonSystemMessage(`Failed to start divergent session: ${payload.error}`, currentThreadId);
          } else {
            const sessionId = typeof payload.session_id === "string" ? payload.session_id : null;
            if (sessionId) {
              setLatestDivergentSessionId(sessionId);
            }
            appendDaemonSystemMessage(
              sessionId
                ? `Divergent session started: \`${sessionId}\`.\nType \`!diverge-get\` to fetch it.`
                : "Divergent session started.",
              currentThreadId,
            );
          }
        } else {
          appendDaemonSystemMessage(
            "Usage: !diverge <problem>. Also ensure this thread is linked to a daemon thread.",
            currentThreadId,
          );
        }
        return;
      }

      if (trimmed.startsWith("!diverge-get")) {
        const explicitSessionId = trimmed.slice("!diverge-get".length).trim();
        const sessionId = explicitSessionId || latestDivergentSessionId || "";
        if (sessionId && amux.agentGetDivergentSession) {
          const response = await amux.agentGetDivergentSession(sessionId);
          const payload = normalizeBridgePayload(response);
          appendDaemonSystemMessage(
            payload?.ok === false && typeof payload?.error === "string"
              ? `Failed to fetch divergent session: ${payload.error}`
              : `Divergent session payload\n\n\`\`\`json\n${JSON.stringify(payload, null, 2)}\n\`\`\``,
            currentThreadId,
          );
        } else {
          appendDaemonSystemMessage(
            "No divergent session id cached yet. Start one with `!diverge <problem>` first, or pass `!diverge-get <session_id>`.",
            currentThreadId,
          );
        }
        return;
      }

      const knownAgentAliases = [
        "main",
        "svarog",
        "swarog",
        "weles",
        "veles",
        "rarog",
        "swarozyc",
        "radogost",
        "domowoj",
        "swietowit",
        "perun",
        "mokosh",
        "dazhbog",
        ...useAgentStore.getState().subAgents.flatMap((agent) => [agent.id, agent.name]),
      ].filter(Boolean);
      const directive = parseLeadingAgentDirective(text, knownAgentAliases);
      if (directive) {
        await runDirective(directive, currentThreadId);
        return;
      }

      let threadId = activeThreadId || daemonLocalThreadRef.current;
      if (!threadId) {
        const provision = await provisionAgentWorkspaceTerminals({
          title: text.slice(0, 50) || "Agent Conversation",
          cwd: activeWorkspace?.cwd ?? null,
        });
        threadId = createThread({
          workspaceId: provision?.workspaceId ?? activeWorkspace?.id ?? null,
          surfaceId: provision?.surfaceId ?? activeWorkspace?.surfaces?.[0]?.id ?? null,
          paneId: provision?.coordinatorPaneId ?? activePaneId ?? null,
          title: text.slice(0, 50),
        });
        setView("chat");
      }

      let thread = useAgentStore.getState().threads.find((entry) => entry.id === threadId);
      let preferredSessionId = thread?.paneId ? resolvePaneSessionId(thread.paneId) : null;
      if (!preferredSessionId && thread?.workspaceId) {
        const pane = await provisionTerminalPaneInWorkspace({
          workspaceId: thread.workspaceId,
          paneName: "Coordinator",
          cwd: activeWorkspace?.cwd ?? null,
          reusePrimaryPane: true,
        });
        preferredSessionId = pane?.sessionId ?? null;
      } else if (!preferredSessionId) {
        const provision = await provisionAgentWorkspaceTerminals({
          title: thread?.title || text.slice(0, 50) || "Agent Conversation",
          cwd: activeWorkspace?.cwd ?? null,
        });
        preferredSessionId = provision?.coordinatorSessionId ?? null;
      }

      if (useAgentStore.getState().activeThreadId !== threadId) {
        setActiveThread(threadId);
      }
      if (!threadId) return;

      addMessage(threadId, {
        role: "user",
        content: text,
        inputTokens: 0,
        outputTokens: 0,
        totalTokens: 0,
        isCompactionSummary: false,
      });

      const isExternalAgent = agentSettings.agent_backend === "openclaw" || agentSettings.agent_backend === "hermes";
      addMessage(threadId, {
        role: "assistant",
        content: "",
        provider: isExternalAgent ? agentSettings.agent_backend : agentSettings.active_provider,
        model: isExternalAgent ? agentSettings.agent_backend : ((agentSettings[agentSettings.active_provider] as any)?.model || "unknown"),
        inputTokens: 0,
        outputTokens: 0,
        totalTokens: 0,
        isCompactionSummary: false,
        isStreaming: true,
      });

      daemonLocalThreadRef.current = threadId;

      let contextMessages: unknown[] | undefined;
      {
        const existingMessages = useAgentStore.getState().getThreadMessages(threadId);
        const historyMessages = existingMessages
          .filter((message) => !message.isStreaming && !message.isCompactionSummary)
          .slice(0, -1);
        if (historyMessages.length > 0) {
          contextMessages = historyMessages.map((message, index) => ({
            id: `${threadId}:ctx:${index}`,
            thread_id: threadId,
            created_at: message.createdAt ?? Date.now(),
            role: message.role,
            content: message.content,
            provider: message.provider ?? null,
            model: message.model ?? null,
            input_tokens: message.inputTokens ?? 0,
            output_tokens: message.outputTokens ?? 0,
            total_tokens: message.totalTokens ?? 0,
            reasoning: message.reasoning ?? null,
            tool_calls_json: message.toolCalls ? JSON.stringify(message.toolCalls) : null,
            metadata_json: message.toolName ? JSON.stringify({
              toolCallId: message.toolCallId,
              toolName: message.toolName,
              toolArguments: message.toolArguments,
              toolStatus: message.toolStatus,
              weles_review: message.welesReview,
            }) : null,
          }));
        }
      }

      const daemonThreadId = daemonThreadIdRef.current;
      await sendAgentMessage(daemonThreadId || threadId, text, preferredSessionId, contextMessages);
    })();

    return true;
  }, [
    activePaneId,
    activeThreadId,
    activeWorkspace,
    addMessage,
    agentSettings,
    createThread,
    daemonLocalThreadRef,
    daemonThreadIdRef,
    goalRunsForTrace,
    latestDivergentSessionId,
    setActiveThread,
    setLatestDivergentSessionId,
    setView,
    runDirective,
  ]);

  const startGoalRunFromPrompt = useCallback(async (text: string) => {
    const goal = text.trim();
    if (!goal || !goalRunSupportAvailable()) {
      return false;
    }

    let threadId = activeThreadId;
    if (!threadId && daemonLocalThreadRef.current) {
      threadId = daemonLocalThreadRef.current;
      setActiveThread(threadId);
    }
    if (!threadId) {
      const workspaceId = useWorkspaceStore.getState().activeWorkspaceId;
      const surfaceId = useWorkspaceStore.getState().activeSurface()?.id ?? null;
      const paneId = useWorkspaceStore.getState().activePaneId();
      threadId = createThread({
        workspaceId,
        surfaceId,
        paneId,
        title: goal.slice(0, 50),
      });
    }

    const provision = await provisionAgentWorkspaceTerminals({
      title: goal,
      cwd: activeWorkspace?.cwd ?? null,
    });

    const effectiveThreadId = daemonThreadIdRef.current || threadId;
    daemonLocalThreadRef.current = threadId;

    addMessage(threadId, {
      role: "user",
      content: goal,
      inputTokens: 0,
      outputTokens: 0,
      totalTokens: 0,
      isCompactionSummary: false,
    });
    setActiveThread(threadId);

    const run = await startGoalRun({
      goal,
      title: goal.slice(0, 72),
      priority: "normal",
      threadId: effectiveThreadId,
      sessionId: provision?.coordinatorSessionId ?? null,
    });

    if (run?.id && provision?.workspaceId) {
      goalRunWorkspacesRef.current[run.id] = provision.workspaceId;
    }

    if (!run) {
      addNotification({
        title: "Goal runner unavailable",
        body: "Could not start long-running goal",
        subtitle: "Backend goal-run IPC is not available yet.",
        icon: "alert-triangle",
        source: "system",
        workspaceId: provision?.workspaceId ?? activeWorkspace?.id ?? null,
        paneId: provision?.coordinatorPaneId ?? activePaneId ?? null,
        panelId: provision?.coordinatorPaneId ?? activePaneId ?? null,
      });
      return false;
    }

    addNotification({
      title: "Goal runner started",
      body: run.title,
      subtitle: run.plan_summary || "The daemon is planning the run.",
      icon: "sparkles",
      source: "system",
      workspaceId: provision?.workspaceId ?? activeWorkspace?.id ?? null,
      paneId: provision?.coordinatorPaneId ?? activePaneId ?? null,
      panelId: provision?.coordinatorPaneId ?? activePaneId ?? null,
    });
    setView("tasks");
    return true;
  }, [
    activePaneId,
    activeThreadId,
    activeWorkspace,
    addMessage,
    addNotification,
    createThread,
    daemonLocalThreadRef,
    daemonThreadIdRef,
    goalRunWorkspacesRef,
    setActiveThread,
    setView,
  ]);

  return {
    builtinAgentSetup: pendingBuiltinAgentSetup,
    canStartGoalRun: shouldUseDaemonRuntime(agentSettings.agent_backend) && goalRunSupportAvailable(),
    cancelBuiltinAgentSetup,
    sendDaemonMessage,
    startGoalRunFromPrompt,
    submitBuiltinAgentSetup,
  };
}
