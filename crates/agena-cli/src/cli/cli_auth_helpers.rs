pub(super) fn review_prompt(base: &str) -> String {
    format!(
        "Review the current workspace changes against `{base}`. Focus on correctness, regressions, security issues, and missing tests. Report findings first, then concise remediation guidance."
    )
}

pub(super) fn normalize_login_provider(provider_id: &str) -> String {
    provider_id.trim_end_matches('/').to_owned()
}

pub(super) fn browser_login_redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}/auth/callback")
}

pub(super) fn prompt_browser_login(authorize_url: &str) -> Result<(), AppError> {
    println!("open this URL to continue: {authorize_url}");
    io::stdout().flush()?;
    Ok(())
}

pub(super) fn prompt_device_login(start: &AuthDeviceStartResource) -> Result<(), AppError> {
    println!("open this URL: {}", start.verification_url);
    println!("enter code: {}", start.user_code);
    io::stdout().flush()?;
    Ok(())
}
use std::io::{self, Write as _};

use agena_application::dto::AuthDeviceStartResource;

use super::AppError;
