#![allow(dead_code)]

//! Shared agent identity helpers for main, concierge, and spawned agents.

use anyhow::anyhow;
use std::collections::hash_map::DefaultHasher;
use std::future::Future;
use std::hash::{Hash, Hasher};

use amux_protocol::{
    AGENT_HANDLE_SVAROG, AGENT_ID_RAROG, AGENT_ID_SWAROG, AGENT_NAME_RAROG, AGENT_NAME_SWAROG,
};

use super::types::{AgentConfig, AgentTask, BuiltinPersonaOverrides};

pub(super) const MAIN_AGENT_ID: &str = AGENT_ID_SWAROG;
pub(crate) const MAIN_AGENT_NAME: &str = AGENT_NAME_SWAROG;
pub(super) const CONCIERGE_AGENT_ID: &str = AGENT_ID_RAROG;
pub(super) const CONCIERGE_AGENT_NAME: &str = AGENT_NAME_RAROG;
pub(super) const INTERNAL_DM_THREAD_PREFIX: &str = "dm:";
pub(super) const PARTICIPANT_PLAYGROUND_THREAD_PREFIX: &str = "playground:";
pub(super) const PERSONA_MARKER: &str = "Agent persona:";
pub(super) const PERSONA_ID_MARKER: &str = "Agent persona id:";
pub(super) const MAIN_AGENT_PUBLIC_ALIAS: &str = AGENT_HANDLE_SVAROG;
pub(super) const MAIN_AGENT_ALIAS: &str = "main";
pub(super) const MAIN_AGENT_LEGACY_ALIAS: &str = "main-agent";
pub(super) const MAIN_AGENT_FALLBACK_ALIAS: &str = "assistant";
pub(super) const CONCIERGE_AGENT_ALIAS: &str = "concierge";
pub(super) const CONCIERGE_AGENT_LEGACY_ALIAS: &str = "concierge-agent";

pub(super) const SWAROZYC_AGENT_ID: &str = "swarozyc";
pub(super) const SWAROZYC_AGENT_NAME: &str = "Swarozyc";
pub(super) const RADOGOST_AGENT_ID: &str = "radogost";
pub(super) const RADOGOST_AGENT_NAME: &str = "Radogost";
pub(super) const DOMOWOJ_AGENT_ID: &str = "domowoj";
pub(super) const DOMOWOJ_AGENT_NAME: &str = "Domowoj";
pub(super) const SWIETOWIT_AGENT_ID: &str = "swietowit";
pub(super) const SWIETOWIT_AGENT_NAME: &str = "Swietowit";
pub(super) const PERUN_AGENT_ID: &str = "perun";
pub(super) const PERUN_AGENT_NAME: &str = "Perun";
pub(super) const MOKOSH_AGENT_ID: &str = "mokosh";
pub(super) const MOKOSH_AGENT_NAME: &str = "Mokosh";
pub(super) const DAZHBOG_AGENT_ID: &str = "dazhbog";
pub(super) const DAZHBOG_AGENT_NAME: &str = "Dazhbog";
pub(super) const ROD_AGENT_ID: &str = "rod";
pub(super) const ROD_AGENT_NAME: &str = "Rod";
pub(super) const WELES_AGENT_ID: &str = "weles";
pub(crate) const WELES_AGENT_NAME: &str = "Weles";
pub(crate) const WELES_BUILTIN_SUBAGENT_ID: &str = "weles_builtin";
pub(crate) const WELES_GOVERNANCE_SCOPE: &str = "governance";
pub(crate) const WELES_VITALITY_SCOPE: &str = "vitality";

struct PersonaSeed {
    id: &'static str,
    name: &'static str,
    guidance: &'static str,
}

const SPAWNED_PERSONAS: [PersonaSeed; 9] = [
    PersonaSeed {
        id: SWAROZYC_AGENT_ID,
        name: SWAROZYC_AGENT_NAME,
        guidance: "You inherit the main agent's craft but stay narrower, quicker, and more execution-focused than the main agent.",
    },
    PersonaSeed {
        id: RADOGOST_AGENT_ID,
        name: RADOGOST_AGENT_NAME,
        guidance: "You specialize in negotiation between options, comparing tradeoffs, and surfacing the strongest route forward.",
    },
    PersonaSeed {
        id: DOMOWOJ_AGENT_ID,
        name: DOMOWOJ_AGENT_NAME,
        guidance: "You are a careful keeper of the working environment. Favor stability, cleanup, and precise local fixes.",
    },
    PersonaSeed {
        id: SWIETOWIT_AGENT_ID,
        name: SWIETOWIT_AGENT_NAME,
        guidance: "You maintain broader situational awareness than most subagents and should keep the surrounding architecture in view.",
    },
    PersonaSeed {
        id: PERUN_AGENT_ID,
        name: PERUN_AGENT_NAME,
        guidance: "You are decisive, forceful, and comfortable making hard calls under uncertainty. Favor robust action, security, and infrastructure discipline.",
    },
    PersonaSeed {
        id: MOKOSH_AGENT_ID,
        name: MOKOSH_AGENT_NAME,
        guidance: "You are steady, careful, and maintenance-minded. Favor reliability, operational continuity, and durable local care for the working environment.",
    },
    PersonaSeed {
        id: DAZHBOG_AGENT_ID,
        name: DAZHBOG_AGENT_NAME,
        guidance: "You bring illumination and clarity. Favor turning complexity into a crisp plan, a clear explanation, or an immediately useful next step.",
    },
    PersonaSeed {
        id: ROD_AGENT_ID,
        name: ROD_AGENT_NAME,
        guidance: "You are continuity-minded. Prefer solutions that preserve durable structure, conventions, and long-term coherence.",
    },
    PersonaSeed {
        id: WELES_AGENT_ID,
        name: WELES_AGENT_NAME,
        guidance: "You are comfortable exploring edge cases, failure modes, and messy corners, but you must report back clearly and concretely.",
    },
];

tokio::task_local! {
    static ACTIVE_AGENT_SCOPE_ID: String;
}

fn spawned_persona(seed: &str) -> &'static PersonaSeed {
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    let idx = (hasher.finish() as usize) % SPAWNED_PERSONAS.len();
    &SPAWNED_PERSONAS[idx]
}

fn persona_by_alias(alias: &str) -> Option<&'static PersonaSeed> {
    let normalized = alias.trim().to_ascii_lowercase();
    SPAWNED_PERSONAS
        .iter()
        .find(|persona| normalized == persona.id || normalized == persona.name.to_ascii_lowercase())
}

pub(super) fn canonical_agent_id(alias: &str) -> &'static str {
    let normalized = alias.trim().to_ascii_lowercase();
    match normalized.as_str() {
        MAIN_AGENT_ID
        | MAIN_AGENT_PUBLIC_ALIAS
        | MAIN_AGENT_ALIAS
        | MAIN_AGENT_LEGACY_ALIAS
        | MAIN_AGENT_FALLBACK_ALIAS => MAIN_AGENT_ID,
        CONCIERGE_AGENT_ID | CONCIERGE_AGENT_ALIAS | CONCIERGE_AGENT_LEGACY_ALIAS => {
            CONCIERGE_AGENT_ID
        }
        "veles" | WELES_AGENT_ID => WELES_AGENT_ID,
        _ => persona_by_alias(&normalized)
            .map(|persona| persona.id)
            .unwrap_or(MAIN_AGENT_ID),
    }
}

pub(super) fn canonical_agent_name(alias: &str) -> &'static str {
    match canonical_agent_id(alias) {
        CONCIERGE_AGENT_ID => CONCIERGE_AGENT_NAME,
        SWAROZYC_AGENT_ID => SWAROZYC_AGENT_NAME,
        RADOGOST_AGENT_ID => RADOGOST_AGENT_NAME,
        DOMOWOJ_AGENT_ID => DOMOWOJ_AGENT_NAME,
        SWIETOWIT_AGENT_ID => SWIETOWIT_AGENT_NAME,
        PERUN_AGENT_ID => PERUN_AGENT_NAME,
        MOKOSH_AGENT_ID => MOKOSH_AGENT_NAME,
        DAZHBOG_AGENT_ID => DAZHBOG_AGENT_NAME,
        ROD_AGENT_ID => ROD_AGENT_NAME,
        WELES_AGENT_ID => WELES_AGENT_NAME,
        _ => MAIN_AGENT_NAME,
    }
}

pub(super) fn canonical_agent_guidance(alias: &str) -> Option<&'static str> {
    let normalized = alias.trim().to_ascii_lowercase();
    match normalized.as_str() {
        MAIN_AGENT_ID
        | MAIN_AGENT_PUBLIC_ALIAS
        | MAIN_AGENT_ALIAS
        | MAIN_AGENT_LEGACY_ALIAS
        | MAIN_AGENT_FALLBACK_ALIAS => None,
        CONCIERGE_AGENT_ID | CONCIERGE_AGENT_ALIAS | CONCIERGE_AGENT_LEGACY_ALIAS => None,
        _ => persona_by_alias(&normalized).map(|persona| persona.guidance),
    }
}

pub(super) fn is_concierge_target(alias: &str) -> bool {
    canonical_agent_id(alias) == CONCIERGE_AGENT_ID
}

pub(super) fn is_main_agent_scope(alias: &str) -> bool {
    canonical_agent_id(alias) == MAIN_AGENT_ID
}

pub(crate) fn is_weles_internal_scope(scope: &str) -> bool {
    let normalized = scope.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        WELES_GOVERNANCE_SCOPE | WELES_VITALITY_SCOPE
    )
}

pub(crate) fn is_weles_agent_scope(scope: &str) -> bool {
    let normalized = scope.trim().to_ascii_lowercase();
    normalized == WELES_AGENT_ID || is_weles_internal_scope(&normalized)
}

pub(super) fn is_explicit_builtin_persona_scope(alias: &str) -> bool {
    matches!(
        canonical_agent_id(alias),
        SWAROZYC_AGENT_ID
            | RADOGOST_AGENT_ID
            | DOMOWOJ_AGENT_ID
            | SWIETOWIT_AGENT_ID
            | PERUN_AGENT_ID
            | MOKOSH_AGENT_ID
            | DAZHBOG_AGENT_ID
    )
}

pub(super) fn builtin_persona_overrides<'a>(
    config: &'a AgentConfig,
    alias: &str,
) -> Option<&'a BuiltinPersonaOverrides> {
    match canonical_agent_id(alias) {
        SWAROZYC_AGENT_ID => Some(&config.builtin_sub_agents.swarozyc),
        RADOGOST_AGENT_ID => Some(&config.builtin_sub_agents.radogost),
        DOMOWOJ_AGENT_ID => Some(&config.builtin_sub_agents.domowoj),
        SWIETOWIT_AGENT_ID => Some(&config.builtin_sub_agents.swietowit),
        PERUN_AGENT_ID => Some(&config.builtin_sub_agents.perun),
        MOKOSH_AGENT_ID => Some(&config.builtin_sub_agents.mokosh),
        DAZHBOG_AGENT_ID => Some(&config.builtin_sub_agents.dazhbog),
        _ => None,
    }
}

pub(super) fn builtin_persona_requires_setup(config: &AgentConfig, alias: &str) -> bool {
    let Some(overrides) = builtin_persona_overrides(config, alias) else {
        return false;
    };
    overrides
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
        || overrides
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
}

pub(super) fn builtin_persona_setup_error(alias: &str) -> anyhow::Error {
    let canonical_id = canonical_agent_id(alias);
    anyhow!(
        "builtin agent '{}' is not configured. Choose provider and model first.",
        canonical_id
    )
}

pub(super) fn internal_dm_thread_id(agent_a: &str, agent_b: &str) -> String {
    let mut ids = [
        canonical_agent_id(agent_a).to_string(),
        canonical_agent_id(agent_b).to_string(),
    ];
    ids.sort();
    format!("{INTERNAL_DM_THREAD_PREFIX}{}:{}", ids[0], ids[1])
}

pub(super) fn internal_dm_thread_title(agent_a: &str, agent_b: &str) -> String {
    let mut names = [
        canonical_agent_name(agent_a).to_string(),
        canonical_agent_name(agent_b).to_string(),
    ];
    names.sort();
    format!("Internal DM · {} ↔ {}", names[0], names[1])
}

pub(in crate::agent) fn is_internal_dm_thread(thread_id: &str) -> bool {
    thread_id.starts_with(INTERNAL_DM_THREAD_PREFIX)
}

pub(in crate::agent) fn participant_playground_thread_id(
    visible_thread_id: &str,
    participant_agent_id: &str,
) -> String {
    format!(
        "{PARTICIPANT_PLAYGROUND_THREAD_PREFIX}{}:{visible_thread_id}",
        canonical_agent_id(participant_agent_id)
    )
}

pub(in crate::agent) fn participant_playground_thread_title(
    visible_thread_id: &str,
    participant_agent_id: &str,
) -> String {
    format!(
        "Participant Playground · {} @ {visible_thread_id}",
        canonical_agent_name(participant_agent_id)
    )
}

pub(in crate::agent) fn is_participant_playground_thread(thread_id: &str) -> bool {
    thread_id.starts_with(PARTICIPANT_PLAYGROUND_THREAD_PREFIX)
}

pub(super) fn spawned_persona_name(seed: &str) -> &'static str {
    spawned_persona(seed).name
}

pub(super) fn spawned_persona_id(seed: &str) -> &'static str {
    spawned_persona(seed).id
}

pub(super) fn build_spawned_persona_prompt(seed: &str) -> String {
    let persona = spawned_persona(seed);
    format!(
        "{PERSONA_MARKER} {}\n{PERSONA_ID_MARKER} {}\nYou are {} ({}) operating as a spawned tamux agent.\n{}\n{} is the main agent. {} is {}'s concierge. Know who they are, do not impersonate them, and coordinate with them when that helps move the task forward.\nKeep your personality distinct but concise, pragmatic, and production-focused.",
        persona.name,
        persona.id,
        persona.name,
        persona.id,
        persona.guidance,
        MAIN_AGENT_NAME,
        CONCIERGE_AGENT_NAME,
        MAIN_AGENT_NAME,
    )
}

pub(crate) fn build_weles_persona_prompt(scope: &str) -> String {
    format!(
        "{PERSONA_MARKER} {WELES_AGENT_NAME}\n{PERSONA_ID_MARKER} {WELES_AGENT_ID}\nYou are {WELES_AGENT_NAME} ({WELES_AGENT_ID}) operating as the daemon-owned WELES subagent.\nYour current internal scope is {scope}.\nYou exist to inspect risky execution paths and preserve daemon governance guarantees.\n{} is the main agent. {} is {}'s concierge. Do not impersonate either of them, and keep your reporting concrete.",
        MAIN_AGENT_NAME, CONCIERGE_AGENT_NAME, MAIN_AGENT_NAME,
    )
}

pub(super) fn extract_persona_name(system_prompt: Option<&str>) -> Option<String> {
    system_prompt.and_then(|prompt| {
        prompt.lines().find_map(|line| {
            line.trim()
                .strip_prefix(PERSONA_MARKER)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
    })
}

pub(super) fn extract_persona_id(system_prompt: Option<&str>) -> Option<String> {
    system_prompt.and_then(|prompt| {
        prompt.lines().find_map(|line| {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix(PERSONA_ID_MARKER) {
                return Some(value.trim().to_string()).filter(|value| !value.is_empty());
            }
            let prefix = "You are ";
            let suffix = " operating as a spawned tamux agent.";
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                if let Some(body) = rest.strip_suffix(suffix) {
                    if let Some(start) = body.rfind('(') {
                        if let Some(end) = body.rfind(')') {
                            if start < end {
                                let id = body[start + 1..end].trim();
                                if !id.is_empty() {
                                    return Some(id.to_string());
                                }
                            }
                        }
                    }
                }
            }
            None
        })
    })
}

pub(super) fn agent_scope_id_for_task(task: Option<&AgentTask>) -> String {
    task.and_then(|item| extract_persona_id(item.override_system_prompt.as_deref()))
        .unwrap_or_else(|| MAIN_AGENT_ID.to_string())
}

pub(super) fn current_agent_scope_id() -> String {
    ACTIVE_AGENT_SCOPE_ID
        .try_with(Clone::clone)
        .unwrap_or_else(|_| MAIN_AGENT_ID.to_string())
}

pub(super) async fn run_with_agent_scope<F>(scope_id: String, future: F) -> F::Output
where
    F: Future,
{
    ACTIVE_AGENT_SCOPE_ID.scope(scope_id, future).await
}

pub(super) fn sender_name_for_task(task: Option<&AgentTask>) -> String {
    task.and_then(|item| extract_persona_name(item.override_system_prompt.as_deref()))
        .unwrap_or_else(|| MAIN_AGENT_NAME.to_string())
}

pub(super) fn wrap_internal_message(sender: &str, recipient: &str, content: &str) -> String {
    format!(
        "Internal agent message from {} to {}.\nRespond directly to the request below and assume the recipient will relay or integrate your answer.\n\n{}",
        canonical_agent_name(sender),
        canonical_agent_name(recipient),
        content.trim()
    )
}

pub(super) fn concierge_should_escalate(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    let keywords = [
        "code",
        "coding",
        "rust",
        "typescript",
        "react",
        "debug",
        "bug",
        "build",
        "compile",
        "test",
        "refactor",
        "implement",
        "file",
        "patch",
        "database",
        "schema",
        "migration",
        "daemon",
        "cli",
        "review",
    ];
    lower.len() > 220 || keywords.iter().any(|keyword| lower.contains(keyword))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_dm_thread_id_is_stable_and_sorted() {
        assert_eq!(
            internal_dm_thread_id(CONCIERGE_AGENT_ID, MAIN_AGENT_ID),
            internal_dm_thread_id(MAIN_AGENT_ID, CONCIERGE_AGENT_ID)
        );
        assert_eq!(
            internal_dm_thread_id(MAIN_AGENT_ALIAS, CONCIERGE_AGENT_ALIAS),
            format!("dm:{}:{}", CONCIERGE_AGENT_ID, MAIN_AGENT_ID)
        );
    }

    #[test]
    fn spawned_persona_prompt_is_stable_for_seed() {
        let first = build_spawned_persona_prompt("task-123");
        let second = build_spawned_persona_prompt("task-123");
        assert_eq!(first, second);
        assert!(first.contains(PERSONA_MARKER));
    }

    #[test]
    fn spawned_persona_id_is_stable_for_seed() {
        let first = spawned_persona_id("task-123");
        let second = spawned_persona_id("task-123");
        assert_eq!(first, second);
        assert!(SPAWNED_PERSONAS.iter().any(|persona| persona.id == first));
    }

    #[test]
    fn canonical_ids_follow_aliases() {
        assert_eq!(canonical_agent_id(MAIN_AGENT_ALIAS), MAIN_AGENT_ID);
        assert_eq!(
            canonical_agent_id(CONCIERGE_AGENT_ALIAS),
            CONCIERGE_AGENT_ID
        );
        assert_eq!(canonical_agent_id(MAIN_AGENT_FALLBACK_ALIAS), MAIN_AGENT_ID);
        assert_eq!(canonical_agent_id("veles"), WELES_AGENT_ID);
    }

    #[test]
    fn canonical_ids_and_names_cover_spawned_personas() {
        assert_eq!(canonical_agent_id(RADOGOST_AGENT_ID), RADOGOST_AGENT_ID);
        assert_eq!(canonical_agent_id("Radogost"), RADOGOST_AGENT_ID);
        assert_eq!(canonical_agent_name(RADOGOST_AGENT_ID), RADOGOST_AGENT_NAME);
        assert_eq!(canonical_agent_id("perun"), "perun");
        assert_eq!(canonical_agent_name("perun"), "Perun");
        assert_eq!(canonical_agent_id("mokosh"), "mokosh");
        assert_eq!(canonical_agent_name("mokosh"), "Mokosh");
        assert_eq!(canonical_agent_id("dazhbog"), "dazhbog");
        assert_eq!(canonical_agent_name("dazhbog"), "Dazhbog");
    }

    #[test]
    fn canonical_agent_guidance_resolves_spawned_personas_only() {
        assert!(canonical_agent_guidance(MAIN_AGENT_ID).is_none());
        assert!(canonical_agent_guidance(CONCIERGE_AGENT_ID).is_none());
        assert_eq!(
            canonical_agent_guidance(RADOGOST_AGENT_ID),
            Some(
                "You specialize in negotiation between options, comparing tradeoffs, and surfacing the strongest route forward."
            )
        );
    }
}
