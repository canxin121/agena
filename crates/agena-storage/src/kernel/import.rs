//! Import a verified new-kernel archive into a fresh, independently owned session.

use super::*;

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
}
