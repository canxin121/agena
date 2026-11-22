//! Exact command replay shared by the normal writer and archive publication.

use super::*;
use crate::receipt_payload::{pack, unpack};

pub(super) async fn read(
    connection: &mut SqliteConnection,
    request: RequestId,
    digest: &[u8; 32],
) -> Result<Option<CommitReceipt<Mutation>>> {
    let row = sqlx::query(
        "SELECT request_digest,commit_sequence,command_ordinal,value_json
         FROM agena_command_results WHERE request_id=?",
    )
    .bind(request.0.as_bytes().as_slice())
    .fetch_optional(connection)
    .await
    .map_err(sql_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.try_get::<&[u8], _>("request_digest").map_err(storage)? != digest {
        return Err(KernelError::Conflict(
            "request id was already used for another command".into(),
        ));
    }
    let payload = row.try_get::<&[u8], _>("value_json").map_err(storage)?;
    let value = serde_json::from_slice(&unpack(payload).map_err(storage)?).map_err(storage)?;
    Ok(Some(CommitReceipt {
        position: CommitPosition {
            sequence: CommitSequence::try_from(
                row.try_get::<i64, _>("commit_sequence").map_err(storage)?,
            )
            .map_err(storage)?,
            ordinal: row.try_get("command_ordinal").map_err(storage)?,
        },
        value,
    }))
}

pub(super) async fn write(
    connection: &mut SqliteConnection,
    request: RequestId,
    digest: &[u8; 32],
    receipt: &CommitReceipt<Mutation>,
) -> Result<()> {
    let json = serde_json::to_vec(&receipt.value).map_err(storage)?;
    sqlx::query(
        "INSERT INTO agena_command_results
           (request_id,request_digest,commit_sequence,command_ordinal,value_json) VALUES (?,?,?,?,?)",
    )
    .bind(request.0.as_bytes().as_slice())
    .bind(digest.as_slice())
    .bind(receipt.position.sequence.get())
    .bind(i64::from(receipt.position.ordinal))
    .bind(pack(&json).map_err(storage)?)
    .execute(connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}
