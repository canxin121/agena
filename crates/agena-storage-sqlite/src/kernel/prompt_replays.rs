//! Shared sets of replay identities and their state visibility at capture time.

use sea_orm::sqlx::sqlite::SqliteRow;

use super::*;
use prompt_manifest::{Block, Statements, bucket};

const CAPTURE_SQL: &str = "SELECT ?1 AS round_id,r.id AS source_round_id,
    COALESCE(f.phase,r.phase) AS phase,
    COALESCE(f.has_provider_state,r.provider_state_bytes>0) AS has_provider_state,
    CASE WHEN COALESCE(f.has_provider_state,r.provider_state_bytes>0)
         THEN r.provider_state_bytes ELSE 0 END AS state_bytes
FROM agena_round_inputs input
CROSS JOIN agena_entries e ON e.id=input.entry_id
CROSS JOIN agena_rounds r ON r.id=e.round_id
LEFT JOIN agena_branch_rounds f ON f.session_id=?2 AND f.round_id=r.id
WHERE input.round_id=?1 GROUP BY r.id ORDER BY r.id";

const WRITE_SQL: Statements = Statements {
    insert_blocks: "INSERT INTO agena_prompt_replay_blocks(digest,member_count,replay_bytes)
        SELECT unhex(value->>2),value->>'$[3][0]',value->>'$[3][1]' FROM json_each(?) WHERE 1
        ON CONFLICT(digest) DO NOTHING RETURNING lower(hex(digest)),id",
    insert_facts: "INSERT INTO agena_prompt_replay_entries(block_id,source_round_id,phase,has_provider_state)
        SELECT block.value->>0,e.value->>'source_round_id',e.value->>'phase',e.value->>'has_provider_state'
        FROM json_each(?) block CROSS JOIN json_each(block.value,'$[1]') e",
    insert_links: "INSERT INTO agena_round_replay_blocks(round_id,bucket,block_id)
        SELECT link.value->>0,link.value->>1,b.id FROM json_each(?) link
        CROSS JOIN agena_prompt_replay_blocks b ON b.digest=unhex(link.value->>2)",
};

pub(super) const BUDGET_SQL: &str = "SELECT COALESCE(sum(b.member_count),0) AS items,
    COALESCE(sum(b.replay_bytes),0) AS bytes
    FROM agena_round_replay_blocks r CROSS JOIN agena_prompt_replay_blocks b ON b.id=r.block_id
    WHERE r.round_id=?1";

pub(super) const READ_SQL: &str = "SELECT input.source_round_id,input.phase,r.provider_state_json
    FROM agena_round_input_replays input
    LEFT JOIN agena_rounds r ON input.has_provider_state=1 AND r.id=input.source_round_id
    WHERE input.round_id=? ORDER BY input.bucket,input.source_round_id LIMIT 10000";

pub(super) const EXPORT_SQL: &str = "SELECT input.round_id,input.source_round_id,input.phase,
           input.has_provider_state,x.message_id AS replay_owner
    FROM agena_round_input_replays input
    LEFT JOIN agena_rounds r ON r.id=input.source_round_id
    LEFT JOIN agena_executions x ON x.id=r.execution_id
    WHERE input.round_id=?1 AND input.bucket>=(?2-1)/64 AND input.source_round_id>?2
    ORDER BY input.bucket,input.source_round_id LIMIT 200";

#[derive(serde::Serialize)]
pub(super) struct FrozenReplay {
    #[serde(skip)]
    round_id: i64,
    source_round_id: i64,
    phase: String,
    has_provider_state: bool,
    #[serde(skip)]
    state_bytes: i64,
}

impl FrozenReplay {
    pub(super) fn from_row(row: SqliteRow) -> std::result::Result<Self, sqlx::Error> {
        Ok(Self {
            round_id: row.try_get("round_id")?,
            source_round_id: row.try_get("source_round_id")?,
            phase: row.try_get("phase")?,
            has_provider_state: row.try_get("has_provider_state")?,
            state_bytes: row.try_get("state_bytes")?,
        })
    }
}

pub(super) fn blocks(replays: &[FrozenReplay]) -> impl DoubleEndedIterator<Item = &[FrozenReplay]> {
    replays.chunk_by(|a, b| {
        a.round_id == b.round_id && bucket(a.source_round_id) == bucket(b.source_round_id)
    })
}

pub(super) async fn capture(
    connection: &mut SqliteConnection,
    round: i64,
    session: SessionId,
) -> Result<()> {
    let replays = sqlx::query(CAPTURE_SQL)
        .bind(round)
        .bind(session.get())
        .try_map(FrozenReplay::from_row)
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
    persist(connection, &replays).await
}

pub(super) async fn persist(
    connection: &mut SqliteConnection,
    replays: &[FrozenReplay],
) -> Result<()> {
    // Visible provider state is written once when its source round finishes.
    // Its byte charge therefore follows the same immutable facts as the digest.
    let blocks = blocks(replays).map(|facts| Block {
        round: facts[0].round_id,
        position: bucket(facts[0].source_round_id),
        metadata: (
            facts.len(),
            facts
                .iter()
                .map(|replay| 256 + replay.state_bytes)
                .sum::<i64>(),
        ),
        facts,
    });
    prompt_manifest::persist(connection, blocks, &WRITE_SQL).await
}
