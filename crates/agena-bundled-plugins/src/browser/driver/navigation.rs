use std::collections::HashSet;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;

use super::CdpEvent;

const QUIET_WINDOW: Duration = Duration::from_millis(600);
pub(super) const SETTLE_LIMIT: Duration = Duration::from_secs(4);
pub(super) const SETTLE_EVENTS: &[&str] = &[
    "Network.requestWillBeSent",
    "Network.loadingFinished",
    "Network.loadingFailed",
    "Page.navigatedWithinDocument",
    "Page.domContentEventFired",
    "Page.javascriptDialogOpening",
];

/// Observe the requests triggered by the action through the same tool call.
/// An SPA can update its URL first and fetch data or redirect soon afterward.
pub(super) async fn settle_after_action(
    mut events: mpsc::UnboundedReceiver<CdpEvent>,
    session: &str,
    main_frame: &str,
) {
    let mut pending = HashSet::new();
    let mut quiet_until = Instant::now() + QUIET_WINDOW;
    loop {
        let event = tokio::select! {
            event = events.recv() => event,
            _ = tokio::time::sleep_until(quiet_until), if pending.is_empty() => return,
        };
        let Some(event) = event else { return };
        if event.session_id.as_deref() != Some(session) {
            continue;
        }
        let params = &event.params;
        match event.method.as_str() {
            "Network.requestWillBeSent"
                if params["frameId"].as_str() == Some(main_frame)
                    && matches!(
                        params["type"].as_str(),
                        Some("Document" | "XHR" | "Fetch" | "Script")
                    ) =>
            {
                if let Some(id) = params["requestId"].as_str() {
                    pending.insert(id.to_owned());
                    quiet_until = Instant::now() + QUIET_WINDOW;
                }
            }
            "Network.loadingFinished" | "Network.loadingFailed" => {
                if params["requestId"]
                    .as_str()
                    .is_some_and(|id| pending.remove(id))
                {
                    quiet_until = Instant::now() + QUIET_WINDOW;
                }
            }
            "Page.navigatedWithinDocument" if params["frameId"].as_str() == Some(main_frame) => {
                quiet_until = Instant::now() + QUIET_WINDOW;
            }
            "Page.domContentEventFired" => {
                quiet_until = Instant::now() + QUIET_WINDOW;
            }
            "Page.javascriptDialogOpening" => return,
            _ => {}
        }
    }
}
