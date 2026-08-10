use super::{AppError, SessionManager, UsageStats, UsageStatsQuery};

impl SessionManager {
    pub async fn usage_stats(&self, query: UsageStatsQuery) -> Result<UsageStats, AppError> {
        let workspace_id = self.current_workspace_id().await?;
        self.store.usage_stats(workspace_id, query).await
    }
}
