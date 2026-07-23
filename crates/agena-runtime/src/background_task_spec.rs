use crate::{RuntimeBackgroundTaskKind, RuntimeBackgroundTaskOrigin};

/// Immutable input used to register a runtime background task.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeBackgroundTaskSpec {
    kind: RuntimeBackgroundTaskKind,
    origin: RuntimeBackgroundTaskOrigin,
    title: String,
    dedupe_key: Option<String>,
    cancellable: bool,
}

impl RuntimeBackgroundTaskSpec {
    pub(crate) fn new(
        kind: RuntimeBackgroundTaskKind,
        origin: RuntimeBackgroundTaskOrigin,
        title: impl Into<String>,
        dedupe_key: Option<String>,
        cancellable: bool,
    ) -> Self {
        Self {
            kind,
            origin,
            title: title.into(),
            dedupe_key,
            cancellable,
        }
    }

    pub(crate) fn kind(&self) -> RuntimeBackgroundTaskKind {
        self.kind
    }

    pub(crate) fn origin(&self) -> RuntimeBackgroundTaskOrigin {
        self.origin
    }

    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    pub(crate) fn dedupe_key(&self) -> Option<&str> {
        self.dedupe_key.as_deref()
    }

    pub(crate) fn cancellable(&self) -> bool {
        self.cancellable
    }
}
