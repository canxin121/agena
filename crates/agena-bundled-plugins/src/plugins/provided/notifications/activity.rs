//! Read-only terminal activity projection. Each runtime snapshot owns a worker,
//! so a newly published host is usable while a retired host still owns a run.
//! Attention history has a separate single-writer lease; this projection neither
//! waits for that lease nor emits/replays attention notifications.

use std::{sync::Arc, time::Duration};

use agena_plugin_host::sdk::host_api::HostClient;
use agena_storage::kernel::{ChangeKind, KernelReader, SessionActivitySummary, WatchError};
use anyhow::Result;
use tokio::task::JoinHandle;

use super::{TerminalActivity, publish_activity};

fn activity(summary: &SessionActivitySummary) -> TerminalActivity {
    if summary.running_sessions > 0 {
        TerminalActivity::Running
    } else if summary.awaiting_interaction_sessions > 0 || summary.latest_terminal_failed {
        TerminalActivity::Blocked
    } else {
        TerminalActivity::Idle
    }
}

struct Projector {
    reader: KernelReader,
    host: Arc<dyn HostClient>,
    published: Option<TerminalActivity>,
}

impl Projector {
    async fn refresh(&mut self) -> Result<()> {
        // Never turn a failed read into Idle. Keep the last published value and
        // let the worker retry; the source is one coherent kernel read snapshot.
        let current = activity(&*self.reader.activity_summary().await?);
        if self.published != Some(current) {
            publish_activity(self.host.as_ref(), current)
                .await
                .map_err(|error| anyhow::anyhow!(error.diagnostic.message))?;
            self.published = Some(current);
        }
        Ok(())
    }
}

pub(super) async fn start(
    reader: KernelReader,
    host: Arc<dyn HostClient>,
) -> Result<JoinHandle<()>> {
    // Subscribe first, including on reset, so a commit between initial scanning
    // and starting the task cannot leave a new display registry stale.
    let mut watch = reader.watch_all(None)?;
    let mut projector = Projector {
        reader,
        host,
        published: None,
    };
    let mut retry_needed = match projector.refresh().await {
        Ok(()) => false,
        Err(error) => {
            tracing::warn!(%error, "terminal activity startup publication deferred");
            true
        }
    };
    Ok(tokio::spawn(async move {
        let mut retry = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(1),
            Duration::from_secs(1),
        );
        retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = retry.tick(), if retry_needed => {}
                change = watch.recv() => match change {
                    Ok(change) if matches!(
                        change.kind,
                        ChangeKind::Execution
                            | ChangeKind::Interaction
                            | ChangeKind::Membership
                            | ChangeKind::Metadata
                    ) => {
                        // Do not discard include_in_lists=false membership
                        // events: deleting/hiding the last running session
                        // changes the aggregate. The reader owns visibility.
                    }
                    Ok(_) => continue,
                    Err(WatchError::Closed) => break,
                    Err(WatchError::Reset { .. }) => match projector.reader.watch_all(None) {
                        Ok(next) => watch = next,
                        Err(error) => {
                            tracing::error!(%error, "terminal activity observer stopped");
                            break;
                        }
                    },
                }
            }
            retry_needed = match projector.refresh().await {
                Ok(()) => false,
                Err(error) => {
                    tracing::warn!(%error, "terminal activity publication deferred");
                    true
                }
            };
        }
    }))
}
