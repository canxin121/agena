//! Follow vendor model cursors on the configured endpoint, preserving its
//! origin and custom query parameters. Never follow response-provided URLs.

use std::collections::BTreeSet;

use serde_json::Value;
use url::Url;

use crate::ProviderError;

pub(crate) struct ModelPagination {
    endpoint: Url,
    seen: BTreeSet<String>,
}

impl ModelPagination {
    pub(crate) fn new(endpoint: &str) -> Result<Self, ProviderError> {
        Ok(Self {
            endpoint: Url::parse(endpoint)
                .map_err(|_| ProviderError::Config("invalid model discovery endpoint".into()))?,
            seen: BTreeSet::new(),
        })
    }

    pub(crate) fn next(&mut self, page: &Value) -> Result<Option<String>, ProviderError> {
        let nonempty = |value: &Value| value.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
        let cursor = if let Some(token) = nonempty(&page["nextPageToken"]) {
            Some(("pageToken", token))
        } else if page["has_more"].as_bool() == Some(true) {
            let id = nonempty(&page["last_id"])
                .or_else(|| {
                    page["data"]
                        .as_array()
                        .and_then(|models| models.last())
                        .and_then(|model| nonempty(&model["id"]))
                })
                .ok_or_else(|| {
                    ProviderError::Provider("model list has_more without a continuation ID".into())
                })?;
            Some(("after", id))
        } else {
            None
        };
        let Some((parameter, value)) = cursor else {
            return Ok(None);
        };
        if self.seen.len() >= 99 {
            return Err(ProviderError::Provider(
                "model discovery exceeded 100 pages".into(),
            ));
        }
        if !self.seen.insert(format!("{parameter}:{value}")) {
            return Err(ProviderError::Provider(
                "model discovery repeated a pagination cursor".into(),
            ));
        }
        let mut next = self.endpoint.clone();
        let query = next
            .query_pairs()
            .filter(|(key, _)| key != parameter)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();
        next.set_query(None);
        next.query_pairs_mut()
            .extend_pairs(query)
            .append_pair(parameter, &value);
        Ok(Some(next.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vendor_cursors_preserve_custom_endpoint_and_never_change_origin() {
        let mut pagination =
            ModelPagination::new("https://gateway.example/custom/models?tenant=a").unwrap();
        let next = pagination
            .next(&json!({"has_more":true,"last_id":"model/second?x=1"}))
            .unwrap()
            .unwrap();
        let next = Url::parse(&next).unwrap();
        assert_eq!(next.host_str(), Some("gateway.example"));
        assert_eq!(next.path(), "/custom/models");
        assert!(next.query_pairs().any(|(k, v)| k == "tenant" && v == "a"));
        assert!(
            next.query_pairs()
                .any(|(k, v)| k == "after" && v == "model/second?x=1")
        );
        assert!(
            pagination
                .next(&json!({"has_more":true,"last_id":"model/second?x=1"}))
                .is_err()
        );
        assert!(
            pagination
                .next(&json!({"has_more":true,"data":[]}))
                .is_err()
        );
    }

    #[test]
    fn google_tokens_and_unpaginated_responses_are_supported() {
        let mut pagination =
            ModelPagination::new("https://gateway.example/models?pageToken=old").unwrap();
        let next = pagination
            .next(&json!({"nextPageToken":"new token"}))
            .unwrap()
            .unwrap();
        let next = Url::parse(&next).unwrap();
        assert_eq!(
            next.query_pairs()
                .filter(|(k, _)| k == "pageToken")
                .collect::<Vec<_>>(),
            vec![("pageToken".into(), "new token".into())]
        );
        assert!(pagination.next(&json!({"models":[]})).unwrap().is_none());
        assert!(pagination.next(&json!([])).unwrap().is_none());
    }
}
