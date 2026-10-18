//! Explicit bounded public projections for real-provider probes.
use agena_api::{live::ToolDetailSection, part::PartResource, queries::ReadPartsParams};
use agena_application::Application;
use std::collections::BTreeMap;

pub async fn parts(
    application: &Application,
    session_id: i64,
) -> anyhow::Result<Vec<PartResource>> {
    let mut cursor = None;
    let mut pages = Vec::new();
    let mut count = 0usize;
    loop {
        let page = application
            .read_parts(ReadPartsParams {
                session_id,
                cursor: cursor.clone(),
                limit: Some(256),
                sections: ToolDetailSection::ALL.to_vec(),
                ..Default::default()
            })
            .await?;
        anyhow::ensure!(
            count.saturating_add(page.parts.len()) <= 65_536,
            "probe transcript exceeds its export limit"
        );
        count += page.parts.len();
        pages.push(page.parts);
        if !page.page.has_more {
            break;
        }
        anyhow::ensure!(
            page.page.next_cursor.is_some() && page.page.next_cursor != cursor,
            "history cursor did not advance"
        );
        cursor = page.page.next_cursor;
    }
    Ok(pages.into_iter().rev().flatten().collect())
}

pub async fn text(
    application: &Application,
    session_id: i64,
    parts: &[PartResource],
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
                let mut after = None;
                loop {
                    let page = application
                        .read_content(session_id, reference.resource_id, after, 64 * 1024)
                        .await?;
                    anyhow::ensure!(
                        !page.gap
                            && page.resource.dropped_packets == 0
                            && page.resource.dropped_bytes == 0,
                        "probe output was incomplete"
                    );
                    for chunk in page.chunks {
                        if let Some(body) = chunk.payload.text_content() {
                            text.push_str(body);
                        }
                    }
                    anyhow::ensure!(
                        text.len() <= 16 * 1024 * 1024,
                        "probe output exceeds its export limit"
                    );
                    if !page.has_more {
                        break;
                    }
                    anyhow::ensure!(
                        Some(page.next_cursor) != after,
                        "content cursor did not advance"
                    );
                    after = Some(page.next_cursor);
                }
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
