//! Bounded first-use media work for the live TUI. Export/pager callers keep
//! synchronous rendering; only an explicitly enabled render context uses this.

use super::*;
use portable_atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

type ArtifactResult = Result<Arc<MathArtifact>, String>;
static WORKERS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(2)));

#[derive(Clone, Copy, Hash)]
pub(super) enum Request {
    Formula(bool),
    Svg,
    Image,
}

#[derive(Debug)]
enum Entry {
    Pending(Option<std::sync::Weak<MathArtifact>>),
    Ready {
        result: ArtifactResult,
        expires: Option<Instant>,
    },
}

impl Entry {
    fn pixels(&self) -> u64 {
        match self {
            Self::Ready {
                result: Ok(artifact),
                ..
            } => image_pixels(&artifact.image),
            _ => 0,
        }
    }
}

#[derive(Debug, Default)]
struct Cache {
    entries: HashMap<u64, Entry>,
    order: VecDeque<u64>,
    pixels: u64,
    // Rows already retain artifacts. Weak lookups avoid decoding them again
    // after LRU eviction without retaining additional image pixels.
    shared: HashMap<u64, (std::sync::Weak<MathArtifact>, Option<Instant>)>,
    shared_order: VecDeque<u64>,
}

impl Cache {
    fn remove(&mut self, key: u64) {
        if let Some(entry) = self.entries.remove(&key) {
            self.pixels = self.pixels.saturating_sub(entry.pixels());
        }
        self.order.retain(|id| *id != key);
    }

    fn evict_one(&mut self) -> bool {
        let Some(key) = self
            .order
            .iter()
            .copied()
            .find(|key| !matches!(self.entries.get(key), Some(Entry::Pending(_))))
        else {
            return false;
        };
        if let Some(Entry::Ready {
            result: Ok(artifact),
            expires,
        }) = self.entries.get(&key)
        {
            self.shared
                .insert(key, (Arc::downgrade(artifact), *expires));
            self.shared_order.retain(|id| *id != key);
            self.shared_order.push_back(key);
            while self.shared_order.len() > MAX_ARTIFACTS * 8 {
                if let Some(expired) = self.shared_order.pop_front() {
                    self.shared.remove(&expired);
                }
            }
        }
        self.remove(key);
        true
    }
}

#[derive(Debug)]
pub(super) struct BackgroundMedia {
    cache: Mutex<Cache>,
    generation: AtomicU64,
    changed: tokio::sync::Notify,
    retry_when_available: AtomicBool,
    workers: Arc<Semaphore>,
}

impl Default for BackgroundMedia {
    fn default() -> Self {
        Self {
            cache: Mutex::new(Cache::default()),
            generation: AtomicU64::new(0),
            changed: tokio::sync::Notify::new(),
            retry_when_available: AtomicBool::new(false),
            workers: WORKERS.clone(),
        }
    }
}

impl BackgroundMedia {
    pub(super) fn notify_ui(&self) {
        self.changed.notify_one();
    }

    pub(super) async fn changed(&self) {
        self.changed.notified().await;
    }

    pub(super) fn generation(&self) -> u64 {
        // Old appearance/session jobs do not publish into this cache. Retry
        // once when their capacity returns, without continuously invalidating.
        if self.workers.available_permits() > 0
            && self.retry_when_available.swap(false, Ordering::AcqRel)
        {
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
        self.generation.load(Ordering::Acquire)
    }

    fn complete(&self, key: u64, result: ArtifactResult, ttl: Option<Duration>) {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.remove(key);
        let entry = Entry::Ready {
            result,
            expires: ttl.map(|ttl| Instant::now() + ttl),
        };
        cache.pixels = cache.pixels.saturating_add(entry.pixels());
        cache.entries.insert(key, entry);
        cache.order.push_back(key);
        while cache.pixels > MAX_CACHED_PIXELS || cache.entries.len() > MAX_ARTIFACTS {
            if !cache.evict_one() {
                break;
            }
        }
        drop(cache);
        self.generation.fetch_add(1, Ordering::Release);
        // Keep one pending wake until the live UI observes it. Image loading
        // must not wait for the 500 ms idle repaint interval between batches.
        self.changed.notify_one();
    }

    fn request(
        self: &Arc<Self>,
        key: u64,
        ttl: Option<Duration>,
        make_work: impl FnOnce() -> Box<dyn FnOnce() -> ArtifactResult + Send>,
    ) -> ArtifactResult {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut previous = None;
        if let Some(entry) = cache.entries.get(&key) {
            match entry {
                Entry::Pending(previous) => {
                    usage::record();
                    return previous
                        .as_ref()
                        .and_then(std::sync::Weak::upgrade)
                        .ok_or_else(|| "media is rendering".into());
                }
                Entry::Ready { result, expires }
                    if expires.is_none_or(|at| at > Instant::now()) =>
                {
                    // A local-file error has a retry TTL, unlike a permanent
                    // invalid embedded image. Keep that entry eligible when
                    // later media updates make another layout pass necessary.
                    if result.is_err() && expires.is_some() {
                        usage::record();
                    }
                    let result = result.clone();
                    cache.order.retain(|id| *id != key);
                    cache.order.push_back(key);
                    return result;
                }
                Entry::Ready { result, .. } => {
                    // While rechecking a mutable local file, retain the last
                    // good image on screen. A placeholder would change row
                    // geometry on every TTL expiration and cause scroll jumps.
                    previous = result.as_ref().ok().cloned();
                    cache.remove(key);
                }
            }
        }
        if let Some((artifact, expires)) = cache.shared.get(&key)
            && expires.is_none_or(|at| at > Instant::now())
            && let Some(artifact) = artifact.upgrade()
        {
            return Ok(artifact);
        }
        // Only incomplete/revalidating media depends on completion events.
        // Stable entries survive other images completing and LRU eviction.
        usage::record();
        cache.shared.remove(&key);
        cache.shared_order.retain(|id| *id != key);
        let Ok(permit) = self.workers.clone().try_acquire_owned() else {
            self.retry_when_available.store(true, Ordering::Release);
            if let Some(previous) = previous {
                cache.entries.insert(
                    key,
                    Entry::Ready {
                        result: Ok(previous.clone()),
                        expires: Some(Instant::now()),
                    },
                );
                cache.pixels = cache.pixels.saturating_add(image_pixels(&previous.image));
                cache.order.push_back(key);
                return Ok(previous);
            }
            return Err("media renderer is busy".into());
        };
        while cache.entries.len() >= MAX_ARTIFACTS {
            if !cache.evict_one() {
                return Err("media cache is busy".into());
            }
        }
        cache
            .entries
            .insert(key, Entry::Pending(previous.as_ref().map(Arc::downgrade)));
        cache.order.push_back(key);
        drop(cache);
        // No queue of large encoded payloads: clone only after getting a slot.
        let work = make_work();
        let owner = self.clone();
        let runtime = tokio::runtime::Handle::try_current().ok();
        let spawned = std::thread::Builder::new()
            .name("agena-tui-media".into())
            .spawn(move || {
                let _permit = permit;
                let _runtime = runtime.as_ref().map(|handle| handle.enter());
                struct CompletionGuard(Arc<BackgroundMedia>, u64, bool);
                impl Drop for CompletionGuard {
                    fn drop(&mut self) {
                        if !self.2 {
                            self.0.complete(
                                self.1,
                                Err("media worker exited without a result".into()),
                                None,
                            );
                        }
                    }
                }
                let mut guard = CompletionGuard(owner.clone(), key, false);
                owner.complete(key, work(), ttl);
                guard.2 = true;
            });
        if let Err(error) = spawned {
            self.complete(key, Err(format!("cannot start media worker: {error}")), ttl);
        }
        previous.ok_or_else(|| "media is rendering".into())
    }
}

/// Resize a decoded image for ordinary colored terminal cells on the same
/// bounded worker pool as raster decoding. Capturing an Arc avoids copying
/// the full source raster on the event-loop thread.
pub(super) fn halfblock_preview(
    artifact: &Arc<MathArtifact>,
    size: Size,
) -> Option<ArtifactResult> {
    if TEXT_MATH_RENDER_DEPTH.get() > 0 {
        return None;
    }
    let owner = RENDER_CONTEXT_STACK.with(|stack| {
        stack
            .borrow()
            .last()
            .and_then(|context| context.background.clone())
    })?;
    let mut hash = DefaultHasher::new();
    ("halfblock-preview", artifact.id, size.width, size.height).hash(&mut hash);
    let key = hash.finish();
    Some(owner.request(key, None, || {
        let artifact = artifact.clone();
        Box::new(move || {
            Ok(Arc::new(MathArtifact {
                id: key,
                image: artifact.image.resize_exact(
                    u32::from(size.width),
                    u32::from(size.height) * 2,
                    image::imageops::FilterType::Triangle,
                ),
                size,
                foreground: None,
                row_layout_weights: Vec::new(),
            }))
        })
    }))
}

pub(super) fn render(request: Request, source: &str) -> Option<ArtifactResult> {
    if TEXT_MATH_RENDER_DEPTH.get() > 0 {
        return None;
    }
    let (owner, mut context) = RENDER_CONTEXT_STACK.with(|stack| {
        let stack = stack.borrow();
        let context = stack.last()?;
        Some((context.background.clone()?, context.clone()))
    })?;
    // Reuse the synchronous renderer's exact limits, containment and theme.
    context.background = None;
    let remote = matches!(request, Request::Image)
        && source.split_once(':').is_some_and(|(scheme, _)| {
            scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
        });
    if remote {
        // Do not spend a decode slot (or publish a layout generation) just to
        // discover that an offscreen/loading image has no bytes yet. Histories
        // larger than the negative cache would otherwise churn empty jobs.
        let ready = url::Url::parse(source)
            .map_err(|error| error.to_string())
            .and_then(|url| {
                if DEFER_REMOTE_IMAGES.get() {
                    remote_image::cached(&url)
                } else {
                    remote_image::load(&url)
                }
            });
        if let Err(error) = ready {
            usage::record();
            return Some(Err(error));
        }
    }
    let local = matches!(request, Request::Image) && !remote && !source.starts_with("data:");
    let mut hash = DefaultHasher::new();
    request.hash(&mut hash);
    source.hash(&mut hash);
    if remote {
        remote_image::generation().hash(&mut hash);
    }
    let deferred = DEFER_REMOTE_IMAGES.get();
    Some(owner.request(
        hash.finish(),
        local.then_some(Duration::from_secs(1)),
        || {
            let source = source.to_owned();
            Box::new(move || {
                with_math_render_context(&context, || {
                    let render = || match request {
                        Request::Formula(display) => render_formula(&source, display),
                        Request::Svg => render_markdown_svg(&source),
                        Request::Image => render_markdown_image(&source),
                    };
                    if deferred {
                        with_deferred_remote_images(render)
                    } else {
                        render()
                    }
                })
            })
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> Arc<BackgroundMedia> {
        Arc::new(BackgroundMedia {
            workers: Arc::new(Semaphore::new(1)),
            ..Default::default()
        })
    }

    fn await_generation(cache: &BackgroundMedia, previous: u64) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while cache.generation() == previous {
            assert!(
                Instant::now() < deadline,
                "worker did not publish completion"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn slow_work_is_nonblocking_deduplicated_and_bounded() {
        let cache = cache();
        let (release, hold) = std::sync::mpsc::channel();
        assert!(
            cache
                .request(1, None, || Box::new(move || {
                    hold.recv_timeout(Duration::from_secs(3)).unwrap();
                    Err("finished".into())
                }))
                .unwrap_err()
                .contains("rendering")
        );
        assert!(cache.request(1, None, || panic!("duplicate job")).is_err());
        assert!(
            cache
                .request(2, None, || panic!("unbounded job"))
                .unwrap_err()
                .contains("busy")
        );
        assert_eq!(cache.workers.available_permits(), 0);
        release.send(()).unwrap();
        await_generation(&cache, 0);
        assert_eq!(
            cache
                .request(1, None, || panic!("negative cache miss"))
                .unwrap_err(),
            "finished"
        );
    }

    #[test]
    fn cached_artifacts_obey_entry_and_pixel_accounting() {
        let cache = cache();
        for key in 0..200 {
            cache.complete(key, Err("invalid source".into()), None);
        }
        let state = cache.cache.lock().unwrap();
        assert_eq!(state.entries.len(), MAX_ARTIFACTS);
        assert_eq!(state.order.len(), MAX_ARTIFACTS);
        assert_eq!(state.pixels, 0);
        drop(state);
        let artifact = Arc::new(MathArtifact {
            id: 1000,
            image: DynamicImage::new_rgba8(2, 2),
            size: Size::new(1, 1),
            foreground: None,
            row_layout_weights: Vec::new(),
        });
        cache.complete(1000, Ok(artifact.clone()), None);
        assert!(Arc::ptr_eq(
            &cache.request(1000, None, || panic!("cache miss")).unwrap(),
            &artifact
        ));
        assert_eq!(cache.cache.lock().unwrap().pixels, 4);
    }

    #[test]
    fn live_image_decode_publishes_a_reusable_artifact() {
        let context = MathRenderContext {
            background: Some(cache()),
            ..Default::default()
        };
        let mut png = std::io::Cursor::new(Vec::new());
        DynamicImage::new_rgba8(3, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let source = format!(
            "data:image/png;base64,{}",
            BASE64_STANDARD.encode(png.into_inner())
        );
        let before = with_math_render_context(&context, remote_image_generation);
        assert!(with_math_render_context(&context, || render_markdown_image(&source)).is_err());
        await_generation(context.background.as_ref().unwrap(), 0);
        let artifact =
            with_math_render_context(&context, || render_markdown_image(&source)).unwrap();
        assert_eq!((artifact.image.width(), artifact.image.height()), (3, 2));
        assert!(with_math_render_context(&context, remote_image_generation) > before);
        let again = with_math_render_context(&context, || render_markdown_image(&source)).unwrap();
        assert!(Arc::ptr_eq(&artifact, &again));
        for key in 0..200 {
            context
                .background
                .as_ref()
                .unwrap()
                .complete(key, Err("other".into()), None);
        }
        let after_eviction =
            with_math_render_context(&context, || render_markdown_image(&source)).unwrap();
        assert!(Arc::ptr_eq(&artifact, &after_eviction));
    }

    #[test]
    fn local_image_refresh_keeps_the_previous_pixels_until_completion() {
        let cache = cache();
        let old = Arc::new(MathArtifact {
            id: 2000,
            image: DynamicImage::new_rgba8(2, 2),
            size: Size::new(1, 1),
            foreground: None,
            row_layout_weights: Vec::new(),
        });
        let new = Arc::new(MathArtifact {
            id: 2001,
            image: DynamicImage::new_rgba8(4, 4),
            size: Size::new(2, 2),
            foreground: None,
            row_layout_weights: Vec::new(),
        });
        cache.complete(1, Ok(old.clone()), Some(Duration::ZERO));
        let generation = cache.generation();
        let (release, hold) = std::sync::mpsc::channel();
        let replacement = new.clone();
        let displayed = cache
            .request(1, None, || {
                Box::new(move || {
                    hold.recv_timeout(Duration::from_secs(3)).unwrap();
                    Ok(replacement)
                })
            })
            .unwrap();
        assert!(Arc::ptr_eq(&displayed, &old));
        assert!(Arc::ptr_eq(
            &cache
                .request(1, None, || panic!("duplicate refresh"))
                .unwrap(),
            &old
        ));
        release.send(()).unwrap();
        await_generation(&cache, generation);
        assert!(Arc::ptr_eq(
            &cache
                .request(1, None, || panic!("unnecessary refresh"))
                .unwrap(),
            &new
        ));
    }

    #[tokio::test]
    async fn completion_wakes_the_ui_even_when_it_is_not_yet_waiting() {
        let cache = cache();
        cache.complete(1, Err("finished".into()), None);
        tokio::time::timeout(Duration::from_secs(1), cache.changed())
            .await
            .unwrap();
        assert_eq!(cache.generation(), 1);
    }

    #[tokio::test]
    async fn halfblock_preview_is_async_reusable_and_pixel_identical() {
        let owner = cache();
        let context = MathRenderContext {
            background: Some(owner.clone()),
            image_background: [231, 232, 233],
            ..Default::default()
        };
        let image = image::RgbaImage::from_fn(64, 48, |x, y| {
            Rgba([(x * 3) as u8, (y * 5) as u8, 91, ((x + y) * 2) as u8])
        });
        let artifact = Arc::new(MathArtifact {
            id: 8844,
            image: DynamicImage::ImageRgba8(image),
            size: Size::new(64, 24),
            foreground: None,
            row_layout_weights: Vec::new(),
        });
        let size = Size::new(10, 4);
        let expected =
            with_math_render_context(&context, || image_halfblock_lines(&artifact, size));
        let pending =
            with_math_render_context(&context, || image_halfblock_lines_shared(&artifact, size));
        assert_eq!(pending.len(), usize::from(size.height));
        assert!(pending.iter().all(|line| line.spans.is_empty()));
        tokio::time::timeout(Duration::from_secs(3), context.background_changed())
            .await
            .unwrap();
        let rendered =
            with_math_render_context(&context, || image_halfblock_lines_shared(&artifact, size));
        assert_eq!(rendered, expected);
        let generation = owner.generation();
        assert_eq!(
            with_math_render_context(&context, || image_halfblock_lines_shared(&artifact, size)),
            expected
        );
        assert_eq!(
            owner.generation(),
            generation,
            "cache hits must not spawn or invalidate"
        );
    }

    #[test]
    fn retryable_failures_keep_a_dependency_but_permanent_failures_do_not() {
        let owner = cache();
        owner.complete(1, Err("invalid embedded payload".into()), None);
        owner.complete(
            2,
            Err("local file not ready".into()),
            Some(Duration::from_secs(30)),
        );
        assert!(!track_media_usage(|| owner.request(1, None, || panic!("cached failure"))).1);
        assert!(track_media_usage(|| owner.request(2, None, || panic!("cached failure"))).1);
    }

    #[test]
    fn offscreen_remote_sources_do_not_spawn_empty_jobs_or_churn_generations() {
        let owner = cache();
        let context = MathRenderContext {
            background: Some(owner.clone()),
            ..Default::default()
        };
        with_math_render_context(&context, || {
            with_deferred_remote_images(|| {
                // More than the bounded failure cache: the old path repeatedly
                // scheduled these nonexistent decodes as they evicted each other.
                for index in 0..MAX_ARTIFACTS * 3 {
                    let source = format!("https://media-audit.invalid/not-downloaded-{index}.png");
                    assert!(render_markdown_image(&source).is_err());
                }
            })
        });
        assert!(owner.cache.lock().unwrap().entries.is_empty());
        assert_eq!(owner.workers.available_permits(), 1);
        assert_eq!(owner.generation(), 0);
    }

    #[test]
    fn live_context_is_opt_in_and_exports_stay_synchronous() {
        let mut context = MathRenderContext::default();
        assert!(with_math_render_context(&context, || render(Request::Svg, "invalid")).is_none());
        context.enable_background_rendering();
        assert!(
            with_math_render_context(&context, || with_text_math_rendering(|| {
                render(Request::Svg, "invalid")
            }))
            .is_none()
        );
    }
}
