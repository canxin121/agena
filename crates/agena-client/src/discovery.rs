//! Local discovery is a hint; callers must still validate the health handshake.

pub fn resolve_server_url(explicit: Option<&str>) -> String {
    if let Some(url) = explicit.filter(|url| !url.trim().is_empty()) {
        return url.to_owned();
    }
    let path = agena_storage_layout::StorageLayout::current()
        .run_dir()
        .join("server.json");
    std::fs::read(path)
        .ok()
        .and_then(|bytes| {
            serde_json::from_slice::<agena_api::resource::ServerEndpointRecord>(&bytes).ok()
        })
        .filter(|record| {
            record.schema == agena_api::resource::ServerEndpointRecord::SCHEMA
                && record.protocol_version == agena_api::PROTOCOL_VERSION
        })
        .map(|record| record.url)
        .unwrap_or_else(|| "http://127.0.0.1:3210".to_owned())
}
