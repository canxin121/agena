//! `web_search` plugin tool backed by the local crawl index.

use crate::message::WebSearchToolInput;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    WebSearchHit,
};

const DEFAULT_MAX_RESULTS: u32 = 8;
const BACKEND_NAME: &str = "crawl";

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &WebSearchToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let q = input.query.trim();
    if q.is_empty() {
        return Err(ToolError::Plugin(
            "web_search: query must not be empty".to_string(),
        ));
    }

    let max = input
        .max_results
        .unwrap_or(DEFAULT_MAX_RESULTS)
        .clamp(1, 20);
    let allow = input.allowed_domains.clone();
    let block = input.blocked_domains.clone();
    let raw_hits = local_crawl_search(executor, q, max as usize)?;

    let hits: Vec<WebSearchHit> = raw_hits
        .into_iter()
        .filter(|hit| domain_allowed(&hit.url, &allow, &block))
        .take(max as usize)
        .collect();

    let summary = if hits.is_empty() {
        format!(
            "[{}] no local crawl results for {q:?}. Run `crawl` first to build the local index.",
            BACKEND_NAME
        )
    } else {
        let mut buf = format!("[{}] {} result(s):\n", BACKEND_NAME, hits.len());
        for (i, h) in hits.iter().enumerate() {
            use std::fmt::Write as _;
            let _ = writeln!(&mut buf, "  {}. {} — {}", i + 1, h.title, h.url);
        }
        buf
    };

    let view = ToolExecutionView::simple(format!("WebSearch {q:?}"), summary);
    let output = ToolPayloadOutput::WebSearch {
        query: q.to_string(),
        backend: BACKEND_NAME.to_string(),
        results: hits,
    };
    Ok(ToolPayloadExecution::new(output, view))
}

fn local_crawl_search(
    executor: &ToolExecutor,
    query: &str,
    limit: usize,
) -> Result<Vec<WebSearchHit>, ToolError> {
    let store = agena_crawl::CrawlStore::for_workspace(executor.workspace_root());
    if !store.dir().exists() {
        return Ok(Vec::new());
    }
    if !store.dir().join(".index").exists() {
        store
            .rebuild_index()
            .map_err(|err| ToolError::Plugin(format!("web_search[crawl]: {err}")))?;
    }
    let hits = store
        .search(query, limit)
        .map_err(|err| ToolError::Plugin(format!("web_search[crawl]: {err}")))?;
    Ok(hits
        .into_iter()
        .map(|hit| WebSearchHit {
            title: hit.title,
            url: hit.url,
            snippet: Some(hit.preview),
        })
        .collect())
}

fn domain_allowed(url: &str, allow: &[String], block: &[String]) -> bool {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default();
    if !allow.is_empty() && !allow.iter().any(|d| host_matches(&host, d)) {
        return false;
    }
    if block.iter().any(|d| host_matches(&host, d)) {
        return false;
    }
    true
}

fn host_matches(host: &str, pattern: &str) -> bool {
    let h = host.to_ascii_lowercase();
    let p = pattern.to_ascii_lowercase();
    h == p || h.ends_with(&format!(".{p}"))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::tempdir;

    use super::execute;
    use crate::agent::Agent;
    use crate::entry::{ToolExecutor, ToolPayloadOutput};
    use crate::message::WebSearchToolInput;
    use crate::permission::PermissionPolicy;

    #[test]
    fn web_search_uses_local_crawl_index_without_external_search_service() {
        let workspace = tempdir().expect("workspace");
        let store = agena_crawl::CrawlStore::for_workspace(workspace.path());
        let document = agena_crawl::StoredDocument {
            id: "doc-1".to_string(),
            url: "https://example.com/docs".to_string(),
            canonical_url: "https://example.com/docs".to_string(),
            title: "Runtime Web Docs".to_string(),
            markdown: "Runtime web crawl search is fully local.".to_string(),
            chunks: vec!["Runtime web crawl search is fully local.".to_string()],
            chunk_hashes: vec!["chunk-hash".to_string()],
            links: Vec::new(),
            content_type: "text/html".to_string(),
            status: 200,
            truncated: false,
            rendered: false,
            hash: "hash".to_string(),
            raw_html_hash: "raw-hash".to_string(),
            markdown_hash: "markdown-hash".to_string(),
            simhash: 1,
            etag: None,
            last_modified: None,
            depth: 0,
            fetched_at: Utc::now(),
        };
        store.save_document(&document).expect("document saved");
        store.rebuild_index().expect("index rebuilt");

        let executor = ToolExecutor::new(
            workspace.path(),
            Agent::new("test", PermissionPolicy::allow_all()),
        );
        let output = execute(
            &executor,
            &WebSearchToolInput {
                query: "fully local".to_string(),
                allowed_domains: Vec::new(),
                blocked_domains: Vec::new(),
                max_results: Some(5),
            },
        )
        .expect("search succeeds");

        let ToolPayloadOutput::WebSearch {
            backend, results, ..
        } = output.output
        else {
            panic!("expected web search output");
        };
        assert_eq!(backend, "crawl");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Runtime Web Docs");
    }
}
