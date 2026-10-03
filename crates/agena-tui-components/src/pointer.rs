//! Frame-local pointer targets. Widgets register the exact cells they draw.
//!
//! Capture is scoped to one synchronous render, isolated per thread, and
//! discarded on unwind. Opening a surface clears targets underneath it. The
//! application owns actions; components never execute callbacks or backend work.
use std::cell::RefCell;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{Frame, layout::Rect, text::Span, widgets::Paragraph};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerAction {
    Key(KeyEvent),
    List {
        panel: usize,
        index: usize,
        selected: usize,
    },
    FocusList(usize),
    PickerInput,
    PickerRow(usize),
    PickerResults,
    PickerPreview(i16),
    Named(&'static str),
    NamedIndex(&'static str, usize),
}

#[derive(Debug, Clone)]
struct Target {
    area: Rect,
    click: Option<PointerAction>,
    wheel: Option<(PointerAction, PointerAction)>,
}

#[derive(Debug, Clone, Default)]
pub struct PointerMap {
    targets: Vec<Target>,
    lists: usize,
}

impl PointerMap {
    pub fn action(&self, event: MouseEvent) -> Option<PointerAction> {
        self.targets.iter().rev().find_map(|target| {
            if !target.area.contains((event.column, event.row).into()) {
                return None;
            }
            match event.kind {
                MouseEventKind::Down(crossterm::event::MouseButton::Left) => target.click.clone(),
                MouseEventKind::ScrollUp => target.wheel.as_ref().map(|pair| pair.0.clone()),
                MouseEventKind::ScrollDown => target.wheel.as_ref().map(|pair| pair.1.clone()),
                _ => None,
            }
        })
    }
}

thread_local! {
    static CAPTURE: RefCell<Option<PointerMap>> = const { RefCell::new(None) };
}

/// Collect targets without changing the public signatures of render-only widgets.
/// Nested/test renders restore the previous capture even if drawing panics.
pub fn capture(draw: impl FnOnce()) -> PointerMap {
    struct Restore(Option<PointerMap>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CAPTURE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let previous = CAPTURE.with(|slot| slot.replace(Some(PointerMap::default())));
    let _restore = Restore(previous);
    draw();
    CAPTURE.with(|slot| slot.borrow_mut().take().unwrap_or_default())
}

pub fn begin_surface() {
    CAPTURE.with(|slot| {
        if let Some(map) = slot.borrow_mut().as_mut() {
            *map = PointerMap::default();
        }
    });
}

pub fn key(code: KeyCode) -> PointerAction {
    PointerAction::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

pub fn register(
    area: Rect,
    click: Option<PointerAction>,
    wheel: Option<(PointerAction, PointerAction)>,
) {
    if area.is_empty() {
        return;
    }
    CAPTURE.with(|slot| {
        if let Some(map) = slot.borrow_mut().as_mut() {
            map.targets.push(Target { area, click, wheel });
        }
    });
}

pub fn next_list() -> usize {
    CAPTURE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(map) = slot.as_mut() else {
            return 0;
        };
        let panel = map.lists;
        map.lists += 1;
        panel
    })
}

/// Draw compact, left-aligned buttons and register only fully visible labels.
pub fn render_buttons(frame: &mut Frame, area: Rect, buttons: &[(&str, PointerAction)]) {
    let mut x = area.x;
    for (label, action) in buttons {
        let text = format!(" {label} ");
        let width = u16::try_from(UnicodeWidthStr::width(text.as_str())).unwrap_or(u16::MAX);
        if x.saturating_add(width) > area.right() || area.height == 0 {
            break;
        }
        let rect = Rect::new(x, area.y, width, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(text, crate::theme::status_chip_style())),
            rect,
        );
        register(rect, Some(action.clone()), None);
        x = x.saturating_add(width).saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn click(x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn top_surface_blocks_background_and_clips_targets() {
        let map = capture(|| {
            register(Rect::new(0, 0, 20, 20), Some(key(KeyCode::Enter)), None);
            begin_surface();
            register(Rect::new(4, 4, 3, 1), Some(key(KeyCode::Esc)), None);
        });
        assert_eq!(map.action(click(4, 4)), Some(key(KeyCode::Esc)));
        assert_eq!(map.action(click(7, 4)), None);
        assert_eq!(map.action(click(0, 0)), None);
        assert!(capture(|| {}).targets.is_empty());
    }

    #[test]
    fn nested_capture_restores_parent() {
        let map = capture(|| {
            register(Rect::new(0, 0, 1, 1), Some(key(KeyCode::Enter)), None);
            let inner = capture(|| register(Rect::new(1, 0, 1, 1), Some(key(KeyCode::Esc)), None));
            assert_eq!(inner.action(click(1, 0)), Some(key(KeyCode::Esc)));
        });
        assert_eq!(map.action(click(0, 0)), Some(key(KeyCode::Enter)));
        assert_eq!(map.action(click(1, 0)), None);
    }
}
