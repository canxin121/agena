//! Validated, ordered HTML engine selection with a string wire format.

use agena_web::WebSearchEngine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(super) enum WebSearchEngineSelection {
    Auto,
    Engines(Vec<WebSearchEngine>),
}

impl TryFrom<String> for WebSearchEngineSelection {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() > 256 {
            return Err("engine must not exceed 256 bytes".into());
        }
        let value = value.trim().to_ascii_lowercase();
        if value == "auto" {
            return Ok(Self::Auto);
        }
        let mut engines = Vec::new();
        for name in value.split(',').map(str::trim) {
            if name.is_empty() {
                return Err(
                    "engine requires a name or comma-separated names, without empty entries".into(),
                );
            }
            if name == "auto" {
                return Err(
                    "engine auto must stand alone; it cannot be combined with other names".into(),
                );
            }
            let engine = WebSearchEngine::ALL
                .into_iter()
                .find(|engine| engine.label() == name)
                .ok_or_else(|| {
                    format!("unknown engine '{name}'; use duckduckgo, bing, baidu, yandex, google, yahoo, brave, naver, or auto alone")
                })?;
            if !engines.contains(&engine) {
                engines.push(engine);
            }
        }
        Ok(Self::Engines(engines))
    }
}

impl From<WebSearchEngineSelection> for String {
    fn from(selection: WebSearchEngineSelection) -> Self {
        selection.label()
    }
}

impl WebSearchEngineSelection {
    pub(super) fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    pub(super) fn engines(&self) -> Vec<WebSearchEngine> {
        match self {
            Self::Auto => WebSearchEngine::ALL.to_vec(),
            Self::Engines(engines) => engines.clone(),
        }
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Auto => "auto".into(),
            Self::Engines(engines) => engines
                .iter()
                .map(|engine| engine.label())
                .collect::<Vec<_>>()
                .join(","),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::CrawlWebSearchInput;
    use super::*;
    use WebSearchEngine::{Baidu, Google};

    #[test]
    fn selection_parses_csv_normalizes_names_and_deduplicates_in_caller_order() {
        let selection: WebSearchEngineSelection =
            serde_json::from_value(serde_json::json!(" GOOGLE, baidu,Google ")).unwrap();
        assert_eq!(selection.engines(), [Google, Baidu]);
        assert_eq!(selection.label(), "google,baidu");
        assert!(!selection.is_auto());
        assert_eq!(serde_json::to_value(selection).unwrap(), "google,baidu");

        for engine in WebSearchEngine::ALL {
            let selection: WebSearchEngineSelection =
                serde_json::from_value(serde_json::json!(engine.label())).unwrap();
            assert_eq!(selection.engines(), [engine]);
            assert_eq!(selection.label(), engine.label());
        }
        let all_names = WebSearchEngine::ALL.map(|engine| engine.label()).join(",");
        let selection = WebSearchEngineSelection::try_from(all_names).unwrap();
        assert_eq!(selection.engines(), WebSearchEngine::ALL);
        assert!(!selection.is_auto());

        for raw in ["auto", " AUTO "] {
            let selection = WebSearchEngineSelection::try_from(raw.to_owned()).unwrap();
            assert!(selection.is_auto());
            assert_eq!(selection.engines(), WebSearchEngine::ALL);
            assert_eq!(selection.label(), "auto");
        }
        for raw in [
            serde_json::json!({"query":"q"}),
            serde_json::json!({"query":"q","engine":null}),
        ] {
            let input: CrawlWebSearchInput = serde_json::from_value(raw).unwrap();
            assert!(input.engine.is_none());
        }
    }

    #[test]
    fn selection_rejects_empty_items_unknown_names_auto_mixtures_and_wrong_types() {
        for (raw, diagnostic) in [
            ("", "empty entries"),
            ("   ", "empty entries"),
            (",google", "empty entries"),
            ("baidu,", "empty entries"),
            ("baidu, ,google", "empty entries"),
            ("baidu,unknown", "unknown engine"),
            ("baidu，google", "unknown engine"),
            ("auto,google", "auto must stand alone"),
            ("baidu,AUTO", "auto must stand alone"),
            ("auto,auto", "auto must stand alone"),
        ] {
            let error = serde_json::from_value::<CrawlWebSearchInput>(
                serde_json::json!({"query":"q","engine":raw}),
            )
            .unwrap_err();
            assert!(error.to_string().contains(diagnostic), "{raw}: {error}");
        }
        for value in [
            serde_json::json!("x".repeat(257)),
            serde_json::json!(["baidu", "google"]),
            serde_json::json!(42),
        ] {
            assert!(
                serde_json::from_value::<CrawlWebSearchInput>(
                    serde_json::json!({"query":"q","engine":value})
                )
                .is_err()
            );
        }
    }
}
