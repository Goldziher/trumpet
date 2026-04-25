//! Bus-to-MCP notification bridge.
//!
//! When a Trumpet client connects over MCP, [`run_tool_notifier`] is spawned
//! to forward tool-registry changes (`Event::ToolRegistered`,
//! `Event::ToolDeregistered`) to the client as a
//! `notifications/tools/list_changed` message. Other event variants are
//! ignored.
//!
//! Bursts of registrations (e.g. an agent registering ten tools at once)
//! are coalesced into a single notification by waiting [`COALESCE_WINDOW`]
//! after the first event before flushing.

use std::sync::Arc;
use std::time::Duration;

use rmcp::Peer;
use rmcp::RoleServer;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, trace, warn};

use crate::core::bus::{Event, MessageBus};

/// How long to wait after the first tool change before flushing the notification.
///
/// 100 ms is short enough to feel instant to a human but long enough that an
/// agent registering several tools in sequence triggers a single notification.
const COALESCE_WINDOW: Duration = Duration::from_millis(100);

/// Run the tool-list notifier for the given MCP peer.
///
/// Subscribes to `bus`, filters to tool-registry events, debounces by
/// [`COALESCE_WINDOW`], and calls
/// [`Peer::notify_tool_list_changed`](rmcp::Peer::notify_tool_list_changed).
/// Returns when the peer disconnects (notification send fails) or the bus
/// is closed.
pub async fn run_tool_notifier(bus: Arc<MessageBus>, peer: Peer<RoleServer>) {
    let mut rx = bus.subscribe();

    loop {
        // Wait for the first tool-registry change.
        let first = match rx.recv().await {
            Ok(event) => event,
            Err(RecvError::Closed) => {
                debug!("bus closed; tool notifier exiting");
                return;
            }
            Err(RecvError::Lagged(n)) => {
                warn!(
                    skipped = n,
                    "tool notifier lagged behind the bus; some events may be missed"
                );
                continue;
            }
        };

        if !is_tool_event(&first) {
            continue;
        }

        // Drain any further tool events that arrive within the coalesce window.
        let deadline = tokio::time::Instant::now() + COALESCE_WINDOW;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(event)) => {
                    if is_tool_event(&event) {
                        trace!("coalescing tool event into pending notification");
                    }
                    // Non-tool events are dropped on the floor here — they will
                    // be re-delivered next iteration via a fresh subscribe is
                    // not needed because broadcast::Receiver buffers behind us.
                }
                Ok(Err(RecvError::Closed)) => {
                    // Send the pending notification, then exit.
                    let _ = send_changed(&peer).await;
                    return;
                }
                Ok(Err(RecvError::Lagged(_))) => {}
                Err(_) => break, // window elapsed
            }
        }

        if !send_changed(&peer).await {
            // Peer is gone — stop notifying.
            return;
        }
    }
}

fn is_tool_event(event: &Event) -> bool {
    matches!(event, Event::ToolRegistered(_) | Event::ToolDeregistered(_))
}

async fn send_changed(peer: &Peer<RoleServer>) -> bool {
    match peer.notify_tool_list_changed().await {
        Ok(()) => {
            trace!("sent tools/list_changed to MCP peer");
            true
        }
        Err(e) => {
            // Peer transport failure usually means the client disconnected.
            // Log at debug because it's an expected lifecycle event.
            debug!(error = %e, "MCP peer rejected tools/list_changed; stopping notifier");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{ToolId, ToolInfo, ToolProvider};

    fn dummy_tool() -> ToolInfo {
        ToolInfo {
            id: ToolId::new(),
            name: "test.tool".into(),
            description: "test".into(),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            provider: ToolProvider::BuiltIn,
        }
    }

    #[test]
    fn is_tool_event_recognises_registry_events() {
        assert!(is_tool_event(&Event::ToolRegistered(dummy_tool())));
        assert!(is_tool_event(&Event::ToolDeregistered(ToolId::new())));
    }

    #[test]
    fn is_tool_event_ignores_other_variants() {
        use crate::core::types::AgentId;
        assert!(!is_tool_event(&Event::AgentDeregistered(AgentId::new())));
    }
}
