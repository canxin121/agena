//! Effect-owned typed event pipelines inspired by Cordis composition modes.
//!
//! Pipelines define composition semantics only. They do not grant authority:
//! Runtime permission checks remain authoritative, and live events are not
//! durable state. Handler registrations belong to exact plugin generations and
//! disappear automatically when the owning `PluginEffectScope` disposes.

use portable_atomic::AtomicU64;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};

use serde::{Deserialize, Serialize};

use crate::effect_scope::{PluginEffectHandle, PluginEffectScope};
use crate::sdk::PluginKey;

pub type PluginEventFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginEventMode {
    Observe,
    ParallelObserve,
    Bail,
    Transform,
    TransformBail,
    Around,
    Guard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEventDefinition {
    pub id: String,
    pub mode: PluginEventMode,
    #[serde(default)]
    pub durable: bool,
    #[serde(default)]
    pub scoped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginPipelineFailurePolicy {
    Abort,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginGuardErrorPolicy {
    Deny,
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginPipelineHandlerDescriptor {
    pub owner: PluginKey,
    pub id: String,
    pub priority: i32,
    pub registration: u64,
}

impl PluginPipelineHandlerDescriptor {
    fn sort_key(&self) -> (std::cmp::Reverse<i32>, u64, String, &str) {
        (
            std::cmp::Reverse(self.priority),
            self.registration,
            self.owner.to_string(),
            self.id.as_str(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginPipelineFailure {
    pub owner: PluginKey,
    pub handler_id: String,
    pub message: String,
}

impl PluginPipelineFailure {
    fn new(meta: &PluginPipelineHandlerDescriptor, message: String) -> Self {
        Self {
            owner: meta.owner.clone(),
            handler_id: meta.id.clone(),
            message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPipelineError {
    pub failure: PluginPipelineFailure,
}

impl std::fmt::Display for PluginPipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "pipeline handler `{}/{}` failed: {}",
            self.failure.owner, self.failure.handler_id, self.failure.message
        )
    }
}

impl std::error::Error for PluginPipelineError {}

type RemoveHandler = Box<dyn FnOnce() + Send + 'static>;

pub struct PluginPipelineRegistration {
    descriptor: PluginPipelineHandlerDescriptor,
    owner: Weak<PluginEffectScope>,
    effect: PluginEffectHandle,
    remove: Mutex<Option<RemoveHandler>>,
}

impl std::fmt::Debug for PluginPipelineRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginPipelineRegistration")
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

impl PluginPipelineRegistration {
    pub fn descriptor(&self) -> &PluginPipelineHandlerDescriptor {
        &self.descriptor
    }

    pub fn generation(&self) -> u64 {
        self.descriptor.registration
    }

    pub async fn dispose(self) -> Result<(), String> {
        if let Some(remove) = self
            .remove
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            remove();
        }
        if let Some(owner) = self.owner.upgrade() {
            owner.release_handle(&self.effect);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginObserveReport {
    pub attempted: usize,
    pub completed: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<PluginPipelineFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginBailReport<O> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<O>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<PluginPipelineFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginTransformReport<T> {
    pub value: T,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<PluginPipelineFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginTransformBailControl<T, B> {
    Continue(T),
    Bail(B),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginTransformBailOutcome<T, B> {
    Continue(T),
    Bail(B),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginTransformBailReport<T, B> {
    pub outcome: PluginTransformBailOutcome<T, B>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<PluginPipelineFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PluginGuardDecision {
    Abstain,
    Deny { code: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginGuardReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denial: Option<PluginGuardDenial>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<PluginPipelineFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginGuardDenial {
    pub owner: PluginKey,
    pub handler_id: String,
    pub code: String,
    pub reason: String,
}

struct Handler<Cb> {
    meta: PluginPipelineHandlerDescriptor,
    callback: Cb,
}

fn sort_handlers<Cb>(handlers: &mut [Handler<Cb>]) {
    handlers.sort_by(|left, right| left.meta.sort_key().cmp(&right.meta.sort_key()));
}

fn next_registration(counter: &AtomicU64) -> u64 {
    counter.fetch_add(1, Ordering::AcqRel)
}

fn register_owned<Cb: Send + Sync + 'static>(
    handlers: &Arc<Mutex<Vec<Handler<Cb>>>>,
    counter: &AtomicU64,
    owner: &Arc<PluginEffectScope>,
    priority: i32,
    label: impl Into<String>,
    callback: Cb,
) -> Result<PluginPipelineRegistration, PluginPipelineError> {
    let label = label.into();
    let registration = next_registration(counter);
    let descriptor = PluginPipelineHandlerDescriptor {
        owner: owner.plugin_id().clone(),
        id: label.clone(),
        priority,
        registration,
    };
    handlers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(Handler {
            meta: descriptor.clone(),
            callback,
        });

    let weak_handlers = Arc::downgrade(handlers);
    let remove_for_effect: RemoveHandler = Box::new(move || {
        if let Some(handlers) = weak_handlers.upgrade() {
            handlers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .retain(|handler| handler.meta.registration != registration);
        }
    });
    let shared_remove = Arc::new(Mutex::new(Some(remove_for_effect)));
    let effect_remove = Arc::clone(&shared_remove);
    let effect = match owner.own_sync("event.handler", label, move || {
        if let Some(remove) = effect_remove
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            remove();
        }
        Ok(())
    }) {
        Ok(effect) => effect,
        Err(error) => {
            handlers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .retain(|handler| handler.meta.registration != registration);
            return Err(PluginPipelineError {
                failure: PluginPipelineFailure::new(&descriptor, error.to_string()),
            });
        }
    };
    let manual_remove = Arc::clone(&shared_remove);
    Ok(PluginPipelineRegistration {
        descriptor,
        owner: Arc::downgrade(owner),
        effect,
        remove: Mutex::new(Some(Box::new(move || {
            if let Some(remove) = manual_remove
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                remove();
            }
        }))),
    })
}

type ObserveCallback<I> = Arc<dyn Fn(I) -> PluginEventFuture<Result<(), String>> + Send + Sync>;

pub struct PluginObservePipeline<I> {
    handlers: Arc<Mutex<Vec<Handler<ObserveCallback<I>>>>>,
    next: AtomicU64,
    parallel: bool,
}

impl<I> PluginObservePipeline<I>
where
    I: Clone + Send + 'static,
{
    pub fn new(parallel: bool) -> Self {
        Self {
            handlers: Arc::new(Mutex::new(Vec::new())),
            next: AtomicU64::new(1),
            parallel,
        }
    }

    pub fn register<F, Fut>(
        &self,
        owner: &Arc<PluginEffectScope>,
        priority: i32,
        label: impl Into<String>,
        callback: F,
    ) -> Result<PluginPipelineRegistration, PluginPipelineError>
    where
        F: Fn(I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        register_owned(
            &self.handlers,
            &self.next,
            owner,
            priority,
            label,
            Arc::new(move |input| Box::pin(callback(input))),
        )
    }

    pub async fn dispatch(&self, input: I) -> PluginObserveReport {
        let mut handlers = self
            .handlers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|handler| Handler {
                meta: handler.meta.clone(),
                callback: Arc::clone(&handler.callback),
            })
            .collect::<Vec<_>>();
        sort_handlers(&mut handlers);
        let attempted = handlers.len();
        if !self.parallel {
            let mut report = PluginObserveReport {
                attempted,
                ..Default::default()
            };
            for handler in handlers {
                match (handler.callback)(input.clone()).await {
                    Ok(()) => report.completed += 1,
                    Err(error) => report
                        .failures
                        .push(PluginPipelineFailure::new(&handler.meta, error)),
                }
            }
            return report;
        }
        let mut tasks = tokio::task::JoinSet::new();
        for handler in handlers {
            let input = input.clone();
            tasks.spawn(async move {
                let result = (handler.callback)(input).await;
                (handler.meta, result)
            });
        }
        let mut report = PluginObserveReport {
            attempted,
            ..Default::default()
        };
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok((_, Ok(()))) => report.completed += 1,
                Ok((meta, Err(error))) => report
                    .failures
                    .push(PluginPipelineFailure::new(&meta, error)),
                Err(error) => report.failures.push(PluginPipelineFailure {
                    owner: "agena.host".parse().expect("host key"),
                    handler_id: "parallel_join".into(),
                    message: error.to_string(),
                }),
            }
        }
        report
            .failures
            .sort_by(|a, b| a.owner.cmp(&b.owner).then(a.handler_id.cmp(&b.handler_id)));
        report
    }

    pub fn inventory(&self) -> Vec<PluginPipelineHandlerDescriptor> {
        inventory(&self.handlers)
    }
}

type BailCallback<I, O> =
    Arc<dyn Fn(I) -> PluginEventFuture<Result<Option<O>, String>> + Send + Sync>;

pub struct PluginBailPipeline<I, O> {
    handlers: Arc<Mutex<Vec<Handler<BailCallback<I, O>>>>>,
    next: AtomicU64,
}

impl<I, O> Default for PluginBailPipeline<I, O>
where
    I: Clone + Send + 'static,
    O: Send + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<I, O> PluginBailPipeline<I, O>
where
    I: Clone + Send + 'static,
    O: Send + 'static,
{
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(Mutex::new(Vec::new())),
            next: AtomicU64::new(1),
        }
    }

    pub fn register<F, Fut>(
        &self,
        owner: &Arc<PluginEffectScope>,
        priority: i32,
        label: impl Into<String>,
        callback: F,
    ) -> Result<PluginPipelineRegistration, PluginPipelineError>
    where
        F: Fn(I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<O>, String>> + Send + 'static,
    {
        register_owned(
            &self.handlers,
            &self.next,
            owner,
            priority,
            label,
            Arc::new(move |input| Box::pin(callback(input))),
        )
    }

    pub async fn dispatch(&self, input: I) -> PluginBailReport<O> {
        let mut handlers = clone_handlers(&self.handlers);
        sort_handlers(&mut handlers);
        let mut failures = Vec::new();
        for handler in handlers {
            match (handler.callback)(input.clone()).await {
                Ok(Some(value)) => {
                    return PluginBailReport {
                        value: Some(value),
                        failures,
                    };
                }
                Ok(None) => {}
                Err(error) => failures.push(PluginPipelineFailure::new(&handler.meta, error)),
            }
        }
        PluginBailReport {
            value: None,
            failures,
        }
    }

    pub fn inventory(&self) -> Vec<PluginPipelineHandlerDescriptor> {
        inventory(&self.handlers)
    }
}

type TransformCallback<T> = Arc<dyn Fn(T) -> PluginEventFuture<Result<T, String>> + Send + Sync>;

pub struct PluginTransformPipeline<T> {
    handlers: Arc<Mutex<Vec<Handler<TransformCallback<T>>>>>,
    next: AtomicU64,
    failure_policy: PluginPipelineFailurePolicy,
}

impl<T> PluginTransformPipeline<T>
where
    T: Clone + Send + 'static,
{
    pub fn new(failure_policy: PluginPipelineFailurePolicy) -> Self {
        Self {
            handlers: Arc::new(Mutex::new(Vec::new())),
            next: AtomicU64::new(1),
            failure_policy,
        }
    }
    pub fn register<F, Fut>(
        &self,
        owner: &Arc<PluginEffectScope>,
        priority: i32,
        label: impl Into<String>,
        callback: F,
    ) -> Result<PluginPipelineRegistration, PluginPipelineError>
    where
        F: Fn(T) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
    {
        register_owned(
            &self.handlers,
            &self.next,
            owner,
            priority,
            label,
            Arc::new(move |input| Box::pin(callback(input))),
        )
    }
    pub async fn dispatch(
        &self,
        mut value: T,
    ) -> Result<PluginTransformReport<T>, PluginPipelineError> {
        let mut handlers = clone_handlers(&self.handlers);
        sort_handlers(&mut handlers);
        let mut failures = Vec::new();
        for handler in handlers {
            let backup = value.clone();
            match (handler.callback)(value).await {
                Ok(next) => value = next,
                Err(error) => {
                    let failure = PluginPipelineFailure::new(&handler.meta, error);
                    if self.failure_policy == PluginPipelineFailurePolicy::Abort {
                        return Err(PluginPipelineError { failure });
                    }
                    failures.push(failure);
                    value = backup;
                }
            }
        }
        Ok(PluginTransformReport { value, failures })
    }
    pub fn inventory(&self) -> Vec<PluginPipelineHandlerDescriptor> {
        inventory(&self.handlers)
    }
}

type TransformBailCallback<T, B> = Arc<
    dyn Fn(T) -> PluginEventFuture<Result<PluginTransformBailControl<T, B>, String>> + Send + Sync,
>;

pub struct PluginTransformBailPipeline<T, B> {
    handlers: Arc<Mutex<Vec<Handler<TransformBailCallback<T, B>>>>>,
    next: AtomicU64,
    failure_policy: PluginPipelineFailurePolicy,
}
impl<T, B> PluginTransformBailPipeline<T, B>
where
    T: Clone + Send + 'static,
    B: Send + 'static,
{
    pub fn new(failure_policy: PluginPipelineFailurePolicy) -> Self {
        Self {
            handlers: Arc::new(Mutex::new(Vec::new())),
            next: AtomicU64::new(1),
            failure_policy,
        }
    }
    pub fn register<F, Fut>(
        &self,
        owner: &Arc<PluginEffectScope>,
        priority: i32,
        label: impl Into<String>,
        callback: F,
    ) -> Result<PluginPipelineRegistration, PluginPipelineError>
    where
        F: Fn(T) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<PluginTransformBailControl<T, B>, String>> + Send + 'static,
    {
        register_owned(
            &self.handlers,
            &self.next,
            owner,
            priority,
            label,
            Arc::new(move |input| Box::pin(callback(input))),
        )
    }
    pub async fn dispatch(
        &self,
        mut value: T,
    ) -> Result<PluginTransformBailReport<T, B>, PluginPipelineError> {
        let mut handlers = clone_handlers(&self.handlers);
        sort_handlers(&mut handlers);
        let mut failures = Vec::new();
        for handler in handlers {
            let backup = value.clone();
            match (handler.callback)(value).await {
                Ok(PluginTransformBailControl::Continue(next)) => value = next,
                Ok(PluginTransformBailControl::Bail(answer)) => {
                    return Ok(PluginTransformBailReport {
                        outcome: PluginTransformBailOutcome::Bail(answer),
                        failures,
                    });
                }
                Err(error) => {
                    let failure = PluginPipelineFailure::new(&handler.meta, error);
                    if self.failure_policy == PluginPipelineFailurePolicy::Abort {
                        return Err(PluginPipelineError { failure });
                    }
                    failures.push(failure);
                    value = backup;
                }
            }
        }
        Ok(PluginTransformBailReport {
            outcome: PluginTransformBailOutcome::Continue(value),
            failures,
        })
    }
    pub fn inventory(&self) -> Vec<PluginPipelineHandlerDescriptor> {
        inventory(&self.handlers)
    }
}

type GuardCallback<I> =
    Arc<dyn Fn(I) -> PluginEventFuture<Result<PluginGuardDecision, String>> + Send + Sync>;
pub struct PluginGuardPipeline<I> {
    handlers: Arc<Mutex<Vec<Handler<GuardCallback<I>>>>>,
    next: AtomicU64,
    error_policy: PluginGuardErrorPolicy,
}
impl<I> PluginGuardPipeline<I>
where
    I: Clone + Send + 'static,
{
    pub fn new(error_policy: PluginGuardErrorPolicy) -> Self {
        Self {
            handlers: Arc::new(Mutex::new(Vec::new())),
            next: AtomicU64::new(1),
            error_policy,
        }
    }
    pub fn register<F, Fut>(
        &self,
        owner: &Arc<PluginEffectScope>,
        priority: i32,
        label: impl Into<String>,
        callback: F,
    ) -> Result<PluginPipelineRegistration, PluginPipelineError>
    where
        F: Fn(I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<PluginGuardDecision, String>> + Send + 'static,
    {
        register_owned(
            &self.handlers,
            &self.next,
            owner,
            priority,
            label,
            Arc::new(move |input| Box::pin(callback(input))),
        )
    }
    pub async fn dispatch(&self, input: I) -> PluginGuardReport {
        let mut handlers = clone_handlers(&self.handlers);
        sort_handlers(&mut handlers);
        let mut failures = Vec::new();
        for handler in handlers {
            match (handler.callback)(input.clone()).await {
                Ok(PluginGuardDecision::Abstain) => {}
                Ok(PluginGuardDecision::Deny { code, reason }) => {
                    return PluginGuardReport {
                        denial: Some(PluginGuardDenial {
                            owner: handler.meta.owner,
                            handler_id: handler.meta.id,
                            code,
                            reason,
                        }),
                        failures,
                    };
                }
                Err(error) => {
                    let failure = PluginPipelineFailure::new(&handler.meta, error.clone());
                    failures.push(failure);
                    if self.error_policy == PluginGuardErrorPolicy::Deny {
                        return PluginGuardReport {
                            denial: Some(PluginGuardDenial {
                                owner: handler.meta.owner,
                                handler_id: handler.meta.id,
                                code: "guard_handler_failed".into(),
                                reason: error,
                            }),
                            failures,
                        };
                    }
                }
            }
        }
        PluginGuardReport {
            denial: None,
            failures,
        }
    }
    pub fn inventory(&self) -> Vec<PluginPipelineHandlerDescriptor> {
        inventory(&self.handlers)
    }
}

pub type PluginAroundFuture<O, E> = Pin<Box<dyn Future<Output = Result<O, E>> + Send + 'static>>;
type AroundCallback<C, O, E> =
    Arc<dyn Fn(C, PluginAroundNext<C, O, E>) -> PluginAroundFuture<O, E> + Send + Sync>;
type AroundTerminal<C, O, E> = Arc<dyn Fn(C) -> PluginAroundFuture<O, E> + Send + Sync>;
struct AroundHandler<C, O, E> {
    meta: PluginPipelineHandlerDescriptor,
    callback: AroundCallback<C, O, E>,
}
impl<C, O, E> Clone for AroundHandler<C, O, E> {
    fn clone(&self) -> Self {
        Self {
            meta: self.meta.clone(),
            callback: Arc::clone(&self.callback),
        }
    }
}

#[derive(Clone)]
pub struct PluginAroundNext<C, O, E> {
    index: usize,
    handlers: Arc<Vec<AroundHandler<C, O, E>>>,
    terminal: AroundTerminal<C, O, E>,
}
impl<C, O, E> PluginAroundNext<C, O, E>
where
    C: Send + 'static,
    O: Send + 'static,
    E: Send + 'static,
{
    pub fn run(self, context: C) -> PluginAroundFuture<O, E> {
        Box::pin(async move {
            if let Some(handler) = self.handlers.get(self.index).cloned() {
                let next = Self {
                    index: self.index + 1,
                    handlers: Arc::clone(&self.handlers),
                    terminal: Arc::clone(&self.terminal),
                };
                (handler.callback)(context, next).await
            } else {
                (self.terminal)(context).await
            }
        })
    }
}

pub struct PluginAroundPipeline<C, O, E> {
    handlers: Arc<Mutex<Vec<AroundHandler<C, O, E>>>>,
    next: AtomicU64,
}

impl<C, O, E> Default for PluginAroundPipeline<C, O, E>
where
    C: Send + 'static,
    O: Send + 'static,
    E: Send + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<C, O, E> PluginAroundPipeline<C, O, E>
where
    C: Send + 'static,
    O: Send + 'static,
    E: Send + 'static,
{
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(Mutex::new(Vec::new())),
            next: AtomicU64::new(1),
        }
    }
    pub fn register<F, Fut>(
        &self,
        owner: &Arc<PluginEffectScope>,
        priority: i32,
        label: impl Into<String>,
        callback: F,
    ) -> Result<PluginPipelineRegistration, PluginPipelineError>
    where
        F: Fn(C, PluginAroundNext<C, O, E>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, E>> + Send + 'static,
    {
        let label = label.into();
        let registration = next_registration(&self.next);
        let descriptor = PluginPipelineHandlerDescriptor {
            owner: owner.plugin_id().clone(),
            id: label.clone(),
            priority,
            registration,
        };
        self.handlers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(AroundHandler {
                meta: descriptor.clone(),
                callback: Arc::new(move |context, next| Box::pin(callback(context, next))),
            });
        let weak = Arc::downgrade(&self.handlers);
        let remove: RemoveHandler = Box::new(move || {
            if let Some(handlers) = weak.upgrade() {
                handlers
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .retain(|handler| handler.meta.registration != registration);
            }
        });
        let shared = Arc::new(Mutex::new(Some(remove)));
        let effect_shared = Arc::clone(&shared);
        let effect = match owner.own_sync("event.handler", label, move || {
            if let Some(remove) = effect_shared
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
            {
                remove();
            }
            Ok(())
        }) {
            Ok(effect) => effect,
            Err(error) => {
                self.handlers
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .retain(|handler| handler.meta.registration != registration);
                return Err(PluginPipelineError {
                    failure: PluginPipelineFailure::new(&descriptor, error.to_string()),
                });
            }
        };
        let manual = Arc::clone(&shared);
        Ok(PluginPipelineRegistration {
            descriptor,
            owner: Arc::downgrade(owner),
            effect,
            remove: Mutex::new(Some(Box::new(move || {
                if let Some(remove) = manual.lock().unwrap_or_else(|p| p.into_inner()).take() {
                    remove();
                }
            }))),
        })
    }
    pub async fn dispatch<F, Fut>(&self, context: C, terminal: F) -> Result<O, E>
    where
        F: Fn(C) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, E>> + Send + 'static,
    {
        let mut handlers = self
            .handlers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        handlers.sort_by(|a, b| a.meta.sort_key().cmp(&b.meta.sort_key()));
        PluginAroundNext {
            index: 0,
            handlers: Arc::new(handlers),
            terminal: Arc::new(move |context| Box::pin(terminal(context))),
        }
        .run(context)
        .await
    }
    pub fn inventory(&self) -> Vec<PluginPipelineHandlerDescriptor> {
        let mut items = self
            .handlers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|h| h.meta.clone())
            .collect::<Vec<_>>();
        items.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        items
    }
}

fn clone_handlers<Cb: Clone>(handlers: &Arc<Mutex<Vec<Handler<Cb>>>>) -> Vec<Handler<Cb>> {
    handlers
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .map(|h| Handler {
            meta: h.meta.clone(),
            callback: h.callback.clone(),
        })
        .collect()
}
fn inventory<Cb>(handlers: &Arc<Mutex<Vec<Handler<Cb>>>>) -> Vec<PluginPipelineHandlerDescriptor> {
    let mut items = handlers
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .map(|h| h.meta.clone())
        .collect::<Vec<_>>();
    items.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn owner(id: &str) -> Arc<PluginEffectScope> {
        PluginEffectScope::new(id.parse().unwrap())
    }
    #[tokio::test]
    async fn transform_bail_is_ordered_and_effect_owned() {
        let pipeline =
            PluginTransformBailPipeline::<i32, String>::new(PluginPipelineFailurePolicy::Abort);
        let a = owner("example.a");
        let b = owner("example.b");
        pipeline
            .register(&a, 0, "plus", |value| async move {
                Ok(PluginTransformBailControl::Continue(value + 1))
            })
            .unwrap();
        pipeline
            .register(&b, -1, "stop", |value| async move {
                Ok(if value == 2 {
                    PluginTransformBailControl::Bail("done".into())
                } else {
                    PluginTransformBailControl::Continue(value)
                })
            })
            .unwrap();
        let report = pipeline.dispatch(1).await.unwrap();
        assert_eq!(
            report.outcome,
            PluginTransformBailOutcome::Bail("done".into())
        );
        assert_eq!(pipeline.inventory().len(), 2);
        a.dispose().await;
        assert_eq!(pipeline.inventory().len(), 1);
    }

    #[tokio::test]
    async fn transform_continue_restores_the_pre_handler_value_after_failure() {
        let pipeline =
            PluginTransformPipeline::<Vec<String>>::new(PluginPipelineFailurePolicy::Continue);
        let a = owner("example.transform-a");
        let b = owner("example.transform-b");
        pipeline
            .register(&a, 10, "append-a", |mut value| async move {
                value.push("a".to_string());
                Ok(value)
            })
            .unwrap();
        pipeline
            .register(&b, 5, "broken", |mut value| async move {
                value.push("must-not-leak".to_string());
                Err("broken".to_string())
            })
            .unwrap();
        pipeline
            .register(&b, 0, "append-b", |mut value| async move {
                value.push("b".to_string());
                Ok(value)
            })
            .unwrap();

        let report = pipeline.dispatch(vec!["root".to_string()]).await.unwrap();
        assert_eq!(report.value, ["root", "a", "b"]);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].handler_id, "broken");
    }

    #[tokio::test]
    async fn transform_bail_continue_keeps_the_last_good_value() {
        let pipeline =
            PluginTransformBailPipeline::<i32, String>::new(PluginPipelineFailurePolicy::Continue);
        let scope = owner("example.transform-bail");
        pipeline
            .register(&scope, 10, "plus-one", |value| async move {
                Ok(PluginTransformBailControl::Continue(value + 1))
            })
            .unwrap();
        pipeline
            .register(&scope, 5, "broken", |_value| async move {
                Err("broken".to_string())
            })
            .unwrap();
        pipeline
            .register(&scope, 0, "stop", |value| async move {
                Ok(PluginTransformBailControl::Bail(format!("value={value}")))
            })
            .unwrap();

        let report = pipeline.dispatch(1).await.unwrap();
        assert_eq!(
            report.outcome,
            PluginTransformBailOutcome::Bail("value=2".to_string())
        );
        assert_eq!(report.failures.len(), 1);
    }
    #[tokio::test]
    async fn around_nests_and_owner_disposal_removes_middleware() {
        let pipeline = PluginAroundPipeline::<i32, i32, String>::new();
        let scope = owner("example.middleware");
        let runs = Arc::new(AtomicUsize::new(0));
        let effect_runs = Arc::clone(&runs);
        pipeline
            .register(&scope, 10, "double", move |value, next| {
                let effect_runs = Arc::clone(&effect_runs);
                async move {
                    effect_runs.fetch_add(1, Ordering::AcqRel);
                    Ok(next.run(value).await? * 2)
                }
            })
            .unwrap();
        assert_eq!(
            pipeline
                .dispatch(3, |value| async move { Ok(value + 1) })
                .await
                .unwrap(),
            8
        );
        assert_eq!(runs.load(Ordering::Acquire), 1);
        scope.dispose().await;
        assert!(pipeline.inventory().is_empty());
        assert_eq!(
            pipeline
                .dispatch(3, |value| async move { Ok(value + 1) })
                .await
                .unwrap(),
            4
        );
    }
    #[tokio::test]
    async fn guard_is_monotonic_and_fails_closed() {
        let pipeline = PluginGuardPipeline::<String>::new(PluginGuardErrorPolicy::Deny);
        let scope = owner("example.guard");
        pipeline
            .register(&scope, 0, "boom", |_| async { Err("broken".into()) })
            .unwrap();
        let report = pipeline.dispatch("x".into()).await;
        assert_eq!(report.denial.unwrap().code, "guard_handler_failed");
    }
}
