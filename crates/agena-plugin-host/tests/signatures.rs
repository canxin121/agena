#![cfg(feature = "signing")]

use std::collections::BTreeMap;

use agena_plugin_host::{PluginSignature, verify_signature_bytes};
use ed25519_dalek::{Signer, SigningKey};

#[test]
fn plugin_signatures_bind_the_artifact_to_the_trusted_key() {
    let key = SigningKey::from_bytes(&[0x42; 32]);
    let bytes = b"plugin artifact";
    let signature = PluginSignature {
        key_id: "publisher".to_string(),
        signature: hex::encode(key.sign(bytes).to_bytes()),
    };
    let mut trusted = BTreeMap::from([(
        signature.key_id.clone(),
        hex::encode(key.verifying_key().as_bytes()),
    )]);
    verify_signature_bytes(bytes, &signature, &trusted).expect("valid publisher signature");
    assert!(verify_signature_bytes(b"changed artifact", &signature, &trusted).is_err());
    trusted.insert(
        signature.key_id.clone(),
        hex::encode(
            SigningKey::from_bytes(&[0x24; 32])
                .verifying_key()
                .as_bytes(),
        ),
    );
    assert!(verify_signature_bytes(bytes, &signature, &trusted).is_err());
    assert!(verify_signature_bytes(bytes, &signature, &BTreeMap::new()).is_err());
}

#[test]
fn plugin_signatures_reject_weak_keys_that_accept_arbitrary_artifacts() {
    // The Edwards identity is a valid encoded point but is not a secure
    // publisher key. With identity R and zero S, a relaxed verifier accepts
    // any message without possession of a private key.
    let mut identity = [0u8; 32];
    identity[0] = 1;
    let mut forged = [0u8; 64];
    forged[0] = 1;
    let signature = PluginSignature {
        key_id: "weak-key".to_string(),
        signature: hex::encode(forged),
    };
    let trusted = BTreeMap::from([(signature.key_id.clone(), hex::encode(identity))]);
    assert!(verify_signature_bytes(b"arbitrary artifact", &signature, &trusted).is_err());
}
