use std::{
    env,
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use agena_api::resource::{ServerEndpointRecord, ServerIdentityResource};
use anyhow::{Context, Result};

pub(crate) struct PublishedServerRecord {
    path: PathBuf,
    server_id: uuid::Uuid,
    pid: u32,
}

pub(crate) fn record_path() -> PathBuf {
    agena_storage_layout::StorageLayout::current()
        .run_dir()
        .join("server.json")
}

pub(crate) fn read_record(path: &Path) -> Result<ServerEndpointRecord> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read server record {}", path.display()))?;
    let record = serde_json::from_slice::<ServerEndpointRecord>(&bytes)
        .with_context(|| format!("failed to decode server record {}", path.display()))?;
    anyhow::ensure!(
        record.schema == ServerEndpointRecord::SCHEMA,
        "unsupported server record schema {}",
        record.schema
    );
    Ok(record)
}

pub(crate) fn publish_record(
    url: String,
    identity: &ServerIdentityResource,
) -> Result<PublishedServerRecord> {
    publish_record_at(record_path(), url, identity)
}

fn publish_record_at(
    path: PathBuf,
    url: String,
    identity: &ServerIdentityResource,
) -> Result<PublishedServerRecord> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create server record directory {}",
                parent.display()
            )
        })?;
    }
    let temp_path = path.with_extension(format!("json.{}.{}.tmp", identity.pid, identity.id));
    let record = ServerEndpointRecord {
        schema: ServerEndpointRecord::SCHEMA,
        url,
        server_id: identity.id,
        pid: identity.pid,
        started_at: identity.started_at,
        protocol_version: identity.protocol_version,
    };
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(&temp_path).with_context(|| {
        format!(
            "failed to create temporary server record {}",
            temp_path.display()
        )
    })?;
    let write_result = (|| -> Result<()> {
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &record)
            .context("failed to serialize server record")?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        fs::rename(&temp_path, &path).with_context(|| {
            format!(
                "failed to atomically publish server record {}",
                path.display()
            )
        })?;
        Ok(())
    })();
    if write_result.is_err()
        && let Err(cleanup_error) = fs::remove_file(&temp_path)
        && cleanup_error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            path = %temp_path.display(),
            diagnostic = %agena_failure::diagnostic::format_error_chain(&cleanup_error),
            "failed to remove a temporary server record after publication failed"
        );
    }
    write_result?;
    Ok(PublishedServerRecord {
        path,
        server_id: identity.id,
        pid: identity.pid,
    })
}

impl Drop for PublishedServerRecord {
    fn drop(&mut self) {
        let Ok(record) = read_record(&self.path) else {
            return;
        };
        if record.server_id == self.server_id
            && record.pid == self.pid
            && let Err(error) = fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %self.path.display(),
                diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                "failed to remove this process's published server record during shutdown"
            );
        }
    }
}
