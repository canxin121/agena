//! OpenTelemetry integration: configures `tracing-subscriber` with an
//! optional OTLP HTTP exporter and exposes `TelemetryConfig` for the
//! main `agena` config layer.
//!
//! When `telemetry.enabled = false` (the default) only the local fmt
//! layer is installed; turning it on adds an OTLP tracer.

use std::collections::BTreeMap;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    Resource,
    trace::{RandomIdGenerator, Sampler, SdkTracer, SdkTracerProvider},
};
use serde::{Deserialize, Serialize};
use tracing::Subscriber;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::LookupSpan;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub service_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub otlp_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            service_name: "agena".to_string(),
            otlp_endpoint: None,
            headers: BTreeMap::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("failed to build OTLP span exporter: {0}")]
    Exporter(String),
}

pub struct TelemetrySubscriber {
    tracer: SdkTracer,
    pub guard: TelemetryGuard,
}

impl TelemetrySubscriber {
    pub fn layer<S>(&self) -> OpenTelemetryLayer<S, SdkTracer>
    where
        S: Subscriber + for<'span> LookupSpan<'span>,
    {
        OpenTelemetryLayer::new(self.tracer.clone())
    }
}

pub struct TelemetryGuard {
    provider: SdkTracerProvider,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        let _ = self.provider.shutdown();
    }
}

pub fn build_layer(
    config: &TelemetryConfig,
) -> Result<Option<TelemetrySubscriber>, TelemetryError> {
    if !config.enabled {
        return Ok(None);
    }

    let mut exporter = opentelemetry_otlp::SpanExporter::builder().with_http();
    if let Some(endpoint) = config
        .otlp_endpoint
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        exporter = exporter.with_endpoint(endpoint.to_string());
    }
    if !config.headers.is_empty() {
        exporter = exporter.with_headers(config.headers.clone().into_iter().collect());
    }

    let exporter = exporter
        .build()
        .map_err(|error| TelemetryError::Exporter(error.to_string()))?;
    let provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)))
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(
            Resource::builder()
                .with_service_name(config.service_name.clone())
                .build(),
        )
        .with_batch_exporter(exporter)
        .build();
    let tracer = provider.tracer(config.service_name.clone());

    Ok(Some(TelemetrySubscriber {
        tracer,
        guard: TelemetryGuard { provider },
    }))
}
