//! Validate a non-empty Agena database against the one current schema.
//! No migrations or repairs are performed.

use std::collections::BTreeMap;

use sea_orm::{ConnectionTrait, DbErr, Statement};

use super::{INDEXES, TABLES};
use crate::schema_invariants::INVARIANT_TRIGGERS;

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

fn incompatible(detail: impl std::fmt::Display) -> DbErr {
    DbErr::Custom(format!(
        "database schema does not match this Agena build: {detail}; delete the database and start with a fresh one"
    ))
}

pub(crate) async fn validate_existing_schema<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let actual = schema_objects(db).await?;
    let mut expected = BTreeMap::new();
    for sql in TABLES.iter().chain(INDEXES).chain(INVARIANT_TRIGGERS) {
        let declaration = declaration(sql)?;
        expected.insert(
            (declaration.kind.to_owned(), declaration.name.to_owned()),
            declaration.stored_sql,
        );
    }
    if actual == expected {
        return Ok(());
    }
    for (identity, sql) in &expected {
        match actual.get(identity) {
            None => {
                return Err(incompatible(format!(
                    "required {} {} is missing",
                    identity.0, identity.1
                )));
            }
            Some(found) if found != sql => {
                return Err(incompatible(format!(
                    "{} {} has a different definition",
                    identity.0, identity.1
                )));
            }
            Some(_) => {}
        }
    }
    if let Some(((kind, name), _)) = actual
        .iter()
        .find(|(identity, _)| !expected.contains_key(*identity))
    {
        return Err(incompatible(format!("unexpected {kind} {name}")));
    }
    Err(incompatible("schema differs from the current declarations"))
}
