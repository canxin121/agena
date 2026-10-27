//! Credential identities bind stored secrets to the exact configured HTTP endpoint.

use sha2::{Digest, Sha256};
use url::Url;

/// Stable opaque identity for a server and its normalized HTTP endpoint.
/// Paths and query parameters remain part of the identity: changing either
/// requires explicitly configuring credentials for the new endpoint. Existing
/// name-only records are retained but are never adopted automatically.
pub fn credential_store_name(name: &str, endpoint: &Url) -> String {
    let mut endpoint = endpoint.clone();
    endpoint.set_fragment(None);
    let name = name.trim();
    let mut digest = Sha256::new();
    digest.update(b"agena-mcp-endpoint-credentials-v1\0");
    digest.update((name.len() as u64).to_be_bytes());
    digest.update(name.as_bytes());
    digest.update(endpoint.as_str().as_bytes());
    let digest = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("endpoint-v1-{digest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_normalize_urls_and_separate_endpoint_paths_queries_and_names() {
        let canonical = Url::parse("https://example.test/mcp?tenant=a").unwrap();
        let equivalent = Url::parse("https://EXAMPLE.test:443/mcp?tenant=a").unwrap();
        let identity = credential_store_name("团队", &canonical);
        assert_eq!(identity, credential_store_name("团队", &equivalent));
        for endpoint in [
            "https://example.test/other?tenant=a",
            "https://example.test/mcp?tenant=b",
            "http://example.test/mcp?tenant=a",
        ] {
            assert_ne!(
                identity,
                credential_store_name("团队", &Url::parse(endpoint).unwrap())
            );
        }
        assert_ne!(identity, credential_store_name("other", &canonical));
        assert!(!identity.contains("tenant"));
    }
}
