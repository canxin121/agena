//! Batch persistence for content-addressed blocks of frozen input facts.

use std::collections::HashMap;

use serde::Serialize;

use super::*;

pub(super) const IDS_PER_BLOCK: i64 = 64;

pub(super) fn bucket(id: i64) -> i64 {
    (id - 1) / IDS_PER_BLOCK
}

pub(super) struct Block<'a, Fact, Metadata> {
    pub round: i64,
    pub position: i64,
    pub facts: &'a [Fact],
    pub metadata: Metadata,
}

pub(super) struct Statements {
    pub insert_blocks: &'static str,
    pub insert_facts: &'static str,
    pub insert_links: &'static str,
}

pub(super) async fn persist<'a, Fact: Serialize + 'a, Metadata: Serialize>(
    connection: &mut SqliteConnection,
    blocks: impl Iterator<Item = Block<'a, Fact, Metadata>>,
    statements: &Statements,
) -> Result<()> {
    let blocks = blocks
        .map(|block| {
            let digest = Sha256::digest(encode(block.facts)?);
            Ok((block, hex::encode(digest)))
        })
        .collect::<Result<Vec<_>>>()?;
    if blocks.is_empty() {
        return Ok(());
    }
    let links = blocks
        .iter()
        .map(|(block, digest)| (block.round, block.position, digest, &block.metadata))
        .collect::<Vec<_>>();
    let links = encode(&links)?;
    let mut created: HashMap<String, i64> = sqlx::query_as(statements.insert_blocks)
        .bind(&links)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?
        .into_iter()
        .collect();
    if !created.is_empty() {
        let facts = blocks
            .iter()
            .filter_map(|(block, digest)| created.remove(digest).map(|id| (id, block.facts)))
            .collect::<Vec<_>>();
        sqlx::query(statements.insert_facts)
            .bind(encode(&facts)?)
            .execute(&mut *connection)
            .await
            .map_err(sql_error)?;
    }
    sqlx::query(statements.insert_links)
        .bind(links)
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(())
}
