//! Shared server discovery for thin clients hosted by the Agena
//! binary.
//!
//! The endpoint record is only a discovery hint. Every caller must still run
//! the public health handshake before sending workspace or session requests.

pub(crate) fn resolve_server_url(explicit: Option<String>) -> String {
    agena_client::resolve_server_url(explicit.as_deref())
}
