//! Explicit factual history reads and temporary content projections for probes.
use agena_storage::store::{Part, PartCursor, SessionStore};
use std::collections::BTreeMap;

pub async fn parts(store: &dyn SessionStore, session_id: i64) -> anyhow::Result<Vec<Part>> {
    let mut before = None;
    let mut parts = Vec::new();
    loop {
        let page = store.load_page(session_id, before, 256).await?;
        let next = page.parts.last().map(|p| PartCursor {
            created_at_ms: p.created_at_ms,
            part_id: p.part_id,
        });
        let more = page.has_more;
        parts.extend(page.parts);
        if !more {
            break;
        }
        anyhow::ensure!(
            next.is_some() && next != before,
            "history cursor did not advance"
        );
        before = next;
    }
    parts.sort_unstable_by_key(|p| (p.created_at_ms, p.part_id));
    Ok(parts)
}

pub async fn text(
    store: &dyn SessionStore,
    parts: &[Part],
) -> anyhow::Result<BTreeMap<i64, String>> {
    let mut result = BTreeMap::new();
    for part in parts {
        let mut text = part
            .content
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if let Some(resources) = part
            .content
            .get("resources")
            .and_then(serde_json::Value::as_array)
        {
            for reference in resources {
                let reference: agena_domain::ContentRef =
                    serde_json::from_value(reference.clone())?;
                let body = store
                    .contents()
                    .read_text(reference.resource_id, 1024 * 1024)
                    .await?;
                anyhow::ensure!(!body.gap && !body.truncated, "probe output was incomplete");
                text.push_str(&body.text);
            }
        }
        if part.kind == "tool_call" {
            let value =
                agena_runtime_contracts::part_content::ToolCallContent::try_from(&part.content)
                    .map_err(anyhow::Error::msg)?;
            if let Some(output) = value.output {
                text.push_str(output.text_content());
            }
        }
        result.insert(part.part_id, text);
    }
    Ok(result)
}
