use super::*;
use agena_runtime_config::ResolvedAgentProfile;

pub(super) struct ConversationProfile {
    pub name: String,
    pub definition: ResolvedAgentProfile,
    pub from_bot: bool,
}

impl ConversationProfile {
    pub fn permission_ceilings(
        &self,
        configuration: &SessionConfiguration,
    ) -> Result<Vec<agena_domain::PermissionConfig>, AppError> {
        let mut layers = configuration.permission_ceiling_layers.clone();
        if let Some(ceiling) = self.definition.read_only_ceiling() {
            super::permission_config::append_permission_ceiling_layer(
                &configuration.permission_ceiling,
                &mut layers,
                &ceiling,
            )?;
        }
        if self.from_bot && configuration.channel_authority.is_some() {
            super::permission_config::append_permission_ceiling_layer(
                &configuration.permission_ceiling,
                &mut layers,
                &self.definition.permission,
            )?;
        }
        Ok(layers)
    }
}

impl SessionManager {
    pub(super) async fn conversation_profile(
        &self,
        configuration: &SessionConfiguration,
        state: &ExecutionState,
    ) -> Result<ConversationProfile, AppError> {
        let bot_profile = self.inherited_bot_profile(configuration).await?;
        let global = bot_profile.as_ref().map(|_| self.global_state.load_full());
        let config = global.as_ref().map_or(&state.config, |state| &state.config);
        let name = config.effective_profile_name(
            configuration.profile.as_deref().or(bot_profile.as_deref()),
            configuration.relation_kind,
        );
        let definition = config
            .resolve_session_profile(Some(name), configuration.relation_kind)
            .map_err(AppError::Config)?;
        Ok(ConversationProfile {
            name: name.to_owned(),
            definition,
            from_bot: bot_profile.is_some(),
        })
    }
}
