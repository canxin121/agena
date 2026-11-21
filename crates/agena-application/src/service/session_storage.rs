use agena_storage::kernel::{
    ConfirmStorageCleanup, DatabaseReclaimResult, SessionStorageDetail, SessionStorageDetailQuery,
    SessionStorageList, SessionStorageQuery, StorageCleanupPreview, StorageOperation,
    StorageSelection,
};

use super::{ApplicationResult, ApplicationService};

impl ApplicationService {
    pub async fn list_session_storage(
        &self,
        query: SessionStorageQuery,
    ) -> ApplicationResult<SessionStorageList> {
        self.session_kernel()
            .storage()
            .list(query)
            .await
            .map_err(crate::kernel_error)
    }

    pub async fn session_storage(
        &self,
        session: i64,
        query: SessionStorageDetailQuery,
    ) -> ApplicationResult<SessionStorageDetail> {
        self.session_kernel()
            .storage()
            .detail(crate::kernel_session_id(session)?, query)
            .await
            .map_err(crate::kernel_error)
    }

    pub async fn session_storage_operation(
        &self,
        session: i64,
    ) -> ApplicationResult<Option<StorageOperation>> {
        self.session_kernel()
            .storage()
            .operation(crate::kernel_session_id(session)?)
            .await
            .map_err(crate::kernel_error)
    }

    pub async fn preview_session_storage(
        &self,
        session: i64,
        selection: StorageSelection,
    ) -> ApplicationResult<StorageCleanupPreview> {
        self.session_kernel()
            .storage()
            .preview(crate::kernel_session_id(session)?, selection)
            .await
            .map_err(crate::kernel_error)
    }

    pub async fn confirm_session_storage(
        &self,
        session: i64,
        confirmation: ConfirmStorageCleanup,
    ) -> ApplicationResult<StorageOperation> {
        self.session_kernel()
            .storage()
            .confirm(crate::kernel_session_id(session)?, confirmation)
            .await
            .map_err(crate::kernel_error)
    }

    pub async fn reclaim_database_storage(&self) -> ApplicationResult<DatabaseReclaimResult> {
        self.session_kernel()
            .storage()
            .reclaim_database()
            .await
            .map_err(crate::kernel_error)
    }
}
