use super::{AppError, SessionManager, UsageStats, UsageStatsQuery};

impl SessionManager {
    pub async fn usage_stats(&self, query: UsageStatsQuery) -> Result<UsageStats, AppError> {
        self.store.usage_stats(query).await
    }
}
