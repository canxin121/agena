//! Runtime-neutral provider configuration model.
//!
//! Concrete credentials, backend persistence, authentication polling, model
//! catalog loading, and runtime reload are app-adapter responsibilities.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderField {
    pub key: String,
    pub label: String,
    pub value: String,
    pub secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDraft {
    pub provider_id: String,
    pub display_name: String,
    pub fields: Vec<ProviderField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStudioAction {
    SelectProvider(String),
    EditField { key: String, value: String },
    Submit,
    RefreshModels,
    StartAuth,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStudioEffect {
    LoadProviders,
    SaveProvider(ProviderDraft),
    LoadModels { provider_id: String },
    StartAuthentication { provider_id: String },
    Close,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderStudioState {
    pub selected_provider_id: Option<String>,
    pub draft: Option<ProviderDraft>,
    pub dirty: bool,
}

impl ProviderStudioState {
    pub fn reduce(&mut self, action: ProviderStudioAction) -> Option<ProviderStudioEffect> {
        match action {
            ProviderStudioAction::SelectProvider(provider_id) => {
                self.selected_provider_id = Some(provider_id);
                None
            }
            ProviderStudioAction::EditField { key, value } => {
                let draft = self.draft.as_mut()?;
                if let Some(field) = draft.fields.iter_mut().find(|field| field.key == key) {
                    field.value = value;
                    self.dirty = true;
                }
                None
            }
            ProviderStudioAction::Submit => {
                self.draft.clone().map(ProviderStudioEffect::SaveProvider)
            }
            ProviderStudioAction::RefreshModels => self
                .selected_provider_id
                .clone()
                .map(|provider_id| ProviderStudioEffect::LoadModels { provider_id }),
            ProviderStudioAction::StartAuth => self
                .selected_provider_id
                .clone()
                .map(|provider_id| ProviderStudioEffect::StartAuthentication { provider_id }),
            ProviderStudioAction::Cancel => Some(ProviderStudioEffect::Close),
        }
    }

    pub fn replace_draft(&mut self, draft: ProviderDraft) {
        self.selected_provider_id = Some(draft.provider_id.clone());
        self.draft = Some(draft);
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ProviderDraft, ProviderField, ProviderStudioAction, ProviderStudioEffect,
        ProviderStudioState,
    };

    fn state() -> ProviderStudioState {
        let mut state = ProviderStudioState::default();
        state.replace_draft(ProviderDraft {
            provider_id: "demo".into(),
            display_name: "Demo".into(),
            fields: vec![ProviderField {
                key: "base_url".into(),
                label: "Base URL".into(),
                value: "".into(),
                secret: false,
            }],
        });
        state
    }

    #[test]
    fn field_edits_are_local_until_submit() {
        let mut state = state();
        assert_eq!(
            state.reduce(ProviderStudioAction::EditField {
                key: "base_url".into(),
                value: "https://example.test".into()
            }),
            None
        );
        assert!(state.dirty);
        assert!(matches!(
            state.reduce(ProviderStudioAction::Submit),
            Some(ProviderStudioEffect::SaveProvider(_))
        ));
    }
}
