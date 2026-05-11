use agena::AppError;

#[tokio::main]
async fn main() -> Result<(), AppError> {
    agena_tui::run_compat_cli().await
}
