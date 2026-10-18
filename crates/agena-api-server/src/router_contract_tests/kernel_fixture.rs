//! Real new-schema kernel fixtures used by transport contract tests.
use super::*;
use agena_storage::kernel::*;

pub(super) async fn attempt(
    application: &Application,
    session_id: i64,
    text: &str,
) -> (ExecutionView, AttemptHandle, RoundHandle) {
    let kernel = application.session_kernel().unwrap();
    let session = session_id.try_into().unwrap();
    let sent = kernel
        .sessions
        .send(
            RequestId::new(),
            session,
            agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                text: text.into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = kernel
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_round(RequestId::new(), vec![sent.message_id])
        .await
        .unwrap()
        .value;
    (sent, attempt, round)
}

pub(super) async fn completed_input(
    application: &Application,
    session_id: i64,
    text: &str,
) -> ExecutionView {
    let (sent, attempt, round) = attempt(application, session_id, text).await;
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    sent
}

pub(super) async fn entries(application: &Application, session_id: i64) -> Vec<EntryView> {
    let kernel = application.session_kernel().unwrap();
    let mut after = None;
    let mut entries = Vec::new();
    loop {
        let page = kernel
            .reader
            .history_after(session_id.try_into().unwrap(), after, 128)
            .await
            .unwrap();
        entries.extend(page.entries.clone());
        if !page.has_more {
            break;
        }
        after = page.entries.last().map(|entry| entry.id);
    }
    entries
}

pub(super) async fn tool(
    application: &Application,
    session_id: i64,
    name: &str,
    input: serde_json::Value,
) -> (AttemptHandle, RoundHandle, ToolHandle, EntryId) {
    let (_, attempt, round) = attempt(application, session_id, "transport fixture").await;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: format!("fixture-{name}"),
                name: name.into(),
                input,
            },
        )
        .await
        .unwrap()
        .value;
    let page = application
        .session_kernel()
        .unwrap()
        .reader
        .history(session_id.try_into().unwrap(), None, 1)
        .await
        .unwrap();
    let entry = page.entries[0].id;
    (attempt, round, tool, entry)
}
