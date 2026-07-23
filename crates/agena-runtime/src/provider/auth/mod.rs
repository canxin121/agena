mod manager;
mod oauth;
mod store;

pub(crate) use manager::AuthManager;
pub(crate) use oauth::{
    exchange_gitlab_oauth_code, exchange_openai_oauth_code, poll_copilot_device_code,
    poll_openai_headless_device_code, refresh_gitlab_token, refresh_openai_token,
    start_copilot_device_code, start_gitlab_oauth, start_openai_browser_oauth,
    start_openai_headless_device_code,
};
pub(crate) use store::AuthStore;
