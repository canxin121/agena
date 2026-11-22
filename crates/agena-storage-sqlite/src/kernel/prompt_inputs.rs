//! Ordered references to immutable blocks of frozen prompt entry facts.

use sea_orm::sqlx::sqlite::SqliteRow;

use super::*;
use prompt_manifest::{Block, Statements, bucket};

const CAPTURE_SQL: &str = "SELECT ?1 AS round_id,CAST(j.key AS INTEGER) AS ordinal,
    e.id AS entry_id,COALESCE(f.revision,e.revision) AS entry_revision,
    CASE WHEN f.entry_id IS NULL THEN a.response_entry_id ELSE f.response_entry_id END AS response_entry_id,
    COALESCE(f.execution_phase,latest.phase,a.phase) AS execution_phase,
    CASE WHEN f.entry_id IS NOT NULL THEN f.tool_phase WHEN e.kind='tool' THEN t.phase END AS tool_phase,
    COALESCE(f.interaction_decided,i.decision_json IS NOT NULL) AS interaction_decided
FROM json_each(?2) j
CROSS JOIN agena_entries e ON e.id=j.value
CROSS JOIN agena_attempts a ON a.execution_id=e.execution_id AND a.generation=e.generation
LEFT JOIN agena_executions x ON e.kind='user' AND x.id=e.execution_id
LEFT JOIN agena_attempts latest ON latest.execution_id=x.id AND latest.generation=x.current_generation
LEFT JOIN agena_branch_entries f ON f.session_id=?3 AND f.entry_id=e.id
LEFT JOIN agena_tool_calls t ON t.id=e.tool_id
LEFT JOIN agena_interactions i ON i.id=e.interaction_id
WHERE EXISTS(
    SELECT 1 FROM agena_segment_entries m INDEXED BY idx_agena_segment_entry_reverse
    CROSS JOIN agena_session_segments s INDEXED BY idx_agena_session_segment_reverse
        ON s.segment_id=m.segment_id AND s.session_id=?3
    WHERE m.entry_id=e.id AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal
)";

const WRITE_SQL: Statements = Statements {
    insert_blocks: "INSERT INTO agena_prompt_input_blocks(digest)
        SELECT unhex(value->>2) FROM json_each(?) WHERE 1
        ON CONFLICT(digest) DO NOTHING RETURNING lower(hex(digest)),id",
    insert_facts: "INSERT INTO agena_prompt_input_entries(
             block_id,ordinal,entry_id,entry_revision,response_entry_id,execution_phase,tool_phase,interaction_decided
         )
         SELECT b.value->>0,CAST(e.key AS INTEGER),e.value->>'entry_id',e.value->>'entry_revision',
                e.value->>'response_entry_id',e.value->>'execution_phase',e.value->>'tool_phase',e.value->>'interaction_decided'
         FROM json_each(?) b CROSS JOIN json_each(b.value,'$[1]') e",
    insert_links: "INSERT INTO agena_round_input_blocks(round_id,start_ordinal,block_id)
        SELECT r.value->>0,r.value->>1,b.id FROM json_each(?) r
        CROSS JOIN agena_prompt_input_blocks b ON b.digest=unhex(r.value->>2)",
};

#[derive(serde::Serialize)]
pub(super) struct FrozenInput {
    #[serde(skip)]
    round_id: i64,
    #[serde(skip)]
    ordinal: i64,
    entry_id: i64,
    entry_revision: i64,
    response_entry_id: Option<i64>,
    execution_phase: String,
    tool_phase: Option<String>,
    interaction_decided: bool,
}

impl FrozenInput {
    pub(super) fn from_row(row: SqliteRow) -> std::result::Result<Self, sqlx::Error> {
        Ok(Self {
            round_id: row.try_get("round_id")?,
            ordinal: row.try_get("ordinal")?,
            entry_id: row.try_get("entry_id")?,
            entry_revision: row.try_get("entry_revision")?,
            response_entry_id: row.try_get("response_entry_id")?,
            execution_phase: row.try_get("execution_phase")?,
            tool_phase: row.try_get("tool_phase")?,
            interaction_decided: row.try_get("interaction_decided")?,
        })
    }
}

// Group by a stable range of entry identities. Dropping one entry from the
// front of a window changes the first group while retaining interior blocks.
pub(super) fn blocks(inputs: &[FrozenInput]) -> impl DoubleEndedIterator<Item = &[FrozenInput]> {
    inputs.chunk_by(|a, b| {
        a.round_id == b.round_id
            && a.ordinal.checked_add(1) == Some(b.ordinal)
            && bucket(a.entry_id) == bucket(b.entry_id)
    })
}

pub(super) async fn capture(
    connection: &mut SqliteConnection,
    round: i64,
    session: SessionId,
    inputs: &[EntryId],
) -> Result<()> {
    if inputs.is_empty() {
        return Ok(());
    }
    let mut frozen = sqlx::query(CAPTURE_SQL)
        .bind(round)
        .bind(encode(inputs)?)
        .bind(session.get())
        .try_map(FrozenInput::from_row)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
    if frozen.len() != inputs.len() {
        return Err(KernelError::Ownership);
    }
    frozen.sort_unstable_by_key(|input| input.ordinal);
    persist(connection, &frozen).await
}

pub(super) async fn persist(
    connection: &mut SqliteConnection,
    inputs: &[FrozenInput],
) -> Result<()> {
    let blocks = blocks(inputs).map(|facts| Block {
        round: facts[0].round_id,
        position: facts[0].ordinal,
        facts,
        metadata: (),
    });
    prompt_manifest::persist(connection, blocks, &WRITE_SQL).await
}
