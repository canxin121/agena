use super::model::Session;

pub(crate) use agena_runtime::SessionCachePolicy;

impl agena_runtime::CacheEntry for Session {
    fn cache_key(&self) -> i64 {
        self.id
    }

    fn approx_cache_bytes(&self) -> usize {
        self.approx_bytes()
    }
}

pub(crate) type SessionCache = agena_runtime::SessionCache<Session>;
