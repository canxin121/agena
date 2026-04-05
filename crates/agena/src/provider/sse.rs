use async_stream::try_stream;
use futures_core::Stream;
use futures_util::StreamExt;
use serde_json::Value;

use crate::error::AppError;

pub fn json_events(
    response: reqwest::Response,
) -> std::pin::Pin<Box<dyn Stream<Item = Result<Value, AppError>> + Send>> {
    Box::pin(try_stream! {
        let mut buffer = String::new();
        let mut data_lines: Vec<String> = Vec::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            while let Some(idx) = buffer.find('\n') {
                let mut line = buffer[..idx].to_owned();
                buffer = buffer[idx + 1..].to_owned();

                if let Some(stripped) = line.strip_suffix('\r') {
                    line = stripped.to_owned();
                }

                if line.is_empty() {
                    if data_lines.is_empty() {
                        continue;
                    }

                    let payload = data_lines.join("\n");
                    data_lines.clear();

                    if payload.trim() == "[DONE]" {
                        continue;
                    }

                    let value: Value = serde_json::from_str(payload.as_str())
                        .map_err(|e| AppError::Provider(format!("invalid sse json payload: {e}")))?;
                    yield value;
                    continue;
                }

                if let Some(data) = line.strip_prefix("data:") {
                    data_lines.push(data.trim_start().to_owned());
                }
            }
        }

        if !data_lines.is_empty() {
            let payload = data_lines.join("\n");
            if payload.trim() != "[DONE]" {
                let value: Value = serde_json::from_str(payload.as_str())
                    .map_err(|e| AppError::Provider(format!("invalid sse json payload: {e}")))?;
                yield value;
            }
        }
    })
}

#[allow(dead_code)]
pub fn json_lines(
    response: reqwest::Response,
) -> std::pin::Pin<Box<dyn Stream<Item = Result<Value, AppError>> + Send>> {
    Box::pin(try_stream! {
        let mut buffer = String::new();
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            while let Some(idx) = buffer.find('\n') {
                let mut line = buffer[..idx].to_owned();
                buffer = buffer[idx + 1..].to_owned();

                if let Some(stripped) = line.strip_suffix('\r') {
                    line = stripped.to_owned();
                }

                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let value = serde_json::from_str::<Value>(line)
                    .map_err(|e| AppError::Provider(format!("invalid json line payload: {e}")))?;
                yield value;
            }
        }

        let remaining = buffer.trim();
        if !remaining.is_empty() {
            let value = serde_json::from_str::<Value>(remaining)
                .map_err(|e| AppError::Provider(format!("invalid json line payload: {e}")))?;
            yield value;
        }
    })
}
