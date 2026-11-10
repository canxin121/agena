//! Rewind keeps two explicit outcomes: branch (`fork`, the default) and a hard
//! in-place truncation. Both run through the production HTTP commands so the
//! wire `mode` field, the kernel membership cut, and the read-back parts are
//! all exercised.

use super::fixture;
use agena_api::commands::{Command, CommandResult, RewindMode, RewindSessionParams};
use agena_testkit::{Reply, RequestMatch, TestResult, UserSession};
use anyhow::{Context, ensure};

/// Completed turns on `session`, and the user-message identities they own.
async fn completed_turns(
    user: &UserSession,
    first: &str,
    second: &str,
) -> anyhow::Result<(i64, i64)> {
    user.send(first)
        .await?
        .wait_settled()
        .await?
        .assert_succeeded()?;
    let second_run = user.send(second).await?;
    second_run
        .wait_settled()
        .await?
        .assert_assistant_text("second-local-answer")?;
    Ok((
        user.parts()
            .await?
            .iter()
            .find(|part| part.message_boundary && part.role == "user")
            .map(|part| part.part_id)
            .context("first user message identity")?,
        second_run.receipt().message_id,
    ))
}

fn script(model: &agena_testkit::ModelHandle) {
    model
        .expect(RequestMatch::user_text("rewind-first-marker"))
        .reply(Reply::text("first-local-answer"));
    model
        .expect(RequestMatch::user_text("rewind-second-marker"))
        .reply(Reply::text("second-local-answer"));
}

async fn rewind(
    client: &agena_client::AgenaClient,
    session_id: i64,
    at_message_id: i64,
    mode: RewindMode,
) -> anyhow::Result<agena_api::resource::SessionExecutionResource> {
    let result = client
        .command(Command::RewindSession(RewindSessionParams {
            session_id,
            at_message_id,
            mode,
            expected_version: None,
        }))
        .await?;
    match result {
        CommandResult::Execution(execution) => Ok(execution),
        other => anyhow::bail!("unexpected rewind result: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_place_rewind_truncates_the_same_session_and_keeps_it_usable() -> TestResult {
    fixture()
        .run(|case| async move {
            script(&case.model());
            let user = case.user().new_session().await?;
            let (first_id, second_id) =
                completed_turns(&user, "rewind-first-marker", "rewind-second-marker").await?;
            let workspace_id = user
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("session workspace")?;

            let execution =
                rewind(user.client(), user.id(), second_id, RewindMode::InPlace).await?;

            // The very same session is returned; no branch was created.
            ensure!(
                execution.session.id == user.id(),
                "in-place rewind changed the session identity"
            );
            ensure!(
                execution.session.relation_kind == agena_domain::SessionRelationKind::Root,
                "in-place rewind turned the session into a branch"
            );
            let parts = user.parts().await?;
            ensure!(
                parts.iter().any(|part| part.part_id == first_id),
                "in-place rewind dropped history before the cut"
            );
            ensure!(
                !parts
                    .iter()
                    .any(|part| part.part_id == second_id || part.message_id == Some(second_id)),
                "in-place rewind kept the retracted turn: {parts:?}"
            );
            ensure!(
                !parts
                    .iter()
                    .any(|part| part.content.to_string().contains("second-local-answer")),
                "in-place rewind kept the retracted reply"
            );
            let overview = user
                .client()
                .session_overview(Some(workspace_id), 50)
                .await?;
            ensure!(
                overview
                    .recent
                    .iter()
                    .filter(|session| session.id == user.id())
                    .all(|session| session.message_count < 2),
                "list projection still counts the retracted turn"
            );

            // The same session must accept a new turn, and the provider must see
            // only the surviving history.
            case.model()
                .expect(RequestMatch::user_text("rewind-after-truncate"))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(
                        body.contains("rewind-first-marker"),
                        "surviving history was lost"
                    );
                    ensure!(
                        !body.contains("rewind-second-marker")
                            && !body.contains("second-local-answer"),
                        "retracted turn reached the provider"
                    );
                    Ok(())
                })
                .reply(Reply::text("truncated-session-continues"));
            let resumed = user.send("rewind-after-truncate").await?;
            ensure!(
                !user.snapshot().await?.queue_paused,
                "in-place rewind fenced the session queue"
            );
            resumed
                .wait_settled()
                .await?
                .assert_assistant_text("truncated-session-continues")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fork_rewind_creates_a_branch_and_leaves_the_source_intact() -> TestResult {
    fixture()
        .run(|case| async move {
            script(&case.model());
            let user = case.user().new_session().await?;
            let (first_id, second_id) =
                completed_turns(&user, "rewind-first-marker", "rewind-second-marker").await?;
            let source_parts = user.parts().await?;

            let execution = rewind(user.client(), user.id(), second_id, RewindMode::Fork).await?;

            let branch_id = execution.session.id;
            ensure!(
                branch_id != user.id(),
                "fork rewind reused the source session"
            );
            ensure!(
                execution.session.parent_id == Some(user.id())
                    && execution.session.relation_kind == agena_domain::SessionRelationKind::Rewind,
                "fork rewind did not record its branch relation: {:?}",
                execution.session
            );
            // The source keeps every fact, including the retracted turn.
            let source_after = user.parts().await?;
            for part in &source_parts {
                ensure!(
                    source_after
                        .iter()
                        .any(|current| current.part_id == part.part_id
                            && current.content == part.content),
                    "fork rewind rewrote the source entry {}",
                    part.part_id
                );
            }
            // The branch stops before the retracted turn.
            let branch = case.user().attach_session(branch_id).await?;
            let branch_parts = branch.parts().await?;
            ensure!(
                branch_parts.iter().any(|part| part.part_id == first_id),
                "branch is missing inherited history"
            );
            ensure!(
                !branch_parts
                    .iter()
                    .any(|part| part.part_id == second_id || part.message_id == Some(second_id)),
                "branch kept the retracted turn"
            );
            // Both sessions remain independently usable.
            case.model()
                .expect(RequestMatch::user_text("rewind-branch-followup"))
                .reply(Reply::text("branch-followup-answer"));
            branch
                .send("rewind-branch-followup")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("branch-followup-answer")?;
            ensure!(
                user.parts()
                    .await?
                    .iter()
                    .any(|part| part.part_id == second_id || part.message_id == Some(second_id)),
                "branch activity removed the source turn"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_place_rewind_settles_an_in_flight_run_instead_of_fencing_the_session() -> TestResult {
    fixture()
        .run(|case| async move {
            let first = case
                .model()
                .expect(RequestMatch::user_text("rewind-running-first"))
                .reply(Reply::text("rewind-running-answer"));
            let second = case
                .model()
                .expect(RequestMatch::user_text("rewind-running-second"))
                .reply(
                    Reply::text_chunks(["running-prefix", "unreleased-suffix"])
                        .hold_after_chunks(1),
                );
            let user = case.user().new_session().await?;
            let first_run = user.send("rewind-running-first").await?;
            first_run.wait_settled().await?.assert_succeeded()?;
            let second_run = user.send("rewind-running-second").await?;
            second_run.wait_text_contains("running-prefix").await?;
            let cut = first_run.receipt().message_id;

            let execution = rewind(user.client(), user.id(), cut, RewindMode::InPlace).await?;
            ensure!(
                execution.session.id == user.id(),
                "in-place rewind forked while a run was active"
            );
            second_run.wait_settled().await?.assert_cancelled()?;
            // The held stream must be torn down rather than left dangling.
            second.disconnected().await?;
            first.finished().await?;
            ensure!(
                !user.parts().await?.iter().any(|part| {
                    part.message_id == Some(second_run.receipt().message_id)
                        || part.content.to_string().contains("running-prefix")
                }),
                "a cancelled run's parts survived the truncation"
            );
            ensure!(
                !user.snapshot().await?.queue_paused,
                "settling the active run paused the queue"
            );
            case.model()
                .expect(RequestMatch::user_text("rewind-running-after"))
                .reply(Reply::text("post-truncation-answer"));
            user.send("rewind-running-after")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("post-truncation-answer")?;
            Ok(())
        })
        .await
}
