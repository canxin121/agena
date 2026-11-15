use super::*;

fn transfer() -> PreparedMediaTransfer {
    from_bytes("notes.txt", b"prepared in worker\n".to_vec(), None)
        .unwrap()
        .into()
}

#[test]
fn sandbox_media_roundtrip_retains_original_bytes_without_changing_public_metadata() {
    let transfer = transfer();
    let encoded = serde_json::to_vec(&transfer).unwrap();
    let decoded: PreparedMediaTransfer = serde_json::from_slice(&encoded).unwrap();
    let prepared = decoded.into_prepared().unwrap();
    assert_eq!(prepared.bytes, b"prepared in worker\n");
    assert_eq!(prepared.size_bytes, prepared.bytes.len() as u64);
    assert_eq!(prepared.kind, AttachmentKind::File);
    let public = serde_json::to_value(prepared).unwrap();
    assert!(public.get("bytes").is_none());
    assert!(public.get("bytes_base64").is_none());
}

#[test]
fn sandbox_media_rejects_mismatched_hash_and_byte_count() {
    let mut data = transfer();
    data.sha256 = "0".repeat(64);
    assert!(data.into_prepared().unwrap_err().contains("SHA-256"));
    let mut data = transfer();
    data.size_bytes += 1;
    assert!(data.into_prepared().unwrap_err().contains("byte count"));
    let mut data = transfer();
    data.size_bytes = MAX_MEDIA_BYTES as u64 + 1;
    assert!(data.into_prepared().unwrap_err().contains("20 MiB"));
}

#[test]
fn sandbox_media_rejects_untrusted_metadata_before_parent_binding() {
    let mut data = transfer();
    data.filename = "../outside.txt".into();
    assert!(data.into_prepared().unwrap_err().contains("filename"));
    let mut data = transfer();
    data.kind = AttachmentKind::Image;
    assert!(data.into_prepared().unwrap_err().contains("MIME"));
    let mut data = transfer();
    data.kind = AttachmentKind::Image;
    data.mime = "image/png".into();
    data.width = Some(20_000);
    data.height = Some(20_000);
    assert!(data.into_prepared().unwrap_err().contains("dimensions"));
}
