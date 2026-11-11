//! Shared operator-facing integration contracts. External providers use the
//! SDK service ABI instead of receiving a general Agena administrator token.
pub use agena_assistant::{
    AdapterDescription, AdapterKind, BindingState, ConversationCounts, ConversationEntry,
    CreateIntegration, IntegrationBinding, IntegrationControl, IntegrationControlAction,
    IntegrationDelivery, IntegrationDetail, IntegrationEvent, IntegrationMode, IntegrationProvider,
    IntegrationSpec, IntegrationsSnapshot, PlatformAccount, PlatformEntry, PlatformInventory,
    ResourceSelector,
};

pub use agena_assistant::{
    ChannelAccessAudit, ChannelAccessPolicy, ChannelGrant, ChannelIdentity, ChannelKind,
    ChannelMember, ChannelOutputRoute, ChannelPolicyUpdate, ChannelRole, ChannelSessionMode,
    ChannelSessionRecord, ChannelThreadMode, ChannelTrigger,
};
