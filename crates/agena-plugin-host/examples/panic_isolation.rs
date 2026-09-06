//! Run with `cargo run --locked -p agena-plugin-host --profile dist --example panic_isolation`.
//! Cargo's test harness forces unwinding, so a normal executable is required
//! to verify that the shipping profile preserves plugin panic recovery.

#[path = "../tests/support/panic_isolation.rs"]
mod panic_isolation;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    panic_isolation::assert_panic_isolation().await;
    println!("synchronous and asynchronous plugin panic isolation passed");
}
