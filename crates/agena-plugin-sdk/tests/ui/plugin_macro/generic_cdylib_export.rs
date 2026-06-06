use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct BadPlugin<T> {
    _marker: std::marker::PhantomData<T>,
}

#[plugin(
    id = "bad.generic_cdylib_export",
    version = "1.0.0",
    description = "Bad plugin.",
    export = cdylib
)]
impl<T> BadPlugin<T>
where
    T: Send + Sync + 'static,
{
}

fn main() {}
