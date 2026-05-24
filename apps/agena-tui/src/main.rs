use agena::AppError;

fn main() -> Result<(), AppError> {
    agena::runtime::build_app_runtime()?.block_on(async_main())
}

async fn async_main() -> Result<(), AppError> {
    agena_tui::run_cli().await
}
