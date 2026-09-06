//! Exact ownership for replaceable global registrations. Keep this metadata
//! beside the value under its registry lock; a plugin key or equal value does
//! not distinguish registrations made by different process or logical scopes.

use std::sync::{Arc, Weak};

use crate::effect_scope::{PluginEffectHandle, PluginEffectScope, PluginEffectScopeError};

#[derive(Debug, Clone)]
pub(crate) struct RegistrationId(Arc<()>);

#[derive(Debug)]
pub(crate) struct RegistrationOwner {
    id: RegistrationId,
    scope: Weak<PluginEffectScope>,
    effect: PluginEffectHandle,
}

impl RegistrationOwner {
    /// Admission must succeed before the caller publishes the value. The
    /// disposer must compare this exact id under the same registry lock.
    pub(crate) fn new<F>(
        scope: &Arc<PluginEffectScope>,
        kind: &'static str,
        label: String,
        disposer: F,
    ) -> Result<Self, PluginEffectScopeError>
    where
        F: FnOnce(RegistrationId) -> Result<(), String> + Send + 'static,
    {
        let id = RegistrationId(Arc::new(()));
        let cleanup_id = id.clone();
        let effect = scope.own_sync(kind, label, move || disposer(cleanup_id))?;
        Ok(Self {
            id,
            scope: Arc::downgrade(scope),
            effect,
        })
    }

    pub(crate) fn matches(&self, id: &RegistrationId) -> bool {
        Arc::ptr_eq(&self.id.0, &id.0)
    }

    pub(crate) fn belongs_to(&self, scope: &PluginEffectScope) -> bool {
        std::ptr::eq(self.scope.as_ptr(), scope)
    }

    /// Release the actual old owner's handle, even when another scope made
    /// the replacement/removal. An already-running disposer remains harmless
    /// because its registration id no longer matches the published entry.
    pub(crate) fn release(self) {
        if let Some(scope) = self.scope.upgrade() {
            scope.release_handle(&self.effect);
        }
    }
}
