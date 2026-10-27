//! Per-operation network authorization for the OAuth SDK's outbound requests.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rmcp::transport::auth::{
    OAuthHttpClient, OAuthHttpClientError, OAuthHttpClientFuture, OAuthHttpRedirectPolicy,
    OAuthHttpRequest,
};

use crate::{McpError, McpResult};

/// The active host callback authorizes the real destination before OAuth
/// discovery, registration, code exchange, or refresh performs any network I/O.
#[async_trait]
pub trait McpOAuthNetworkAuthorizer: Send + Sync {
    async fn authorize(&self, target: &str) -> McpResult<()>;
}

#[derive(Default)]
struct AuthorizationState {
    authorizer: Option<Arc<dyn McpOAuthNetworkAuthorizer>>,
    failure: Option<McpError>,
}

#[derive(Default)]
pub(crate) struct OAuthAuthorizationSlot(Mutex<AuthorizationState>);

impl OAuthAuthorizationSlot {
    pub(crate) fn install(
        self: &Arc<Self>,
        authorizer: Option<Arc<dyn McpOAuthNetworkAuthorizer>>,
    ) -> OAuthAuthorizationScope {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = AuthorizationState {
            authorizer,
            failure: None,
        };
        OAuthAuthorizationScope(Arc::clone(self))
    }

    pub(crate) fn take_failure(&self) -> Option<McpError> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .failure
            .take()
    }

    async fn require(&self, target: &str) -> Result<(), OAuthHttpClientError> {
        let authorizer = {
            let state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            if state.failure.is_some() {
                return Err(Box::new(McpError::PermissionDenied));
            }
            state.authorizer.clone()
        };
        if let Some(authorizer) = authorizer
            && let Err(error) = authorizer.authorize(target).await
        {
            self.0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .failure = Some(error);
            return Err(Box::new(McpError::PermissionDenied));
        }
        Ok(())
    }
}

pub(crate) struct OAuthAuthorizationScope(Arc<OAuthAuthorizationSlot>);
impl Drop for OAuthAuthorizationScope {
    fn drop(&mut self) {
        // Pending PKCE state may survive this call, but callback authority must
        // not. Completion installs its own live authority (or trusted UI mode).
        *self.0.0.lock().unwrap_or_else(|error| error.into_inner()) = AuthorizationState::default();
    }
}

pub(crate) struct AuthorizedOAuthHttpClient {
    client: reqwest::Client,
    authorization: Arc<OAuthAuthorizationSlot>,
}

impl AuthorizedOAuthHttpClient {
    pub(crate) fn new(authorization: Arc<OAuthAuthorizationSlot>) -> McpResult<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| McpError::Auth("Could not create the OAuth HTTP client.".into()))?;
        Ok(Self {
            client,
            authorization,
        })
    }
}

fn http_error(message: &str) -> OAuthHttpClientError {
    Box::new(McpError::Auth(message.to_owned()))
}

impl OAuthHttpClient for AuthorizedOAuthHttpClient {
    fn execute(&self, operation: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move {
            let follows_redirects =
                matches!(operation.redirect_policy, OAuthHttpRedirectPolicy::Follow);
            let mut request = reqwest::Request::try_from(operation.request)
                .map_err(|_| http_error("Could not construct the OAuth HTTP request."))?;
            *request.timeout_mut() = Some(
                operation
                    .timeout
                    .unwrap_or(Duration::from_secs(30))
                    .min(Duration::from_secs(30)),
            );
            for redirects in 0..=10 {
                self.authorization.require(request.url().as_str()).await?;
                let outgoing = request
                    .try_clone()
                    .ok_or_else(|| http_error("Could not replay the OAuth HTTP request."))?;
                let mut response = self
                    .client
                    .execute(outgoing)
                    .await
                    .map_err(|_| http_error("The OAuth HTTP request failed."))?;
                let status = response.status();
                if follows_redirects && matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
                    if redirects == 10 {
                        return Err(http_error("The OAuth server returned too many redirects."));
                    }
                    let location = response
                        .headers()
                        .get(reqwest::header::LOCATION)
                        .and_then(|value| value.to_str().ok())
                        .ok_or_else(|| {
                            http_error("The OAuth redirect is missing a valid location.")
                        })?;
                    let next = request
                        .url()
                        .join(location)
                        .map_err(|_| http_error("The OAuth redirect URL is invalid."))?;
                    if !matches!(next.scheme(), "http" | "https")
                        || !next.username().is_empty()
                        || next.password().is_some()
                    {
                        return Err(http_error(
                            "The OAuth redirect URL is not an HTTP(S) endpoint.",
                        ));
                    }
                    if next.origin() != request.url().origin() {
                        let contains_body = request.body().is_some_and(|body| {
                            body.as_bytes().is_none_or(|bytes| !bytes.is_empty())
                        });
                        if request
                            .headers()
                            .contains_key(reqwest::header::AUTHORIZATION)
                            || contains_body
                        {
                            return Err(http_error(
                                "A credential-bearing OAuth request cannot redirect to another origin.",
                            ));
                        }
                    }
                    if status.as_u16() == 303
                        || (matches!(status.as_u16(), 301 | 302)
                            && request.method() == reqwest::Method::POST)
                    {
                        *request.method_mut() = reqwest::Method::GET;
                        *request.body_mut() = None;
                        request.headers_mut().remove(reqwest::header::CONTENT_TYPE);
                        request
                            .headers_mut()
                            .remove(reqwest::header::CONTENT_LENGTH);
                    }
                    request.headers_mut().remove(reqwest::header::HOST);
                    *request.url_mut() = next;
                    continue;
                }
                let mut builder = oauth2::http::Response::builder()
                    .status(status)
                    .version(response.version());
                for (name, value) in response.headers() {
                    builder = builder.header(name, value);
                }
                let mut body = Vec::new();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|_| http_error("Could not read the OAuth HTTP response."))?
                {
                    if chunk.len() > (1024 * 1024_usize).saturating_sub(body.len()) {
                        return Err(http_error("The OAuth HTTP response is larger than 1 MiB."));
                    }
                    body.extend_from_slice(&chunk);
                }
                return builder
                    .body(body)
                    .map_err(|_| http_error("Could not decode the OAuth HTTP response."));
            }
            Err(http_error("The OAuth server returned too many redirects."))
        })
    }
}
