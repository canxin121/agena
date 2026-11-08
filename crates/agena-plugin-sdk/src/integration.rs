//! Versioned channel/source ABI over the EXISTING plugin service transport.
//! In-process, cdylib, stdio and HTTP providers use the same three methods.
pub use agena_superagent::{
    AdapterBatch, AdapterDescribeRequest, AdapterDescription, AdapterKind, AdapterPollRequest,
    ChannelDelivery, ChannelEnvelope, ChannelIdentity, ChannelKind, ChannelReceipt, ExternalEvent,
    INTEGRATION_API_VERSION, INTEGRATION_SERVICE, SendDisposition,
};

/// Export the standard adapter service. Source plugins omit send; channels
/// implement it with an honest uncertainty/idempotency contract.
pub fn integration_export(channel: bool) -> crate::PluginServiceExport {
    let export = crate::PluginServiceExport::new(INTEGRATION_SERVICE, INTEGRATION_API_VERSION)
        .with_method(crate::service_method_for::<
            AdapterDescribeRequest,
            AdapterDescription,
        >("describe"))
        .with_method(crate::service_method_for::<AdapterPollRequest, AdapterBatch>("poll"));
    if channel {
        export.with_method(crate::service_method_for::<ChannelDelivery, ChannelReceipt>("send"))
    } else {
        export
    }
}
