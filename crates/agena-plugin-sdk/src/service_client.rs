//! Typed consumer/provider helpers for declared cross-plugin service seams.
//!
//! These helpers deliberately do not discover providers. A plugin manifest
//! declares the import, the Host resolves exactly one provider, and this client
//! only supplies the service id/API version/method plus typed payload.

use std::sync::Arc;

use serde::{Serialize, de::DeserializeOwned};

use crate::{
    HostClient, JsonSchema, PluginError, PluginServiceImport, PluginServiceInvokeInput,
    PluginServiceMethod, Result,
};

/// One shared, typed cross-plugin service endpoint.
///
/// Put the implementation in an API crate that both providers and consumers
/// depend on. The endpoint then becomes the single source of truth for the
/// service id, API version, method id, request type, and response type.
pub trait PluginServiceEndpoint: Send + Sync + 'static {
    type Input: Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static;
    type Output: Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static;

    const SERVICE: &'static str;
    const API_VERSION: u32;
    const METHOD: &'static str;

    fn method_contract() -> PluginServiceMethod {
        crate::service_method_for::<Self::Input, Self::Output>(Self::METHOD)
    }

    fn required_import() -> PluginServiceImport {
        PluginServiceImport::required(Self::SERVICE, Self::API_VERSION)
    }

    fn optional_import() -> PluginServiceImport {
        PluginServiceImport::optional(Self::SERVICE, Self::API_VERSION)
    }
}

#[derive(Clone)]
pub struct PluginServiceEndpointClient<E> {
    inner: PluginServiceClient,
    marker: std::marker::PhantomData<fn() -> E>,
}

impl<E> std::fmt::Debug for PluginServiceEndpointClient<E>
where
    E: PluginServiceEndpoint,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginServiceEndpointClient")
            .field("service", &E::SERVICE)
            .field("api_version", &E::API_VERSION)
            .field("method", &E::METHOD)
            .finish_non_exhaustive()
    }
}

impl<E> PluginServiceEndpointClient<E>
where
    E: PluginServiceEndpoint,
{
    pub fn new(host: Arc<dyn HostClient>) -> Self {
        Self {
            inner: PluginServiceClient::new(host, E::SERVICE, E::API_VERSION),
            marker: std::marker::PhantomData,
        }
    }

    pub async fn call(&self, input: &E::Input) -> Result<PluginServiceResponse<E::Output>> {
        self.inner.call(E::METHOD, input).await
    }

    pub fn service(&self) -> &'static str {
        E::SERVICE
    }

    pub fn api_version(&self) -> u32 {
        E::API_VERSION
    }

    pub fn method(&self) -> &'static str {
        E::METHOD
    }
}

/// Declare a zero-sized typed service endpoint in one shared API module.
///
/// ```ignore
/// plugin_service_endpoint! {
///     pub SearchQuery {
///         service: "workspace.search",
///         version: 1,
///         method: "query",
///         input: SearchRequest,
///         output: SearchResponse,
///     }
/// }
/// ```
#[macro_export]
macro_rules! plugin_service_endpoint {
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident {
            service: $service:literal,
            version: $version:expr,
            method: $method:literal,
            input: $input:ty,
            output: $output:ty $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default)]
        $vis struct $name;

        const _: () = assert!(
            $version > 0,
            "plugin service endpoint version must be positive",
        );

        impl $crate::PluginServiceEndpoint for $name {
            type Input = $input;
            type Output = $output;

            const SERVICE: &'static str = $service;
            const API_VERSION: u32 = $version;
            const METHOD: &'static str = $method;
        }

        impl $name {
            pub fn required_import() -> $crate::PluginServiceImport {
                <Self as $crate::PluginServiceEndpoint>::required_import()
            }

            pub fn optional_import() -> $crate::PluginServiceImport {
                <Self as $crate::PluginServiceEndpoint>::optional_import()
            }

            pub fn method_contract() -> $crate::PluginServiceMethod {
                <Self as $crate::PluginServiceEndpoint>::method_contract()
            }
        }
    };
}

#[derive(Clone)]
pub struct PluginServiceClient {
    host: Arc<dyn HostClient>,
    service: String,
    api_version: u32,
}

impl std::fmt::Debug for PluginServiceClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginServiceClient")
            .field("service", &self.service)
            .field("api_version", &self.api_version)
            .finish_non_exhaustive()
    }
}

impl PluginServiceClient {
    pub fn new(host: Arc<dyn HostClient>, service: impl Into<String>, api_version: u32) -> Self {
        Self {
            host,
            service: service.into(),
            api_version,
        }
    }

    pub fn service(&self) -> &str {
        &self.service
    }

    pub fn api_version(&self) -> u32 {
        self.api_version
    }

    pub fn endpoint<E>(host: Arc<dyn HostClient>) -> PluginServiceEndpointClient<E>
    where
        E: PluginServiceEndpoint,
    {
        PluginServiceEndpointClient::new(host)
    }

    /// Call a declared method and decode the validated provider result into the
    /// Rust output type. Provider selection remains immutable and host-owned.
    pub async fn call<I, O>(
        &self,
        method: impl Into<String>,
        input: &I,
    ) -> Result<PluginServiceResponse<O>>
    where
        I: Serialize + Sync,
        O: DeserializeOwned,
    {
        let method = method.into();
        let input = serde_json::to_value(input).map_err(|error| {
            PluginError::invalid_params(format!(
                "failed to encode service `{}@v{}::{method}` input: {error}",
                self.service, self.api_version
            ))
        })?;
        let response = self
            .host
            .invoke_service(PluginServiceInvokeInput {
                service: self.service.clone(),
                api_version: self.api_version,
                method: method.clone(),
                input,
            })
            .await?;
        let output = serde_json::from_value(response.output).map_err(|error| {
            PluginError::internal(format!(
                "validated service `{}@v{}::{method}` output could not decode into the requested Rust type: {error}",
                self.service, self.api_version
            ))
        })?;
        Ok(PluginServiceResponse {
            provider: response.provider,
            output,
        })
    }

    pub async fn call_value(
        &self,
        method: impl Into<String>,
        input: serde_json::Value,
    ) -> Result<crate::PluginServiceInvokeOutput> {
        self.host
            .invoke_service(PluginServiceInvokeInput {
                service: self.service.clone(),
                api_version: self.api_version,
                method: method.into(),
                input,
            })
            .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginServiceResponse<T> {
    pub provider: String,
    pub output: T,
}

/// Provider-side typed input decoding after the Host has already validated the
/// wire method contract. Keeping this helper in the SDK prevents every service
/// provider from hand-writing slightly different serde diagnostics.
pub trait PluginServiceInvokeExt {
    fn decode<T>(&self) -> Result<T>
    where
        T: DeserializeOwned;
}

impl PluginServiceInvokeExt for PluginServiceInvokeInput {
    fn decode<T>(&self) -> Result<T>
    where
        T: DeserializeOwned,
    {
        serde_json::from_value(self.input.clone()).map_err(|error| {
            PluginError::invalid_params(format!(
                "failed to decode service `{}@v{}::{}` input: {error}",
                self.service, self.api_version, self.method
            ))
        })
    }
}

pub fn encode_service_output<T>(value: T) -> Result<serde_json::Value>
where
    T: Serialize,
{
    serde_json::to_value(value)
        .map_err(|error| PluginError::internal(format!("failed to encode service output: {error}")))
}
