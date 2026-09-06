#[path = "support/panic_isolation.rs"]
mod panic_isolation;

#[tokio::test]
async fn plugin_panics_are_isolated_and_transport_remains_usable() {
    panic_isolation::assert_panic_isolation().await;
}
