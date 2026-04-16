#![allow(dead_code)]

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use amux_protocol::{
    ClientMessage, DaemonMessage, GatewayBootstrapPayload, GatewayConnectionStatus,
    GatewayContinuityState, GatewayCursorState, GatewayHealthState, GatewayIncomingEvent,
    GatewayProviderBootstrap, GatewayRegistration, GatewayRouteMode, GatewayRouteModeState,
    GatewayThreadBindingState, SessionInfo, GATEWAY_IPC_PROTOCOL_VERSION,
};
use anyhow::{Context, Result};
use futures::SinkExt;
use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, mpsc};
use tokio_util::codec::Framed;

use crate::agent::skill_community::{
    export_skill, import_community_skill, prepare_publish, unpack_skill, ImportResult,
};
use crate::agent::skill_registry::{to_community_entry, RegistryClient};
use crate::agent::AgentEngine;
use crate::session_manager::SessionManager;

include!("server/helpers_part1.rs");
include!("server/helpers_part2.rs");
include!("server/operations.rs");
include!("server/subsystem_queue.rs");
include!("server/subsystem_metrics.rs");
include!("server/operation_retention.rs");
include!("server/operations_registry.rs");

#[cfg(test)]
mod tests {
    include!("server/tests_part1.rs");
    include!("server/tests_part2.rs");
    include!("server/tests_part3.rs");
}

include!("server/post_tests.rs");

#[derive(Clone)]
struct StartupReadiness {
    ready: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}

impl StartupReadiness {
    fn new(initial_ready: bool) -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(initial_ready)),
            notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    async fn wait_until_ready(&self) {
        if self.ready.load(Ordering::Acquire) {
            return;
        }

        loop {
            let notified = self.notify.notified();
            if self.ready.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

fn client_message_requires_startup_readiness(msg: &ClientMessage) -> bool {
    !matches!(
        msg,
        ClientMessage::Ping
            | ClientMessage::AgentGetConfig
            | ClientMessage::AgentGetEffectiveConfigState
            | ClientMessage::GatewayRegister { .. }
            | ClientMessage::GatewayAck { .. }
            | ClientMessage::GatewayIncomingEvent { .. }
            | ClientMessage::GatewayCursorUpdate { .. }
            | ClientMessage::GatewayThreadBindingUpdate { .. }
            | ClientMessage::GatewayRouteModeUpdate { .. }
            | ClientMessage::GatewaySendResult { .. }
            | ClientMessage::GatewayHealthUpdate { .. }
            | ClientMessage::SpawnSession { .. }
            | ClientMessage::CloneSession { .. }
            | ClientMessage::AttachSession { .. }
            | ClientMessage::DetachSession { .. }
            | ClientMessage::AgentSubscribe
            | ClientMessage::AgentUnsubscribe
            | ClientMessage::AgentDeclareAsyncCommandCapability { .. }
            | ClientMessage::AgentGetOperationStatus { .. }
            | ClientMessage::AgentGetHealthStatus
    )
}

#[cfg(test)]
mod startup_readiness_tests {
    use super::*;

    #[test]
    fn config_requests_are_not_blocked_on_startup_readiness() {
        assert!(!client_message_requires_startup_readiness(
            &ClientMessage::AgentGetConfig
        ));
        assert!(!client_message_requires_startup_readiness(
            &ClientMessage::AgentGetEffectiveConfigState
        ));
    }

    #[tokio::test]
    async fn cloned_startup_readiness_observes_mark_ready() {
        let readiness = StartupReadiness::new(false);
        let cloned = readiness.clone();

        readiness.mark_ready();

        tokio::time::timeout(std::time::Duration::from_millis(50), cloned.wait_until_ready())
            .await
            .expect("cloned startup readiness should observe mark_ready");
    }
}

async fn handle_connection<S>(
    stream: S,
    manager: Arc<SessionManager>,
    agent: Arc<AgentEngine>,
    plugin_manager: Arc<crate::plugin::PluginManager>,
    startup_readiness: StartupReadiness,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    use amux_protocol::DaemonCodec;
    let mut framed = Framed::new(stream, DaemonCodec);

    let mut attached_rxs: Vec<(amux_protocol::SessionId, broadcast::Receiver<DaemonMessage>)> =
        Vec::new();
    let mut client_agent_threads: HashSet<String> = HashSet::new();
    let mut last_concierge_welcome_fingerprint: Option<String> = None;
    let mut agent_event_rx: Option<broadcast::Receiver<crate::agent::types::AgentEvent>> = None;
    let mut background_daemon_queues = BackgroundSubsystemQueues::new();
    let mut background_daemon_pending = BackgroundPendingCounts::default();
    let mut whatsapp_link_rx: Option<
        broadcast::Receiver<crate::agent::types::WhatsAppLinkRuntimeEvent>,
    > = None;
    let mut gateway_ipc_rx: Option<mpsc::UnboundedReceiver<DaemonMessage>> = None;
    let mut whatsapp_link_subscriber_guard = WhatsAppLinkSubscriberGuard::new(agent.clone());
    let mut whatsapp_link_snapshot_replayed = false;
    let mut gateway_connection_state = GatewayConnectionState::Unregistered;

    loop {
        let msg = include!("server/connection_pre_dispatch.rs");

        if let Some(msg) = msg {
            if client_message_requires_startup_readiness(&msg) {
                startup_readiness.wait_until_ready().await;
            }
            include!("server/dispatch_part1.rs");
            include!("server/dispatch_part2.rs");
            include!("server/dispatch_part3.rs");
            include!("server/dispatch_part4.rs");
            include!("server/dispatch_part5.rs");
            include!("server/dispatch_part6.rs");
            include!("server/dispatch_part7.rs");
            include!("server/dispatch_part8.rs");
            include!("server/dispatch_part9.rs");
            include!("server/dispatch_part10.rs");
        }
    }
}
