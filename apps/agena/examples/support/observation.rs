//! Bounded public transcript observations for live gateway examples.
use agena_api::{live::ToolDetailSection, part::PartResource, queries::ReadPartsParams};
use agena_application::Application;

pub async fn parts(application: &Application, session: i64) -> anyhow::Result<Vec<PartResource>> {
    let mut pages = Vec::new();
    let mut count = 0usize;
    let mut cursor = None;
    loop {
        let page = application
            .read_parts(ReadPartsParams {
                session_id: session,
                cursor: cursor.clone(),
                limit: Some(200),
                sections: ToolDetailSection::ALL.to_vec(),
                ..Default::default()
            })
            .await?;
        anyhow::ensure!(
            count + page.parts.len() <= 65_536,
            "probe history exceeds its observation budget"
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
    session: i64,
    part: &PartResource,
) -> anyhow::Result<String> {
    let mut result = part.content["text"].as_str().unwrap_or_default().to_owned();
    if let Some(resources) = part.content["resources"].as_array() {
        for value in resources {
            let reference: agena_domain::ContentRef = serde_json::from_value(value.clone())?;
            let mut after = None;
            loop {
                let page = application
                    .read_content(session, reference.resource_id, after, 64 * 1024)
                    .await?;
                anyhow::ensure!(
                    !page.gap
                        && page.resource.dropped_bytes == 0
                        && page.resource.dropped_packets == 0,
                    "probe content is incomplete"
                );
                for chunk in page.chunks {
                    if let Some(text) = chunk.payload.text_content() {
                        result.push_str(text);
                    }
                }
                anyhow::ensure!(
                    result.len() <= 16 * 1024 * 1024,
                    "probe output exceeds observation budget"
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
    Ok(result)
}
