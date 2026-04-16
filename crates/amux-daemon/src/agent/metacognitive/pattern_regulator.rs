use serde::{Deserialize, Serialize};

use crate::agent::engine::AgentEngine;
use crate::agent::explanation::ConfidenceBand;
use crate::agent::generate_message_id;
use crate::agent::metacognitive::introspector::{
    BiasSignal, InterventionStrength, IntrospectionOutcome,
};
use crate::agent::now_millis;
use crate::agent::types::{AgentMessage, AgentMessageKind, MessageRole, ToolCall};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InterventionAction {
    Allow,
    Warn,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegulationDecision {
    pub action: InterventionAction,
    pub summary: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_adjustment: Option<f64>,
}

pub fn regulate(outcome: &IntrospectionOutcome, tool_call: &ToolCall) -> RegulationDecision {
    match outcome.strength {
        InterventionStrength::None => RegulationDecision {
            action: InterventionAction::Allow,
            summary: "No metacognitive intervention needed.".to_string(),
            warnings: Vec::new(),
            system_message: None,
            confidence_adjustment: None,
        },
        InterventionStrength::Warn => RegulationDecision {
            action: InterventionAction::Warn,
            summary: format!(
                "Meta-cognitive warning before `{}`: {}",
                tool_call.function.name,
                joined_warnings(&outcome.signals)
            ),
            warnings: outcome
                .signals
                .iter()
                .map(render_warning)
                .collect::<Vec<_>>(),
            system_message: Some(build_reflection_message(tool_call, &outcome.signals, false)),
            confidence_adjustment: outcome.confidence_adjustment,
        },
        InterventionStrength::Block => RegulationDecision {
            action: InterventionAction::Block,
            summary: format!(
                "Meta-cognitive block before `{}`: {}",
                tool_call.function.name,
                joined_warnings(&outcome.signals)
            ),
            warnings: outcome
                .signals
                .iter()
                .map(render_warning)
                .collect::<Vec<_>>(),
            system_message: Some(build_reflection_message(tool_call, &outcome.signals, true)),
            confidence_adjustment: outcome.confidence_adjustment,
        },
    }
}

impl AgentEngine {
    pub(crate) async fn append_metacognitive_system_message(&self, thread_id: &str, content: &str) {
        let now = now_millis();
        {
            let mut threads = self.threads.write().await;
            if let Some(thread) = threads.get_mut(thread_id) {
                thread.messages.push(AgentMessage {
                    id: generate_message_id(),
                    role: MessageRole::System,
                    content: content.to_string(),
                    tool_calls: None,
                    tool_call_id: None,
                    tool_name: None,
                    tool_arguments: None,
                    tool_status: None,
                    weles_review: None,
                    input_tokens: 0,
                    output_tokens: 0,
                    cost: None,
                    provider: None,
                    model: None,
                    api_transport: None,
                    response_id: None,
                    upstream_message: None,
                    provider_final_result: None,
                    author_agent_id: None,
                    author_agent_name: None,
                    reasoning: None,
                    message_kind: AgentMessageKind::Normal,
                    compaction_strategy: None,
                    compaction_payload: None,
                    offloaded_payload_id: None,
                    structural_refs: Vec::new(),
                    timestamp: now,
                });
                thread.updated_at = now;
            }
        }
        self.persist_thread_by_id(thread_id).await;
    }

    pub(crate) async fn reinforce_meta_cognitive_bias_occurrence(&self, bias_name: &str) {
        let maybe_model = {
            let mut model = self.meta_cognitive_self_model.write().await;
            if let Some(bias) = model.biases.iter_mut().find(|bias| bias.name == bias_name) {
                bias.occurrence_count = bias.occurrence_count.saturating_add(1);
                model.last_updated_ms = now_millis();
                Some(model.clone())
            } else {
                None
            }
        };

        if let Some(model) = maybe_model {
            if let Err(error) = self.persist_meta_cognitive_self_model(&model).await {
                tracing::warn!(bias_name = %bias_name, "failed to persist meta-cognitive bias reinforcement: {error}");
            }
        }
    }

    pub(crate) async fn apply_meta_cognitive_calibration_adjustment(
        &self,
        adjustment: f64,
        predicted_band: ConfidenceBand,
    ) {
        let now = now_millis();
        let model_to_persist = {
            let mut model = self.meta_cognitive_self_model.write().await;
            model.calibration_offset = (model.calibration_offset + adjustment).clamp(-0.35, 0.35);
            model.last_updated_ms = now;
            model.clone()
        };
        let predicted_success = predicted_band != ConfidenceBand::Guessing;
        self.calibration_tracker.write().await.record_observation(
            predicted_band,
            predicted_success,
            now,
        );

        if let Err(error) = self
            .persist_meta_cognitive_self_model(&model_to_persist)
            .await
        {
            tracing::warn!(
                adjustment,
                "failed to persist meta-cognitive calibration update: {error}"
            );
        }
    }
}

fn render_warning(signal: &BiasSignal) -> String {
    format!("{} — {}", signal.bias_name, signal.rationale)
}

fn joined_warnings(signals: &[BiasSignal]) -> String {
    signals
        .iter()
        .map(|signal| signal.bias_name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn build_reflection_message(
    tool_call: &ToolCall,
    signals: &[BiasSignal],
    hard_block: bool,
) -> String {
    let header = if hard_block {
        "Meta-cognitive intervention: tool call blocked before execution."
    } else {
        "Meta-cognitive intervention: warning before tool execution."
    };
    let warnings = signals
        .iter()
        .map(|signal| format!("- {}: {}", signal.bias_name, signal.mitigation_prompt))
        .collect::<Vec<_>>()
        .join("\n");
    let action = if hard_block {
        "Do not repeat the same action immediately. First reflect on why this approach may be failing, inspect fresh state, and choose a materially different next step."
    } else {
        "Before continuing, briefly reflect on whether this is the best next step and whether a different approach would reduce risk."
    };

    format!(
        "{header}\nPlanned tool: {}\nArguments: {}\nDetected risks:\n{}\n{}",
        tool_call.function.name, tool_call.function.arguments, warnings, action,
    )
}
