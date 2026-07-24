use std::sync::LazyLock;

pub(crate) fn block_on<F>(future: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        match handle.runtime_flavor() {
            tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(|| handle.block_on(future))
            }
            _ => block_on_fallback_runtime(future),
        }
    } else {
        block_on_fallback_runtime(future)
    }
}

static FALLBACK_RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("agena-tool-fallback")
        .thread_stack_size(32 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("tool fallback runtime")
});

fn block_on_fallback_runtime<F>(future: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(move || FALLBACK_RUNTIME.handle().block_on(future))
            .join()
            .expect("tool fallback runtime thread panicked")
    })
}
