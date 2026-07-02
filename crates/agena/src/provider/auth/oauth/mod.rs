mod callback;
mod copilot;
mod gitlab;
mod openai;
mod shared;

pub use callback::{OAuthCallback, parse_oauth_callback_url, wait_for_oauth_callback};
pub use copilot::{poll_copilot_device_code, start_copilot_device_code};
pub use gitlab::{exchange_gitlab_oauth_code, refresh_gitlab_token, start_gitlab_oauth};
pub use openai::{
    exchange_openai_oauth_code, poll_openai_headless_device_code, refresh_openai_token,
    revoke_openai_token, start_openai_browser_oauth, start_openai_headless_device_code,
};
