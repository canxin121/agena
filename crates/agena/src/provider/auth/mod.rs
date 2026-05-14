mod manager;
mod oauth;
mod store;
mod types;

pub use manager::AuthManager;
pub use oauth::{
    OAuthCallback, exchange_gitlab_oauth_code, exchange_openai_oauth_code,
    poll_copilot_device_code, poll_openai_headless_device_code, refresh_gitlab_token,
    refresh_openai_token, start_copilot_device_code, start_gitlab_oauth,
    start_openai_browser_oauth, start_openai_headless_device_code, wait_for_oauth_callback,
};
pub use store::AuthStore;
pub use types::{
    AuthData, CopilotDeployment, CredentialIssuer, DeviceCodeStart, OAuthAuthorizeStart,
    OAuthTokenResponse,
};
