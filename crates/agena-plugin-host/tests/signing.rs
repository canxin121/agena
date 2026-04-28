//! sha256 verification + ed25519 signing smoke. We don't try to actually
//! load a plugin; we exercise the verifier against a tiny fixture file and
//! a real ed25519 keypair.

#![cfg(feature = "signing")]

use std::collections::BTreeMap;
use std::io::Write;

use agena_plugin_host::PluginSignature;
use ed25519_dalek::{Signer, SigningKey};

fn temp_path(suffix: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("agena-signing-{}-{}", std::process::id(), suffix));
    path
}

#[test]
fn sha256_verifies_match() {
    let path = temp_path("sha-ok");
    std::fs::write(&path, b"hello").unwrap();
    // sha256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
    let ok = agena_plugin_host::loader::verify_sha256(
        &path,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    );
    assert!(ok.is_ok(), "{ok:?}");
}

#[test]
fn sha256_rejects_mismatch() {
    let path = temp_path("sha-bad");
    std::fs::write(&path, b"hello").unwrap();
    let bad = agena_plugin_host::loader::verify_sha256(&path, "deadbeef".repeat(8).as_str());
    assert!(bad.is_err());
}

#[test]
fn ed25519_round_trip() {
    let signing_key = SigningKey::from_bytes(&[7u8; 32]);
    let verifying = signing_key.verifying_key();
    let path = temp_path("sig");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(b"plugin-bytes").unwrap();
    let sig = signing_key.sign(b"plugin-bytes");

    let mut keys: BTreeMap<String, String> = BTreeMap::new();
    keys.insert("k1".into(), hex::encode(verifying.to_bytes()));

    let pk_sig = PluginSignature {
        key_id: "k1".into(),
        signature: hex::encode(sig.to_bytes()),
    };
    agena_plugin_host::loader::verify_signature(&path, &pk_sig, &keys)
        .expect("signature should verify");

    let bad_sig = PluginSignature {
        key_id: "k1".into(),
        signature: hex::encode([0u8; 64]),
    };
    assert!(
        agena_plugin_host::loader::verify_signature(&path, &bad_sig, &keys).is_err(),
        "tampered signature must be rejected"
    );
}
