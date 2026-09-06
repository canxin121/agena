//! Check the version marker against the declarations Agena actually created.
//! A compatible trigger correction may be refreshed; incompatible tables and
//! required indexes are never repaired or silently claimed as a fresh schema.

use std::collections::{BTreeMap, BTreeSet};

use sea_orm::{ConnectionTrait, DbErr, Statement};

use super::{INDEXES, TABLES};

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
        "database schema is incompatible: {detail}; Agena does not migrate incompatible databases, so create a fresh database"
    ))
}

pub(super) async fn require_empty_database<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    if let Some(((kind, name), _)) = schema_objects(db).await?.first_key_value() {
        return Err(incompatible(format!(
            "version zero requires an empty database, but {kind} {name} already exists"
        )));
    }
    Ok(())
}

pub(super) async fn validate_existing_schema<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let objects = schema_objects(db).await?;
    let mut expected_tables = BTreeSet::new();
    for sql in TABLES.iter().chain(INDEXES) {
        let expected = declaration(sql)?;
        if expected.kind == "table" {
            expected_tables.insert(expected.name);
        }
        match objects.get(&(expected.kind.to_owned(), expected.name.to_owned())) {
            Some(actual) if actual == &expected.stored_sql => {}
            Some(_) => {
                return Err(incompatible(format!(
                    "{} {} has an unsupported definition",
                    expected.kind, expected.name
                )));
            }
            None => {
                return Err(incompatible(format!(
                    "required {} {} is missing",
                    expected.kind, expected.name
                )));
            }
        }
    }
    for (kind, name) in objects.keys() {
        if kind == "table" && name.starts_with("agena_") && !expected_tables.contains(name.as_str())
        {
            return Err(incompatible(format!("unexpected Agena table {name}")));
        }
    }
    Ok(())
}
