//! Shared operator-facing integration contracts. External providers use the
//! SDK service ABI instead of receiving a general Agena administrator token.
pub use agena_superagent::{
    AdapterDescription, AdapterKind, BindingState, CreateIntegration, IntegrationBinding,
    IntegrationControl, IntegrationControlAction, IntegrationDelivery, IntegrationDetail,
    IntegrationEvent, IntegrationMode, IntegrationProvider, IntegrationSpec, IntegrationsSnapshot,
};

pub use agena_superagent::{
    ChannelAccessAudit, ChannelAccessPolicy, ChannelGrant, ChannelIdentity, ChannelKind,
    ChannelMember, ChannelOutputRoute, ChannelPolicyUpdate, ChannelRole, ChannelSessionMode,
    ChannelSessionRecord, ChannelThreadMode, ChannelTrigger,
};
