use super::*;
use agena_tui_components::pointer::{self, PointerAction};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

struct Hint {
    line: Line<'static>,
    width: usize,
    action: Option<PointerAction>,
}

pub(super) struct Footer(Vec<Hint>);

impl Footer {
    pub(super) fn new(
        view: HubView,
        selected: Option<&SessionHubItem>,
        searching: bool,
        i18n: &I18n,
    ) -> Self {
        let mut footer = Self(Vec::new());
        let key = |code, modifiers| Some(PointerAction::Key(KeyEvent::new(code, modifiers)));
        let ctrl = |c| key(KeyCode::Char(c), KeyModifiers::CONTROL);
        let alt = |c| key(KeyCode::Char(c), KeyModifiers::ALT);
        footer.push("↑↓", "hub-hint-move", None, i18n);
        footer.push(
            "Enter",
            "hub-hint-open-expand",
            key(KeyCode::Enter, KeyModifiers::NONE),
            i18n,
        );
        footer.push("Tab/⇧Tab", "hub-hint-view", None, i18n);
        if view == HubView::Projects {
            footer.push("h/l ←→", "hub-hint-project", None, i18n);
            footer.push("Ctrl+P", "hub-action-add-project", ctrl('p'), i18n);
        }
        footer.push("[ ]", "session-action-page", None, i18n);
        footer.push("PgUp/Dn", "session-action-scroll", None, i18n);
        footer.push("Ctrl+N", "hub-action-create", ctrl('n'), i18n);
        footer.push("/", "hub-action-search", ctrl('f'), i18n);
        for (key, enabled, on, off, action) in [
            (
                "p",
                selected.is_some_and(|item| item.pinned),
                "session-action-unpin",
                "session-action-pin",
                alt('p'),
            ),
            (
                "f",
                selected.is_some_and(|item| item.favorite),
                "hub-action-unfavorite",
                "hub-action-favorite",
                alt('f'),
            ),
        ] {
            footer.push(key, if enabled { on } else { off }, action, i18n);
            // Mark toggles must not change footer height and resize/refetch
            // the session page. Reserve the wider of the two action labels.
            let width = build_shortcut_line([ShortcutHint::new(
                key,
                i18n.text(if enabled { off } else { on }),
            )])
            .width();
            let hint = footer.0.last_mut().expect("just inserted a mark action");
            hint.width = hint.width.max(width);
        }
        footer.push("Ctrl+R", "hub-action-refresh", ctrl('r'), i18n);
        if searching {
            footer.push(
                "Esc",
                "hub-action-clear-search",
                key(KeyCode::Esc, KeyModifiers::NONE),
                i18n,
            );
        }
        footer
    }

    fn push(&mut self, key: &'static str, label: &str, action: Option<PointerAction>, i18n: &I18n) {
        let line = build_shortcut_line([ShortcutHint::new(key, i18n.text(label))]);
        self.0.push(Hint {
            width: line.width(),
            line,
            action,
        });
    }

    fn cells(&self, area: Rect) -> Vec<(usize, Rect)> {
        if area.is_empty() {
            return Vec::new();
        }
        let mut cells = Vec::new();
        let (mut x, mut y) = (area.x, area.y);
        for (index, hint) in self.0.iter().enumerate() {
            let width = hint.width.min(usize::from(area.width)) as u16;
            if x > area.x && x.saturating_add(width) > area.right() {
                x = area.x;
                y = y.saturating_add(1);
            }
            if y >= area.bottom() {
                break;
            }
            cells.push((index, Rect::new(x, y, width, 1)));
            x = x.saturating_add(width).saturating_add(3);
        }
        cells
    }

    pub(super) fn height(&self, width: u16) -> u16 {
        self.cells(Rect::new(0, 0, width, u16::MAX))
            .last()
            .map_or(0, |(_, rect)| rect.bottom())
    }

    pub(super) fn render(&self, frame: &mut Frame, area: Rect) {
        for (index, rect) in self.cells(area) {
            let hint = &self.0[index];
            frame.render_widget(Paragraph::new(hint.line.clone()), rect);
            if let Some(action) = &hint.action {
                let target = Rect {
                    width: rect.width.min(hint.line.width() as u16),
                    ..rect
                };
                pointer::register(target, Some(action.clone()), None);
            }
        }
    }
}
