//! Import a verified new-kernel archive into a fresh, independently owned session.

use super::*;

/// Validate the exact decoded archive configuration before it is staged.
/// The callback can capture runtime policy without coupling storage to runtime.
pub type SessionImportValidator =
    Arc<dyn Fn(&SessionConfiguration) -> std::result::Result<(), String> + Send + Sync>;

impl SessionCommands {
    /// The backend consumes the file through a bounded staging parser, verifies
    /// its complete footer and references, then publishes one atomic session.
    /// The supplied file is read only. Imported work is never scheduled.
    pub async fn import_archive(
        &self,
        request: RequestId,
        workspace_id: i64,
        path: &std::path::Path,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        self.0
            .backend
            .import_archive(request, workspace_id, path)
            .await
    }

    /// Apply host policy to the same parsed configuration that is published by
    /// the transactional import. The source file is not reopened for validation.
    pub async fn import_archive_validated(
        &self,
        request: RequestId,
        workspace_id: i64,
        path: &std::path::Path,
        validator: SessionImportValidator,
    ) -> Result<CommitReceipt<SessionMetadata>> {
        self.0
            .backend
            .import_archive_validated(request, workspace_id, path, validator)
            .await
    }
}
