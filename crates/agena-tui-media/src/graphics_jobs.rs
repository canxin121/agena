//! Encode complete viewport-relative native image cells on bounded workers.
//! iTerm2 crops are pre-rendered; Kitty/Sixel keep their cheap scroll-time
//! placement/slicing without re-encoding pixels or prematurely acknowledging upload.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, LazyLock, mpsc},
};

use ratatui::{
    buffer::Buffer,
    layout::{Rect, Size},
    widgets::Widget,
};
use ratatui_image::{
    Resize,
    picker::{Picker, ProtocolType},
    sliced::{SignedPosition, SlicedImage, SlicedProtocol},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{
    MathArtifact,
    background::BackgroundMedia,
    graphics_selection::{GraphicHighlight, highlighted_graphic},
};

type Key = (u64, u16, u16);
type Prepared = Result<PreparedGraphic, String>;

pub(super) enum PreparedGraphic {
    Cells(Buffer),
    Native {
        protocol: Box<SlicedProtocol>,
        estimated_bytes: usize,
    },
}

impl PreparedGraphic {
    fn bytes(&self) -> usize {
        match self {
            Self::Cells(buffer) => buffer.content.iter().fold(0usize, |sum, cell| {
                sum.saturating_add(std::mem::size_of_val(cell))
                    .saturating_add(cell.symbol().len())
            }),
            Self::Native {
                estimated_bytes, ..
            } => *estimated_bytes,
        }
    }

    pub(super) fn paint(&self, target: &mut Buffer, area: Rect, position: SignedPosition) {
        match self {
            Self::Cells(buffer) => blit(
                buffer,
                target,
                area.x.saturating_add(position.x.max(0) as u16),
                area.y.saturating_add(position.y.max(0) as u16),
            ),
            Self::Native { protocol, .. } => {
                // Kitty's first transmission is acknowledged only when its
                // cells actually enter the UI frame, never on a worker.
                SlicedImage::new(protocol, position).render(area, target);
            }
        }
    }
}
const MAX_ENTRIES: usize = 128;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_CELLS: usize = 256 * 1024;
static WORKERS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(2)));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Viewport {
    pub skip: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Signature {
    style: u64,
    viewport: Viewport,
}

struct Entry {
    signature: Signature,
    buffer: Option<PreparedGraphic>,
    bytes: usize,
}

struct Completed {
    key: Key,
    signature: Signature,
    result: Prepared,
    // Finished, unread results also count towards the concurrency limit.
    // A slow UI cannot accumulate a queue of large encoded rasters.
    _permit: OwnedSemaphorePermit,
}

struct Completion {
    key: Key,
    signature: Signature,
    sender: mpsc::Sender<Completed>,
    notifier: Arc<BackgroundMedia>,
    permit: Option<OwnedSemaphorePermit>,
}

impl Completion {
    fn publish(&mut self, result: Prepared) {
        if let Some(permit) = self.permit.take() {
            let _ = self.sender.send(Completed {
                key: self.key,
                signature: self.signature,
                result,
                _permit: permit,
            });
            // Only request repaint: protocol completion must not invalidate
            // Markdown layout or all other image artifacts.
            self.notifier.notify_ui();
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        if self.permit.is_some() {
            self.publish(Err("native image worker exited without a result".into()));
        }
    }
}

pub(super) struct GraphicsJobs {
    cache: HashMap<Key, Entry>,
    order: VecDeque<Key>,
    bytes: usize,
    pending: HashSet<Key>,
    sender: mpsc::Sender<Completed>,
    receiver: mpsc::Receiver<Completed>,
    workers: Arc<Semaphore>,
}

impl Default for GraphicsJobs {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            cache: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            pending: HashSet::new(),
            sender,
            receiver,
            workers: WORKERS.clone(),
        }
    }
}

impl GraphicsJobs {
    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(completed) = self.receiver.try_recv() {
            changed = true;
            self.pending.remove(&completed.key);
            self.insert(completed.key, completed.signature, completed.result);
            // The result's permit is released only after adoption or rejection.
        }
        changed
    }

    fn insert(&mut self, key: Key, signature: Signature, result: Prepared) {
        let mut buffer = match result {
            Ok(buffer) => Some(buffer),
            Err(error) => {
                tracing::debug!(artifact_id = key.0, %error, "native image preparation failed");
                None
            }
        };
        let mut bytes = buffer.as_ref().map_or(0, PreparedGraphic::bytes);
        if bytes > MAX_BYTES {
            // Cache failure as well; otherwise an oversized image retries on
            // every animation frame while continuously allocating huge buffers.
            buffer = None;
            bytes = 0;
        }
        if let Some(previous) = self.cache.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
        }
        self.order.retain(|item| *item != key);
        self.cache.insert(
            key,
            Entry {
                signature,
                buffer,
                bytes,
            },
        );
        self.order.push_back(key);
        self.bytes = self.bytes.saturating_add(bytes);
        while self.cache.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
            if let Some(oldest) = self.order.pop_front() {
                if let Some(entry) = self.cache.remove(&oldest) {
                    self.bytes = self.bytes.saturating_sub(entry.bytes);
                }
            } else {
                break;
            }
        }
    }

    fn request(
        &mut self,
        key: Key,
        signature: Signature,
        notifier: Arc<BackgroundMedia>,
        work: impl FnOnce() -> Prepared + Send + 'static,
    ) -> Option<&PreparedGraphic> {
        self.poll();
        let current = self
            .cache
            .get(&key)
            .is_some_and(|entry| entry.signature == signature);
        if !current
            && !self.pending.contains(&key)
            && let Ok(permit) = self.workers.clone().try_acquire_owned()
        {
            self.pending.insert(key);
            let mut completion = Completion {
                key,
                signature,
                sender: self.sender.clone(),
                notifier,
                permit: Some(permit),
            };
            // Spawn failure and unwinding both publish a failed completion, so
            // this key cannot get stuck in pending forever.
            let _ = std::thread::Builder::new()
                .name("agena-tui-image-encode".into())
                .spawn(move || completion.publish(work()));
        }
        self.order.retain(|item| *item != key);
        if self.cache.contains_key(&key) {
            self.order.push_back(key);
        }
        self.cache
            .get(&key)
            .filter(|entry| {
                // Keep the previous highlight while its replacement is encoding,
                // but never paint a stale viewport crop at different coordinates.
                entry.signature.viewport == signature.viewport
            })
            .and_then(|entry| entry.buffer.as_ref())
    }

    pub(super) fn prepare(
        &mut self,
        placement: &super::TranscriptMathPlacement,
        style: u64,
        viewport: Viewport,
        highlights: Vec<GraphicHighlight>,
        picker: Picker,
        notifier: Arc<BackgroundMedia>,
    ) -> Option<&PreparedGraphic> {
        let artifact = placement.artifact.clone();
        let size = placement.size;
        let key = (artifact.id, size.width, size.height);
        let reusable = matches!(
            picker.protocol_type(),
            ProtocolType::Kitty | ProtocolType::Sixel
        );
        let signature = Signature {
            style,
            // These protocols can crop at placement time without encoding
            // again. A viewport change must not invalidate their image ID.
            viewport: if reusable {
                Viewport {
                    skip: 0,
                    width: size.width,
                    height: size.height,
                }
            } else {
                viewport
            },
        };
        self.request(key, signature, notifier, move || {
            if reusable {
                let font = picker.font_size();
                // Conservative raster/sequence budget; iTerm2 cell buffers
                // below are charged by their actual stored symbol lengths.
                let estimated_bytes = usize::from(size.width)
                    .saturating_mul(usize::from(size.height))
                    .saturating_mul(usize::from(font.width))
                    .saturating_mul(usize::from(font.height))
                    .saturating_mul(16);
                if estimated_bytes > MAX_BYTES {
                    return Err("native image exceeds encoded cache budget".into());
                }
                let image = highlighted_graphic(&artifact, size, font, &highlights)
                    .ok_or_else(|| "native image target exceeds pixel safety limits".to_owned())?;
                let protocol =
                    SlicedProtocol::new_with_resize(&picker, image, size, Resize::Scale(None))
                        .map_err(|error| format!("native image encoding failed: {error}"))?;
                Ok(PreparedGraphic::Native {
                    protocol: Box::new(protocol),
                    estimated_bytes,
                })
            } else {
                prepare_cells(&artifact, size, viewport, &highlights, &picker)
                    .map(PreparedGraphic::Cells)
            }
        })
    }
}

fn prepare_cells(
    artifact: &MathArtifact,
    size: Size,
    viewport: Viewport,
    highlights: &[GraphicHighlight],
    picker: &Picker,
) -> Result<Buffer, String> {
    if viewport.width == 0
        || viewport.height == 0
        || usize::from(viewport.width) * usize::from(viewport.height) > MAX_CELLS
    {
        return Err("native image viewport exceeds cell safety limits".into());
    }
    let image = highlighted_graphic(artifact, size, picker.font_size(), highlights)
        .ok_or_else(|| "native image target exceeds pixel safety limits".to_owned())?;
    let protocol = SlicedProtocol::new_with_resize(picker, image, size, Resize::Scale(None))
        .map_err(|error| format!("native image encoding failed: {error}"))?;
    let skip = i16::try_from(viewport.skip).map_err(|_| "image crop exceeds coordinate limits")?;
    let area = Rect::new(0, 0, viewport.width, viewport.height);
    let mut buffer = Buffer::empty(area);
    SlicedImage::new(&protocol, SignedPosition::from((0, -skip))).render(area, &mut buffer);
    Ok(buffer)
}

pub(super) fn blit(source: &Buffer, target: &mut Buffer, x: u16, y: u16) {
    for row in 0..source.area.height {
        for column in 0..source.area.width {
            if let Some(cell) = target.cell_mut((x.saturating_add(column), y.saturating_add(row))) {
                *cell = source[(column, row)].clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::DynamicImage;
    use ratatui_image::{FontSize, picker::ProtocolType};
    use std::time::{Duration, Instant};

    fn signature(style: u64) -> Signature {
        Signature {
            style,
            viewport: Viewport {
                skip: 0,
                width: 2,
                height: 2,
            },
        }
    }

    #[test]
    fn native_scroll_reuses_encoded_pixels_and_kitty_upload_belongs_to_the_real_frame() {
        for protocol in [ProtocolType::Kitty, ProtocolType::Sixel] {
            let picker = Picker::from_parts(FontSize::new(2, 2), protocol, false, vec![]);
            let placement = super::super::TranscriptMathPlacement {
                line: 0,
                column: 0,
                size: Size::new(2, 4),
                artifact: Arc::new(MathArtifact {
                    id: 55,
                    image: DynamicImage::new_rgba8(4, 8),
                    size: Size::new(2, 4),
                    foreground: None,
                    row_layout_weights: vec![],
                }),
            };
            let mut jobs = GraphicsJobs {
                workers: Arc::new(Semaphore::new(1)),
                ..Default::default()
            };
            let notifier = Arc::new(BackgroundMedia::default());
            let viewport = Viewport {
                skip: 0,
                width: 2,
                height: 4,
            };
            assert!(
                jobs.prepare(
                    &placement,
                    0,
                    viewport,
                    vec![],
                    picker.clone(),
                    notifier.clone()
                )
                .is_none()
            );
            let deadline = Instant::now() + Duration::from_secs(3);
            while !jobs.pending.is_empty() {
                jobs.poll();
                assert!(Instant::now() < deadline, "encoding did not complete");
                std::thread::sleep(Duration::from_millis(1));
            }
            let graphic = jobs
                .prepare(
                    &placement,
                    0,
                    viewport,
                    vec![],
                    picker.clone(),
                    notifier.clone(),
                )
                .unwrap();
            let mut first = Buffer::empty(Rect::new(0, 0, 2, 4));
            graphic.paint(
                &mut first,
                Rect::new(0, 0, 2, 4),
                SignedPosition::from((0, 0)),
            );
            if protocol == ProtocolType::Kitty {
                assert!(
                    first[(0, 0)].symbol().contains("\x1b_G"),
                    "worker preparation must not consume Kitty's first transmission"
                );
            }
            let scrolled = Viewport {
                skip: 1,
                width: 2,
                height: 2,
            };
            let graphic = jobs
                .prepare(&placement, 0, scrolled, vec![], picker, notifier)
                .unwrap();
            let mut second = Buffer::empty(Rect::new(0, 0, 2, 2));
            graphic.paint(
                &mut second,
                Rect::new(0, 0, 2, 2),
                SignedPosition::from((0, -1)),
            );
            if protocol == ProtocolType::Kitty {
                assert!(
                    !second[(0, 0)].symbol().contains("\x1b_G"),
                    "scrolling must not upload the same pixels again"
                );
            }
            assert!(
                jobs.pending.is_empty(),
                "scrolling must not start another encoding job"
            );
            assert_eq!(jobs.cache.len(), 1);
        }
    }

    #[test]
    fn cropped_prepared_cells_match_synchronous_native_rendering() {
        let picker = Picker::from_parts(FontSize::new(2, 2), ProtocolType::Iterm2, false, vec![]);
        let artifact = MathArtifact {
            id: 1,
            image: DynamicImage::new_rgba8(4, 8),
            size: Size::new(2, 4),
            foreground: None,
            row_layout_weights: vec![],
        };
        for (skip, height) in [(0, 4), (1, 2), (0, 1), (3, 1)] {
            let viewport = Viewport {
                skip,
                width: 2,
                height,
            };
            let prepared = prepare_cells(&artifact, artifact.size, viewport, &[], &picker).unwrap();
            let protocol = SlicedProtocol::new_with_resize(
                &picker,
                artifact.image.clone(),
                artifact.size,
                Resize::Scale(None),
            )
            .unwrap();
            let area = Rect::new(3, 2, 2, height);
            let mut expected = Buffer::empty(Rect::new(0, 0, 8, 8));
            SlicedImage::new(&protocol, SignedPosition::from((0, -(skip as i16))))
                .render(area, &mut expected);
            let mut actual = Buffer::empty(expected.area);
            blit(&prepared, &mut actual, area.x, area.y);
            assert_eq!(actual, expected, "skip={skip}, height={height}");
        }
    }

    #[test]
    fn pending_work_and_unread_results_are_bounded_and_stale_crops_are_not_painted() {
        let workers = Arc::new(Semaphore::new(1));
        let mut jobs = GraphicsJobs {
            workers: workers.clone(),
            ..Default::default()
        };
        let notifier = Arc::new(BackgroundMedia::default());
        let (release, hold) = mpsc::channel();
        assert!(
            jobs.request((1, 2, 2), signature(0), notifier.clone(), move || {
                hold.recv_timeout(Duration::from_secs(3)).unwrap();
                Ok(PreparedGraphic::Cells(Buffer::empty(Rect::new(0, 0, 2, 2))))
            })
            .is_none()
        );
        assert!(
            jobs.request((1, 2, 2), signature(1), notifier.clone(), || panic!(
                "duplicate job"
            ))
            .is_none()
        );
        assert!(
            jobs.request((2, 2, 2), signature(0), notifier.clone(), || panic!(
                "unbounded job"
            ))
            .is_none()
        );
        assert_eq!(workers.available_permits(), 0);
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while jobs.cache.is_empty() {
            jobs.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(workers.available_permits(), 1);
        assert!(
            jobs.request((1, 2, 2), signature(0), notifier.clone(), || panic!(
                "cache miss"
            ))
            .is_some()
        );
        let cropped = Signature {
            viewport: Viewport {
                skip: 1,
                width: 2,
                height: 1,
            },
            ..signature(0)
        };
        assert!(
            jobs.request((1, 2, 2), cropped, notifier, || Err("failed".into()))
                .is_none()
        );
    }

    #[test]
    fn lru_eviction_preserves_recent_images_and_failures_do_not_retry() {
        let mut jobs = GraphicsJobs::default();
        for id in 0..MAX_ENTRIES as u64 {
            jobs.insert(
                (id, 2, 2),
                signature(0),
                Ok(PreparedGraphic::Cells(Buffer::empty(Rect::new(0, 0, 2, 2)))),
            );
        }
        let notifier = Arc::new(BackgroundMedia::default());
        jobs.request((0, 2, 2), signature(0), notifier.clone(), || {
            panic!("cached")
        });
        jobs.insert(
            (MAX_ENTRIES as u64, 2, 2),
            signature(0),
            Err("invalid image".into()),
        );
        assert!(jobs.cache.contains_key(&(0, 2, 2)));
        assert!(!jobs.cache.contains_key(&(1, 2, 2)));
        assert_eq!(jobs.cache.len(), MAX_ENTRIES);
        assert!(jobs.bytes <= MAX_BYTES);
        assert!(
            jobs.request(
                (MAX_ENTRIES as u64, 2, 2),
                signature(0),
                notifier,
                || panic!("retry storm")
            )
            .is_none()
        );
        assert!(jobs.pending.is_empty());
    }
}
