//! Shared processing-center discovery for thin clients hosted by the Agena
//! binary.
//!
//! The endpoint record is only a discovery hint. Every caller must still run
//! the public health handshake before sending workspace or session requests.

pub(crate) const DEFAULT_CENTER_URL: &str = "http://127.0.0.1:3210";

pub(crate) fn resolve_center_url(explicit: Option<String>) -> String {
    explicit
        .filter(|url| !url.trim().is_empty())
        .or_else(|| {
            let path = crate::server::center_record::record_path();
            crate::server::center_record::read_record(path.as_path())
                .ok()
                .filter(|record| record.protocol_version == agena_api::PROTOCOL_VERSION)
                .map(|record| record.url)
        })
        .unwrap_or_else(|| DEFAULT_CENTER_URL.to_owned())
}
