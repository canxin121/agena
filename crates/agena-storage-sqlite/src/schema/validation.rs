//! Validate the durable schema and present performance indexes exactly.
//! Return missing derived indexes; never repair tables or rewrite records.

use std::collections::BTreeMap;

use sea_orm::{ConnectionTrait, DbErr, Statement};

pub(crate) struct Declaration<'a> {
    pub kind: &'static str,
    pub name: &'a str,
    pub stored_sql: String,
}

/// SQLite removes IF NOT EXISTS from the stored CREATE statement. The rest
/// of our declaration is retained verbatim. Only trusted, internal DDL is
/// parsed here; this is deliberately not a general SQL parser.
pub(crate) fn declaration(sql: &str) -> Result<Declaration<'_>, DbErr> {
    for (prefix, kind) in [
        ("CREATE TABLE IF NOT EXISTS ", "table"),
        ("CREATE INDEX IF NOT EXISTS ", "index"),
        ("CREATE UNIQUE INDEX IF NOT EXISTS ", "index"),
        ("CREATE TRIGGER IF NOT EXISTS ", "trigger"),
    ] {
        if let Some(rest) = sql.strip_prefix(prefix) {
            let name = rest.split([' ', '(']).next().unwrap_or_default();
            if !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Ok(Declaration {
                    kind,
                    name,
                    stored_sql: sql.replacen(" IF NOT EXISTS ", " ", 1),
                });
            }
        }
    }
    Err(DbErr::Custom(
        "invalid internal schema declaration".to_owned(),
    ))
}

pub(crate) async fn schema_objects<C>(db: &C) -> Result<BTreeMap<(String, String), String>, DbErr>
where
    C: ConnectionTrait,
{
    db.query_all(Statement::from_string(
        db.get_database_backend(),
        "SELECT type, name, sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'",
    ))
    .await?
    .into_iter()
    .map(|row| {
        Ok((
            (row.try_get("", "type")?, row.try_get("", "name")?),
            row.try_get("", "sql")?,
        ))
    })
    .collect()
}
