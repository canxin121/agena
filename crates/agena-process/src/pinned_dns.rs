//! Share raw system DNS answers between policy checks and pinned HTTP routes.
//! Callers authorize DNS and check the returned addresses under live policy.

use std::io;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use moka::sync::Cache;

type Answers = Cache<(String, u16), Arc<[SocketAddr]>>;

static ANSWERS: LazyLock<Answers> = LazyLock::new(|| {
    Cache::builder()
        .max_capacity(256)
        .time_to_live(Duration::from_secs(5))
        .build()
});
static LOOKUPS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

/// Resolve a target for a client that pins and reauthorizes the actual addresses.
/// Concurrent misses share one system lookup and errors are eligible for retry.
pub fn resolve_blocking(host: &str, port: u16) -> io::Result<Arc<[SocketAddr]>> {
    resolve(&ANSWERS, (host.to_owned(), port), || {
        Ok((host, port).to_socket_addrs()?.collect())
    })
}

/// Async counterpart of `resolve_blocking`, using the same DNS answers.
pub async fn resolve_async(host: &str, port: u16) -> io::Result<Arc<[SocketAddr]>> {
    let key = (host.to_owned(), port);
    if let Some(hit) = ANSWERS.get(&key) {
        return Ok(hit);
    }
    LOOKUPS
        .run(move || resolve_blocking(&key.0, key.1))
        .await
        .map_err(io::Error::other)?
}

fn resolve(
    answers: &Answers,
    key: (String, u16),
    lookup: impl FnOnce() -> io::Result<Vec<SocketAddr>>,
) -> io::Result<Arc<[SocketAddr]>> {
    answers
        .try_get_with(key, || {
            let mut addresses = lookup()?;
            if addresses.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "DNS lookup returned no addresses",
                ));
            }
            addresses.sort_unstable();
            addresses.dedup();
            Ok(Arc::from(addresses))
        })
        .map_err(|error| io::Error::new(error.kind(), error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn overlapping_lookups_share_answers_and_failures_can_be_retried() {
        let answers = Cache::new(4);
        let key = ("fixture.example".to_owned(), 443);
        let address = "192.0.2.4:443".parse().unwrap();
        assert!(resolve(&answers, key.clone(), || Ok(Vec::new())).is_err());

        let starts = Barrier::new(8);
        let queries = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    starts.wait();
                    let resolved = resolve(&answers, key.clone(), || {
                        queries.fetch_add(1, Ordering::Relaxed);
                        std::thread::sleep(Duration::from_millis(20));
                        Ok(vec![address, address])
                    })
                    .unwrap();
                    assert_eq!(&*resolved, &[address]);
                });
            }
        });
        assert_eq!(queries.load(Ordering::Relaxed), 1);
    }
}
