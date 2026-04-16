#![allow(dead_code)]

use super::*;
use std::collections::HashSet;

pub(crate) const OPERATOR_MODEL_VERSION: &str = "1.0";
pub(crate) const OPERATOR_PROFILE_VERSION: &str = "1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperatorProfileInputKind {
    Boolean,
}

impl OperatorProfileInputKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OperatorProfileQuestion {
    pub id: String,
    pub field_key: String,
    pub prompt: String,
    pub input_kind: OperatorProfileInputKind,
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct OperatorProfileQuestionState {
    pub answer_json: Option<String>,
    pub skipped: bool,
    pub skip_reason: Option<String>,
    pub deferred_until_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OperatorProfileSessionState {
    pub version: String,
    pub session_id: String,
    pub kind: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub questions: Vec<OperatorProfileQuestion>,
    pub answers: HashMap<String, OperatorProfileQuestionState>,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OperatorProfileQuestionPayload {
    pub session_id: String,
    pub question_id: String,
    pub field_key: String,
    pub prompt: String,
    pub input_kind: String,
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OperatorProfileProgressPayload {
    pub session_id: String,
    pub answered: u32,
    pub remaining: u32,
    pub completion_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OperatorProfileCompletionPayload {
    pub session_id: String,
    pub updated_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct OperatorProfileSessionStarted {
    pub session_id: String,
    pub kind: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VerbosityPreference {
    Terse,
    #[default]
    Moderate,
    Verbose,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReadingDepth {
    Skim,
    #[default]
    Standard,
    Deep,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RiskTolerance {
    Conservative,
    #[default]
    Moderate,
    Aggressive,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct CognitiveStyle {
    pub avg_message_length: f64,
    pub question_frequency: f64,
    pub confirmation_seeking: f64,
    pub verbosity_preference: VerbosityPreference,
    pub reading_depth: ReadingDepth,
    pub prefers_summaries: bool,
    pub skips_reasoning: bool,
    pub message_count: u64,
    pub question_count: u64,
    pub confirmation_count: u64,
    pub summary_request_count: u64,
    pub reasoning_skip_request_count: u64,
    pub detail_request_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RiskFingerprint {
    pub approval_rate_by_category: HashMap<String, f64>,
    pub avg_response_time_secs: f64,
    pub risk_tolerance: RiskTolerance,
    pub approval_requests: u64,
    pub approvals: u64,
    pub denials: u64,
    pub category_requests: HashMap<String, u64>,
    pub category_approvals: HashMap<String, u64>,
    pub fast_denials_by_category: HashMap<String, u64>,
    pub auto_approve_categories: Vec<String>,
    pub auto_deny_categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct SessionRhythm {
    pub typical_start_hour_utc: Option<u8>,
    pub session_duration_avg_minutes: f64,
    pub peak_activity_hours_utc: Vec<u8>,
    pub session_count: u64,
    pub start_hour_histogram: HashMap<u8, u64>,
    pub activity_hour_histogram: HashMap<u8, u64>,
    pub total_observed_session_minutes: u64,
    #[serde(default)]
    pub smoothed_activity_histogram: HashMap<u8, f64>,
}

/// Exponential moving average update: `alpha * sample + (1 - alpha) * current`.
pub(crate) fn ema_update(current: f64, sample: f64, alpha: f64) -> f64 {
    alpha * sample + (1.0 - alpha) * current
}

/// Apply EMA smoothing to an activity histogram across all 24 hours.
pub(crate) fn smooth_activity_histogram(
    current: &HashMap<u8, f64>,
    observation: &HashMap<u8, u64>,
    alpha: f64,
) -> HashMap<u8, f64> {
    let mut smoothed = current.clone();
    for hour in 0..24u8 {
        let observed = observation.get(&hour).copied().unwrap_or(0) as f64;
        let old = smoothed.get(&hour).copied().unwrap_or(0.0);
        smoothed.insert(hour, ema_update(old, observed, alpha));
    }
    smoothed
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct AttentionTopology {
    pub dominant_surface: Option<String>,
    pub common_surfaces: Vec<String>,
    pub top_transitions: Vec<String>,
    pub surface_histogram: HashMap<String, u64>,
    pub transition_histogram: HashMap<String, u64>,
    pub dwell_histogram: HashMap<String, u64>,
    pub focus_event_count: u64,
    pub avg_surface_dwell_secs: f64,
    pub rapid_switch_count: u64,
    pub deep_focus_surface: Option<String>,
    pub last_surface: Option<String>,
    pub last_surface_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct ImplicitFeedback {
    pub tool_hesitation_count: u64,
    pub revision_message_count: u64,
    pub correction_message_count: u64,
    pub fast_denial_count: u64,
    #[serde(default)]
    pub rapid_revert_count: u64,
    #[serde(default)]
    pub session_abandon_count: u64,
    pub fallback_histogram: HashMap<String, u64>,
    pub top_tool_fallbacks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BehaviorAdaptationProfile {
    pub mode: SatisfactionAdaptationMode,
    pub prompt_for_clarification: bool,
    pub compact_response: bool,
    pub preferred_tool_fallbacks: Vec<String>,
}

impl BehaviorAdaptationProfile {
    pub(crate) fn from_model(model: &OperatorModel) -> Self {
        let mode = SatisfactionAdaptationMode::from_label(&model.operator_satisfaction.label);
        let prompt_for_clarification = matches!(mode, SatisfactionAdaptationMode::Minimal)
            && (model.implicit_feedback.rapid_revert_count > 0
                || model.implicit_feedback.session_abandon_count > 0
                || model.implicit_feedback.correction_message_count > 0)
            || matches!(mode, SatisfactionAdaptationMode::Tightened)
                && (model.implicit_feedback.correction_message_count > 0
                    || model.implicit_feedback.fast_denial_count > 0);
        let compact_response = !matches!(mode, SatisfactionAdaptationMode::Normal)
            || model.implicit_feedback.correction_message_count > 0
            || model.implicit_feedback.fast_denial_count > 0;
        Self {
            mode,
            prompt_for_clarification,
            compact_response,
            preferred_tool_fallbacks: preferred_tool_fallback_targets(
                &model.implicit_feedback.top_tool_fallbacks,
                3,
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SatisfactionAdaptationMode {
    Normal,
    Tightened,
    Minimal,
}

impl SatisfactionAdaptationMode {
    pub(crate) fn from_label(label: &str) -> Self {
        match label {
            "strained" => Self::Minimal,
            "fragile" => Self::Tightened,
            _ => Self::Normal,
        }
    }

    pub(crate) fn max_goal_plan_steps(self) -> usize {
        match self {
            Self::Normal => 6,
            Self::Tightened => 5,
            Self::Minimal => 4,
        }
    }

    pub(crate) fn max_goal_replan_steps(self) -> usize {
        match self {
            Self::Normal => 4,
            Self::Tightened | Self::Minimal => 3,
        }
    }

    pub(crate) fn max_rejected_alternatives(self) -> usize {
        match self {
            Self::Normal => 3,
            Self::Tightened => 2,
            Self::Minimal => 1,
        }
    }

    pub(crate) fn max_goal_replans(self, default: u32) -> u32 {
        match self {
            Self::Normal => default,
            Self::Tightened | Self::Minimal => default.min(1),
        }
    }

    pub(crate) fn max_goal_task_retries(self, default: u32) -> u32 {
        match self {
            Self::Normal => default,
            Self::Tightened | Self::Minimal => default.min(1),
        }
    }
}

pub(crate) fn preferred_tool_fallback_targets(pairs: &[String], limit: usize) -> Vec<String> {
    let mut preferred = Vec::new();
    let mut seen = HashSet::new();

    for pair in pairs {
        let Some((_, to_tool)) = pair.split_once("->") else {
            continue;
        };
        let target = to_tool.trim();
        if target.is_empty() {
            continue;
        }
        let normalized = target.to_ascii_lowercase();
        if seen.insert(normalized) {
            preferred.push(target.to_string());
        }
        if preferred.len() >= limit {
            break;
        }
    }

    preferred
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct OperatorSatisfaction {
    pub score: f64,
    pub label: String,
}

impl Default for OperatorSatisfaction {
    fn default() -> Self {
        Self {
            score: 0.5,
            label: "unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct OperatorModel {
    pub version: String,
    pub last_updated: u64,
    pub session_count: u64,
    pub cognitive_style: CognitiveStyle,
    pub risk_fingerprint: RiskFingerprint,
    pub session_rhythm: SessionRhythm,
    pub attention_topology: AttentionTopology,
    pub implicit_feedback: ImplicitFeedback,
    #[serde(default)]
    pub operator_satisfaction: OperatorSatisfaction,
    #[serde(default)]
    pub unique_tools_seen: Vec<String>,
    #[serde(default)]
    pub goal_runs_completed: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingApprovalObservation {
    pub requested_at: u64,
    pub category: String,
}

impl Default for OperatorModel {
    fn default() -> Self {
        Self {
            version: OPERATOR_MODEL_VERSION.to_string(),
            last_updated: 0,
            session_count: 0,
            cognitive_style: CognitiveStyle::default(),
            risk_fingerprint: RiskFingerprint::default(),
            session_rhythm: SessionRhythm::default(),
            attention_topology: AttentionTopology::default(),
            implicit_feedback: ImplicitFeedback::default(),
            operator_satisfaction: OperatorSatisfaction::default(),
            unique_tools_seen: Vec::new(),
            goal_runs_completed: 0,
        }
    }
}

impl OperatorModel {
    pub(crate) fn diagnostic_summary(&self) -> String {
        let signal_present = self.cognitive_style.message_count > 0
            || self.risk_fingerprint.approval_requests > 0
            || self.attention_topology.focus_event_count > 0
            || self.implicit_feedback.tool_hesitation_count > 0
            || self.implicit_feedback.revision_message_count > 0
            || self.implicit_feedback.correction_message_count > 0
            || self.implicit_feedback.fast_denial_count > 0
            || self.implicit_feedback.rapid_revert_count > 0
            || self.implicit_feedback.session_abandon_count > 0;

        let signal_state = if signal_present { "present" } else { "missing" };
        format!(
            "satisfaction={} ({:.2}) [strained <0.35, fragile <0.55, healthy <0.80, strong >=0.80]; signal {}; friction revisions {}, corrections {}, tool fallbacks {}, fast denials {}, rapid reverts {}, session abandons {}, rapid switches {}",
            self.operator_satisfaction.label,
            self.operator_satisfaction.score,
            signal_state,
            self.implicit_feedback.revision_message_count,
            self.implicit_feedback.correction_message_count,
            self.implicit_feedback.tool_hesitation_count,
            self.implicit_feedback.fast_denial_count,
            self.implicit_feedback.rapid_revert_count,
            self.implicit_feedback.session_abandon_count,
            self.attention_topology.rapid_switch_count,
        )
    }
}
