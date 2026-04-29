use agena_rollout::{
    RolloutKind, RolloutReader, RolloutRecorder, SessionMeta, frame::RolloutFrame,
};
use serde_json::json;
use tempfile::tempdir;

#[tokio::test]
async fn round_trip_session() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sess.jsonl");

    let recorder = RolloutRecorder::open(&path).await.unwrap();
    recorder
        .append(RolloutKind::SessionMeta(SessionMeta {
            session_id: "abc".into(),
            agena_version: "test".into(),
            context: json!({"model": "gpt-test"}),
        }))
        .await
        .unwrap();
    recorder
        .append(RolloutKind::UserMessage {
            parts: json!([{"text": "hi"}]),
        })
        .await
        .unwrap();
    recorder
        .append(RolloutKind::AssistantMessage {
            parts: json!([{"text": "hello"}]),
        })
        .await
        .unwrap();

    let reader = RolloutReader::open(&path);
    let frames: Vec<RolloutFrame> = reader.read_all().unwrap();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].seq, 1);
    assert_eq!(frames[2].seq, 3);
    let meta = reader.session_meta().unwrap();
    assert_eq!(meta.session_id, "abc");
}

#[tokio::test]
async fn resume_seq_after_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sess.jsonl");

    let r1 = RolloutRecorder::open(&path).await.unwrap();
    for i in 0..3 {
        r1.append(RolloutKind::UserMessage {
            parts: json!([{"i": i}]),
        })
        .await
        .unwrap();
    }
    drop(r1);

    let r2 = RolloutRecorder::open(&path).await.unwrap();
    let seq = r2
        .append(RolloutKind::UserMessage { parts: json!([]) })
        .await
        .unwrap();
    assert_eq!(seq, 4);
}
