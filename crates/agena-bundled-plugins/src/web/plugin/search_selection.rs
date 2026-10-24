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
