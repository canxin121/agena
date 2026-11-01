//! Host clipboard calls never run on the terminal event-loop thread. Some OS
//! clipboard APIs have no cancellation primitive, so use a bounded dedicated
//! worker rather than a Tokio blocking task that could hold runtime shutdown.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use agena_tui_platform::attachment_source::{
    AttachmentAcquisition, AttachmentSource, ClipboardImageSource,
};
use agena_tui_platform::clipboard::{
    ClipboardCopyMethod, clipboard_file_list, get_clipboard_text, set_clipboard_text,
};
use agena_tui_platform::terminal::TerminalContext;
use tokio::sync::{Semaphore, oneshot};

use crate::{App, AppMessage, UiAction};

static CLIPBOARD_GATE: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));
const CLIPBOARD_DEADLINE: Duration = Duration::from_secs(12);

#[derive(Debug)]
pub(crate) struct ClipboardSnapshot {
    pub(crate) files: Option<Result<Vec<std::path::PathBuf>, String>>,
    pub(crate) image: Option<Result<AttachmentAcquisition, String>>,
    pub(crate) text: Option<Result<String, String>>,
}

impl ClipboardSnapshot {
    pub(crate) fn discard_image(acquisition: AttachmentAcquisition) {
        drop(Self {
            files: None,
            text: None,
            image: Some(Ok(acquisition)),
        });
    }

    fn read(context: &TerminalContext) -> Self {
        if !context.capabilities.clipboard_read_native.is_operational() {
            return Self {
                files: None,
                image: None,
                text: None,
            };
        }
        // Keep one excess item to report truncation. Metadata checks and PNG
        // encoding happen here, never on the UI thread.
        let files = clipboard_file_list()
            .map(|paths| {
                paths
                    .into_iter()
                    .filter(|path| path.is_file())
                    .take(9)
                    .collect::<Vec<_>>()
            })
            .map_err(|error| error.to_string());
        if files.as_ref().is_ok_and(|paths| !paths.is_empty()) {
            return Self {
                files: Some(files),
                image: None,
                text: None,
            };
        }
        let image = ClipboardImageSource
            .acquire()
            .map_err(|error| error.to_string());
        let text = get_clipboard_text(context).map_err(|error| error.to_string());
        Self {
            files: Some(files),
            image: Some(image),
            text: Some(text),
        }
    }
}

impl Drop for ClipboardSnapshot {
    fn drop(&mut self) {
        let Some(Ok(acquisition)) = self.image.take() else {
            return;
        };
        // A timeout or a session switch discards the result, not its temporary
        // image. Drop may run on the UI thread, so cleanup is offloaded there.
        let cleanup = move || {
            for item in acquisition.items {
                if item.temporary {
                    let _ = std::fs::remove_file(item.path);
                }
            }
            if let Some(root) = acquisition.cleanup_root {
                let _ = std::fs::remove_dir_all(root);
            }
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(cleanup);
        } else {
            cleanup();
        }
    }
}

async fn host_clipboard_work<T: Send + 'static>(
    gate: Arc<Semaphore>,
    deadline: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let permit = gate.try_acquire_owned().map_err(|_| {
        "a previous clipboard request is still running; no additional worker was started".to_owned()
    })?;
    let (tx, rx) = oneshot::channel();
    std::thread::Builder::new()
        .name("agena-tui-clipboard".into())
        .spawn(move || {
            // A timed-out OS call cannot be forcibly stopped. Keep its permit
            // until it really exits, so repeated shortcuts cannot leak threads.
            let _permit = permit;
            let _ = tx.send(work());
        })
        .map_err(|error| format!("failed to start clipboard worker: {error}"))?;
    tokio::time::timeout(deadline, rx)
        .await
        .map_err(|_| "clipboard request timed out; the interface remains available".to_owned())?
        .map_err(|_| "clipboard worker exited without a result".to_owned())
}

impl App {
    pub(crate) fn paste_without_blocking(&mut self, context: TerminalContext) {
        self.reset_prompt_history_recall();
        self.focus = agena_tui::main_focus::Focus::Composer;
        let epoch = self.composer_media_epoch;
        let slot = self.current_draft_slot();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result =
                host_clipboard_work(CLIPBOARD_GATE.clone(), CLIPBOARD_DEADLINE, move || {
                    ClipboardSnapshot::read(&context)
                })
                .await;
            let _ = tx
                .send(AppMessage::AsyncOperationCompleted(Box::new(move |app| {
                    app.apply_clipboard_read(epoch, slot, result);
                })))
                .await;
        });
    }

    pub(crate) fn accepts_clipboard_read(&self, epoch: u64, slot: crate::DraftSlot) -> bool {
        self.composer_media_epoch == epoch
            && self.current_draft_slot() == slot
            && self.current_route_is_main()
            && self.focus == agena_tui::main_focus::Focus::Composer
            && self.overlay.is_none()
            && self.context_help.is_none()
            && self.interaction_editing.is_none()
            && self.prompt_history_search.is_none()
    }

    fn apply_clipboard_read(
        &mut self,
        epoch: u64,
        slot: crate::DraftSlot,
        result: Result<ClipboardSnapshot, String>,
    ) {
        if !self.accepts_clipboard_read(epoch, slot) {
            return;
        }
        match result {
            Ok(snapshot) => {
                self.pending_ui_action = Some(UiAction::PreparedClipboardPaste {
                    snapshot,
                    epoch,
                    slot,
                })
            }
            Err(error) => self.flash_warning(self.i18n.text_args(
                "flash-clipboard-paste-failed",
                &agena_tui::fl_args!("error" => error),
            )),
        }
    }

    pub(crate) fn copy_text_without_blocking(
        &mut self,
        text: String,
        success: String,
        context: TerminalContext,
    ) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result =
                host_clipboard_work(CLIPBOARD_GATE.clone(), CLIPBOARD_DEADLINE, move || {
                    let mut sequence = Vec::new();
                    let method = set_clipboard_text(&text, &context, |bytes| {
                        // Never let a worker touch stdout or terminal protocols.
                        sequence.extend_from_slice(bytes);
                        Ok(())
                    })
                    .map_err(|error| error.to_string())?;
                    Ok::<_, String>((method, sequence))
                })
                .await
                .and_then(|result| result);
            let _ = tx
                .send(AppMessage::AsyncOperationCompleted(Box::new(
                    move |app| match result {
                        Ok((ClipboardCopyMethod::Osc52, sequence)) => {
                            app.pending_ui_action =
                                Some(UiAction::WriteClipboard { sequence, success });
                        }
                        Ok((method, _)) => app.flash_clipboard_copy_success(method, success),
                        Err(error) => app.flash_error(app.i18n.text_args(
                            "flash-clipboard-copy-failed",
                            &agena_tui::fl_args!("error" => error),
                        )),
                    },
                )))
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abandoned_image_results_clean_only_owned_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let temporary = directory.path().join("temporary.png");
        let original = directory.path().join("original.png");
        std::fs::write(&temporary, b"image").unwrap();
        std::fs::write(&original, b"image").unwrap();
        ClipboardSnapshot::discard_image(AttachmentAcquisition {
            items: vec![
                agena_tui_platform::attachment_source::AcquiredAttachment {
                    path: temporary.clone(),
                    temporary: true,
                    image_info: None,
                },
                agena_tui_platform::attachment_source::AcquiredAttachment {
                    path: original.clone(),
                    temporary: false,
                    image_info: None,
                },
            ],
            cleanup_root: None,
        });
        assert!(!temporary.exists());
        assert!(original.exists());
    }

    #[tokio::test]
    async fn late_clipboard_results_cannot_enter_a_replaced_composer() {
        let directory = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(directory.path().to_path_buf());
        let mut app = App::new_with_backend(backend, Default::default(), crate::I18n::english());
        app.draft_store_path = directory.path().join("drafts.json");
        app.prompt_history_path = directory.path().join("history.jsonl");
        app.current_route = crate::Route::Main;
        app.focus = agena_tui::main_focus::Focus::Composer;
        let slot = app.current_draft_slot();
        let old_epoch = app.composer_media_epoch;
        app.clear_composer_state();
        assert_ne!(app.composer_media_epoch, old_epoch);
        app.apply_clipboard_read(
            old_epoch,
            slot,
            Ok(ClipboardSnapshot {
                files: None,
                image: None,
                text: Some(Ok("obsolete paste".into())),
            }),
        );
        assert!(app.pending_ui_action.is_none());
        assert!(app.composer.text().is_empty());
        app.focus = agena_tui::main_focus::Focus::Transcript;
        app.apply_clipboard_read(
            app.composer_media_epoch,
            slot,
            Ok(ClipboardSnapshot {
                files: None,
                image: None,
                text: Some(Ok("wrong focus".into())),
            }),
        );
        assert!(app.pending_ui_action.is_none());
        app.focus = agena_tui::main_focus::Focus::Composer;
        app.apply_clipboard_read(
            app.composer_media_epoch,
            slot,
            Ok(ClipboardSnapshot {
                files: None,
                image: None,
                text: Some(Ok("current paste".into())),
            }),
        );
        assert!(matches!(
            app.pending_ui_action,
            Some(UiAction::PreparedClipboardPaste { .. })
        ));
    }

    #[tokio::test]
    async fn timeout_keeps_the_worker_bounded_and_does_not_block_the_runtime() {
        let gate = Arc::new(Semaphore::new(1));
        let (release, hold) = std::sync::mpsc::channel();
        let (done, finished) = oneshot::channel();
        let result = host_clipboard_work(gate.clone(), Duration::from_millis(30), move || {
            // A release channel, not a long sleep, keeps failure cleanup fast.
            let _ = hold.recv_timeout(Duration::from_secs(3));
            let _ = done.send(());
        })
        .await;
        assert!(result.unwrap_err().contains("timed out"));
        assert_eq!(gate.available_permits(), 0);
        assert!(
            host_clipboard_work(gate.clone(), Duration::from_secs(1), || ())
                .await
                .unwrap_err()
                .contains("still running")
        );
        release.send(()).unwrap();
        finished.await.unwrap();
        let permit = tokio::time::timeout(Duration::from_secs(1), gate.clone().acquire_owned())
            .await
            .unwrap()
            .unwrap();
        drop(permit);
        assert_eq!(
            host_clipboard_work(gate, Duration::from_secs(1), || 42)
                .await
                .unwrap(),
            42
        );
    }
}
