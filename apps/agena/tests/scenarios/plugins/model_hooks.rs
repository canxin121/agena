//! Model and run hooks exercised through the real HTTP plugin and model protocols.
//! The probe records callbacks; it never calls a hook, kernel, or manager itself.
use std::collections::BTreeMap;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

use super::super::fixture;
use agena_api::resource::RunOptions;
use agena_domain::{
    CancellationResult,
    session_kernel::{ExecutionAttemptSettlement, ExecutionPhase},
};
use agena_plugin_sdk::prelude::*;
use agena_testkit::{Reply, RequestMatch, SettingsLayer, TestAgenaBuilder, TestResult};
use anyhow::{Context, ensure};
use tokio::sync::Semaphore;

#[derive(Clone, Debug)]
struct Event {
    name: &'static str,
    input: Value,
}

struct Observations {
    events: Mutex<Vec<Event>>,
    retry_errors: AtomicBool,
    gate_next_post: AtomicBool,
    post_entered: Semaphore,
    post_release: Semaphore,
    completed_posts: AtomicUsize,
}

impl Default for Observations {
    fn default() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            retry_errors: AtomicBool::new(false),
            gate_next_post: AtomicBool::new(false),
            post_entered: Semaphore::new(0),
            post_release: Semaphore::new(0),
            completed_posts: AtomicUsize::new(0),
        }
    }
}

impl Observations {
    fn record(&self, name: &'static str, input: &impl Serialize) {
        self.events.lock().unwrap().push(Event {
            name,
            input: serde_json::to_value(input).expect("hook input is serializable"),
        });
    }

    fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    fn count(&self, name: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.name == name)
            .count()
    }

    async fn wait_for_post(&self) -> anyhow::Result<()> {
        tokio::time::timeout(Duration::from_secs(15), self.post_entered.acquire())
            .await
            .context("the real run.post hook was not reached")??
            .forget();
        Ok(())
    }
}

struct ModelHookProbe {
    seen: Arc<Observations>,
}

#[agena_plugin(
    namespace = "testkit",
    name = "model-hooks",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Observe the actual provider and execution lifecycle over HTTP."
)]
impl ModelHookProbe {
    #[hook(chat.system, summary = "Append system context", help = "Adds a provider-bound marker without changing persisted prompts.")]
    async fn system(&self, input: ChatSystemTransformInput) -> Option<ChatSystemTransformPatch> {
        self.seen.record("chat.system", &input);
        Some(ChatSystemTransformPatch {
            append: Some("\nSYSTEM HOOK MARKER".into()),
            ..Default::default()
        })
    }

    #[hook(chat.message, summary = "Observe message projection", help = "Records the direction and session of each actual projected message.")]
    async fn message(&self, input: ChatMessageInput) -> Option<ChatMessagePatch> {
        self.seen.record("chat.message", &input);
        None
    }

    #[hook(chat.messages, summary = "Add transient context", help = "Injects a synthetic system message into the outgoing model projection.")]
    async fn messages(
        &self,
        input: ChatMessagesTransformInput,
    ) -> Option<ChatMessagesTransformPatch> {
        self.seen.record("chat.messages", &input);
        let mut messages = vec![ChatMessage::system("MEMORY HOOK MARKER")];
        messages.extend(input.messages);
        Some(ChatMessagesTransformPatch {
            messages: Some(messages),
        })
    }

    #[hook(chat.headers, summary = "Set scoped model headers", help = "Adds a header to the real model HTTP request.")]
    async fn headers(&self, input: ChatHeadersInput) -> Option<ChatHeadersPatch> {
        self.seen.record("chat.headers", &input);
        Some(ChatHeadersPatch {
            set: BTreeMap::from([("x-hook-probe".into(), "present".into())]),
            remove: Vec::new(),
        })
    }

    #[hook(run.pre, summary = "Observe run admission", help = "Records the execution before its first provider effect.")]
    async fn before_run(&self, input: PreRunInput) {
        self.seen.record("run.pre", &input);
    }

    #[hook(run.post, summary = "Observe run settlement", help = "Can hold the final hook while tests use the public attempt API.")]
    async fn after_run(&self, input: PostRunInput) {
        self.seen.record("run.post", &input);
        if self.seen.gate_next_post.swap(false, Ordering::SeqCst) {
            self.seen.post_entered.add_permits(1);
            if let Ok(permit) = self.seen.post_release.acquire().await {
                permit.forget();
            }
        }
        self.seen.completed_posts.fetch_add(1, Ordering::SeqCst);
    }

    #[hook(model.request.before, summary = "Observe model admission", help = "Records each admitted provider request before network I/O.")]
    async fn model_before(
        &self,
        input: ModelRequestBeforeInput,
    ) -> Option<ModelRequestBeforePatch> {
        self.seen.record("model.request.before", &input);
        None
    }

    #[hook(model.request.after, summary = "Observe model completion", help = "Records successful and failed provider rounds.")]
    async fn model_after(&self, input: ModelRequestAfterInput) {
        self.seen.record("model.request.after", &input);
    }

    #[hook(model.request.error, summary = "Control an explicit retry", help = "Requests a new round only for the scripted retry marker.")]
    async fn model_error(&self, input: ModelRequestErrorInput) -> Option<ModelRequestErrorPatch> {
        self.seen.record("model.request.error", &input);
        Some(ModelRequestErrorPatch {
            retry: self.seen.retry_errors.load(Ordering::SeqCst)
                && input.message.contains("retryable-probe-failure"),
        })
    }
}

fn hooked_fixture(seen: Arc<Observations>) -> TestAgenaBuilder {
    fixture()
        .before_start(move |case| async move {
            let router = agena_plugin_sdk::drivers::http::router(
                move || ModelHookProbe { seen: seen.clone() },
                Arc::new(NoopHostClient),
            );
            let service = case.serve_http(router).await?;
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["testkit.model-hooks"] = json!({
                "enabled": true,
                "package": {"kind": "http", "url": service.url("/rpc")}
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .expect_plugin("testkit.model-hooks")
}

// An assertion failure must release the callback before the fixture drains the SUT.
struct ReleaseOnDrop(Arc<Observations>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.post_release.add_permits(1);
    }
}

fn assert_counts(seen: &Observations, expected: &[(&str, usize)]) -> anyhow::Result<()> {
    for (name, count) in expected {
        ensure!(
            seen.count(name) == *count,
            "{name}: expected {count}, got {:?}",
            seen.events()
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_hooks_fire_at_the_actual_provider_boundary() -> TestResult {
    let seen = Arc::new(Observations::default());
    hooked_fixture(seen.clone())
        .run(move |case| async move {
            case.model()
                .expect(RequestMatch::user_text("provider-boundary-marker"))
                .check(|request| {
                    ensure!(
                        request.header("x-hook-probe") == Some("present"),
                        "{:?}",
                        request.headers
                    );
                    let body = request.body.to_string();
                    ensure!(
                        body.contains("SYSTEM HOOK MARKER"),
                        "system hook did not reach HTTP"
                    );
                    ensure!(
                        body.contains("MEMORY HOOK MARKER"),
                        "messages hook did not reach HTTP"
                    );
                    Ok(())
                })
                .reply(Reply::text("provider-boundary-answer"));
            let user = case.user().new_session().await?;
            let run = user.send("provider-boundary-marker").await?;
            run.wait_settled()
                .await?
                .assert_assistant_text("provider-boundary-answer")?;

            let events = seen.events();
            for name in [
                "chat.system",
                "chat.message",
                "chat.messages",
                "chat.headers",
                "run.pre",
                "run.post",
                "model.request.before",
                "model.request.after",
            ] {
                ensure!(
                    events.iter().any(|event| event.name == name),
                    "missing {name}: {events:?}"
                );
            }
            let position = |name| events.iter().position(|event| event.name == name).unwrap();
            ensure!(
                position("model.request.before") < position("chat.headers"),
                "{events:?}"
            );
            for event in &events {
                ensure!(event.input["session_id"] == user.id(), "{event:?}");
                if event.name.starts_with("model.request.") {
                    ensure!(event.input["provider"] == "testkit", "{event:?}");
                }
                if event.name == "chat.headers" {
                    // The transport hook identifies the selected adapter, while
                    // request lifecycle hooks expose the configured provider ID.
                    ensure!(
                        event.input["provider"] == "testkit::openai_responses",
                        "{event:?}"
                    );
                }
                if event.name.starts_with("run.") || event.name.starts_with("model.request.") {
                    ensure!(event.input["model"] == "fake-model", "{event:?}");
                }
                if event.name.starts_with("run.") {
                    ensure!(event.input["kind"] == "conversation", "{event:?}");
                    ensure!(
                        event.input["execution_id"] == run.receipt().execution_id.to_string(),
                        "{event:?}"
                    );
                }
                if event.name == "chat.message" {
                    ensure!(event.input["direction"] == "to_provider", "{event:?}");
                }
            }
            ensure!(
                events
                    .iter()
                    .find(|event| event.name == "run.post")
                    .unwrap()
                    .input["status"]
                    == "succeeded"
            );
            assert_counts(
                &seen,
                &[
                    ("run.pre", 1),
                    ("run.post", 1),
                    ("model.request.before", 1),
                    ("model.request.after", 1),
                ],
            )?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_hooks_fire_once_even_when_an_execution_requires_two_model_rounds() -> TestResult {
    let seen = Arc::new(Observations::default());
    hooked_fixture(seen.clone())
        .run(move |case| async move {
            let first = case
                .model()
                .expect(RequestMatch::user_text("length-continuation-marker"))
                .reply(Reply::text("first-length-prefix").incomplete("max_output_tokens"));
            case.model()
                .expect(RequestMatch::user_text("length-continuation-marker"))
                .after(&first)
                .reply(Reply::text("second-length-answer"));
            let user = case.user().new_session().await?;
            user.send("length-continuation-marker")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            ensure!(
                case.model().requests().len() == 2,
                "a length finish did not cause exactly one new model round"
            );
            assert_counts(
                &seen,
                &[
                    ("run.pre", 1),
                    ("run.post", 1),
                    ("model.request.before", 2),
                    ("model.request.after", 2),
                    ("chat.headers", 2),
                ],
            )?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_model_request_is_retried_only_through_the_explicit_error_hook() -> TestResult {
    let seen = Arc::new(Observations::default());
    seen.retry_errors.store(true, Ordering::SeqCst);
    hooked_fixture(seen.clone()).run(move |case| async move {
        let error = || Reply::raw_json(400, json!({
            "error": {"message": "retryable-probe-failure", "type": "invalid_request_error"}
        }));
        let failed = case.model().expect(RequestMatch::user_text("hook-retry-marker")).reply(error());
        case.model().expect(RequestMatch::user_text("hook-retry-marker"))
            .after(&failed).reply(Reply::text("explicit-retry-answer"));
        let user = case.user().new_session().await?;
        user.send("hook-retry-marker").await?.wait_settled().await?.assert_assistant_text("explicit-retry-answer")?;
        assert_counts(&seen, &[("run.pre", 1), ("run.post", 1), ("model.request.before", 2), ("model.request.after", 2), ("model.request.error", 1)])?;
        let requests: Vec<_> = seen.events().into_iter().filter(|event| event.name == "model.request.before").collect();
        ensure!(requests[0].input["request_id"] != requests[1].input["request_id"], "retry reused an admitted request");

        // The same transport error is terminal when the real error hook declines it.
        seen.retry_errors.store(false, Ordering::SeqCst);
        case.model().expect(RequestMatch::user_text("hook-no-retry-marker")).reply(error());
        let other = case.user().new_session().await?;
        other.send("hook-no-retry-marker").await?.wait_settled().await?.assert_failed()?;
        ensure!(case.model().requests().len() == 3, "the declined retry still reached the provider");
        assert_counts(&seen, &[("run.pre", 2), ("run.post", 2), ("model.request.before", 3), ("model.request.after", 3), ("model.request.error", 2)])?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_attempt_waits_for_post_run_and_never_follows_a_retry() -> TestResult {
    let seen = Arc::new(Observations::default());
    hooked_fixture(seen.clone())
        .run(move |case| async move {
            let first = case
                .model()
                .expect(RequestMatch::user_text("attempt-settlement-marker"))
                .reply(Reply::text("first-attempt-answer"));
            let second = case
                .model()
                .expect(RequestMatch::user_text("attempt-settlement-marker"))
                .after(&first)
                .reply(Reply::text("second-attempt-answer").held());
            let user = case.user().new_session().await?;
            let _release_on_failure = ReleaseOnDrop(seen.clone());
            seen.gate_next_post.store(true, Ordering::SeqCst);
            let run = user.send("attempt-settlement-marker").await?;
            seen.wait_for_post().await?;
            let terminal = run.wait_terminal().await?;
            terminal.assert_succeeded()?;
            ensure!(
                terminal.status.settlement == ExecutionAttemptSettlement::Terminal,
                "{:?}",
                terminal.status
            );
            ensure!(terminal.status.owner_epoch == Some(terminal.status.kernel_epoch));
            ensure!(seen.completed_posts.load(Ordering::SeqCst) == 0);
            seen.post_release.add_permits(1);
            run.wait_settled().await?.assert_succeeded()?;
            ensure!(seen.completed_posts.load(Ordering::SeqCst) == 1);

            user.client()
                .set_message_queue_paused(user.id(), true)
                .await?;
            let retry = user.continue_run(RunOptions::default()).await?;
            ensure!(retry.receipt().execution_id == run.receipt().execution_id);
            ensure!(retry.receipt().message_id == run.receipt().message_id);
            ensure!(retry.receipt().attempt_generation > run.receipt().attempt_generation);
            let previous = run.status().await?;
            ensure!(
                previous.settlement == ExecutionAttemptSettlement::Superseded,
                "{previous:?}"
            );
            ensure!(previous.current_attempt_generation == retry.receipt().attempt_generation);
            let queued = retry.status().await?;
            ensure!(
                queued.phase == ExecutionPhase::Queued
                    && queued.settlement == ExecutionAttemptSettlement::Pending,
                "{queued:?}"
            );
            let unknown = case
                .http()
                .await
                .get(&format!(
                    "/api/v1/sessions/{}/executions/{}/attempts/{}",
                    user.id(),
                    run.receipt().execution_id,
                    retry.receipt().attempt_generation + 100,
                ))
                .await?;
            ensure!(
                unknown.status.as_u16() == 404,
                "unknown attempt: {unknown:?}"
            );
            user.client()
                .set_message_queue_paused(user.id(), false)
                .await?;
            second.observed().await?;
            second.release();
            retry
                .wait_settled()
                .await?
                .assert_assistant_text("second-attempt-answer")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn per_run_system_reaches_provider_without_persisting_or_removing_hooks() -> TestResult {
    let seen = Arc::new(Observations::default());
    hooked_fixture(seen)
        .run(|case| async move {
            let user = case.user().new_session().await?;
            user.client().set_session_prompts(user.id(), Some(serde_json::from_value(json!({
            "order": ["inherited-session-note"],
            "inline": [{"id": "inherited-session-note", "body": "INHERITED SESSION MARKER"}]
        }))?)).await?;
            for (index, system) in [None, Some("PER RUN SYSTEM MARKER"), Some(""), None]
                .into_iter()
                .enumerate()
            {
                let marker = format!("system-override-{index}-marker");
                case.model()
                    .expect(RequestMatch::user_text(marker.clone()))
                    .check(move |request| {
                        let prompt = request
                            .body
                            .get("instructions")
                            .and_then(Value::as_str)
                            .context("model request has no instructions")?;
                        ensure!(
                            prompt.contains("SYSTEM HOOK MARKER"),
                            "override removed the system hook: {prompt}"
                        );
                        ensure!(
                            prompt.contains("INHERITED SESSION MARKER") == system.is_none(),
                            "inherited prompt: {prompt}"
                        );
                        ensure!(
                            prompt.contains("PER RUN SYSTEM MARKER")
                                == (system == Some("PER RUN SYSTEM MARKER")),
                            "per-run prompt: {prompt}"
                        );
                        Ok(())
                    })
                    .reply(Reply::text(format!("system-override-{index}-answer")));
                user.send_with(
                    marker,
                    RunOptions {
                        system: system.map(str::to_owned),
                        ..Default::default()
                    },
                )
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            }
            let preview = user.client().session_prompt_preview(user.id()).await?;
            ensure!(
                serde_json::to_value(preview)?
                    .to_string()
                    .contains("INHERITED SESSION MARKER"),
                "request override persisted into the session prompt"
            );
            ensure!(case.model().requests().len() == 4);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retry_is_not_lost_while_the_previous_generation_is_still_settling() -> TestResult {
    let seen = Arc::new(Observations::default());
    hooked_fixture(seen.clone())
        .run(move |case| async move {
            let first = case
                .model()
                .expect(RequestMatch::user_text("settling-retry-marker"))
                .reply(Reply::text("old-attempt-completed"));
            let second = case
                .model()
                .expect(RequestMatch::user_text("settling-retry-marker"))
                .after(&first)
                .reply(Reply::text("new-attempt-completed").held());
            let user = case.user().new_session().await?;
            let _release_on_failure = ReleaseOnDrop(seen.clone());
            seen.gate_next_post.store(true, Ordering::SeqCst);
            let original = user.send("settling-retry-marker").await?;
            seen.wait_for_post().await?;
            original.wait_terminal().await?.assert_succeeded()?;
            let retry = user.continue_run(RunOptions::default()).await?;
            ensure!(retry.receipt().execution_id == original.receipt().execution_id);
            ensure!(retry.receipt().attempt_generation > original.receipt().attempt_generation);
            ensure!(original.status().await?.settlement == ExecutionAttemptSettlement::Superseded);
            ensure!(
                retry.status().await?.phase == ExecutionPhase::Queued,
                "the replacement started before its predecessor released run.post"
            );
            let cancelled = original.cancel().await?;
            ensure!(
                cancelled.result == CancellationResult::ExecutionMismatch,
                "{cancelled:?}"
            );
            ensure!(
                !user.snapshot().await?.queue_paused,
                "stale cancellation paused the replacement"
            );
            seen.post_release.add_permits(1);
            second.observed().await?;
            ensure!(!retry.status().await?.phase.is_terminal());
            second.release();
            retry
                .wait_settled()
                .await?
                .assert_assistant_text("new-attempt-completed")?;
            ensure!(seen.completed_posts.load(Ordering::SeqCst) == 2);
            ensure!(
                case.model().requests().len() == 2,
                "the queued replacement was duplicated"
            );
            Ok(())
        })
        .await
}
