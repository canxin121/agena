use crate::{SessionViewMode, session_hub::*, session_search::*};
use agena_tui::i18n::I18n;
use agena_tui_components::{
    Editor, SearchPickerConfig, SearchPickerFocus, SearchPickerSearchMode,
    pointer::{self, PointerAction},
};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, text::Line};

fn item(id: i64, chinese: bool) -> SessionSearchItem {
    let label = if chinese {
        format!("会话 {id:02} · 优化终端设置与会话浏览体验")
    } else {
        format!("Session {id:02} · Improve terminal settings")
    };
    SessionSearchItem {
        session_id: id,
        title: label.clone(),
        label,
        pinned: id == 3,
        favorite: id == 3 || id == 6,
        summary: if chinese {
            "就绪 · 10-18 12:30"
        } else {
            "Ready · 10-18 12:30"
        }
        .into(),
        detail: format!("/projects/agena · #{id} · 24 messages · 2026-10-18 12:30"),
    }
}

fn search(chinese: bool) -> SessionSearchOverlay {
    let mut state = SessionSearchOverlay::new(
        if chinese {
            "恢复会话"
        } else {
            "Resume Session"
        }
        .into(),
        String::new(),
        String::new(),
        "No sessions".into(),
        Editor::from_text(String::new()),
        SearchPickerConfig {
            search_mode: SearchPickerSearchMode::External,
            ..SearchPickerConfig::searchable()
        },
        None,
        SessionSearchPresentation::new(SessionViewMode::All, None),
    );
    state.replace_items((1..=45).map(|id| item(id, chinese)).collect());
    state.focus = SearchPickerFocus::Results;
    state.move_selection(2);
    state
}

fn hub(chinese: bool) -> SessionHubPresentation {
    let convert = |id| {
        let item = item(id, chinese);
        SessionHubItem {
            session_id: id,
            workspace_id: 1,
            pinned: item.pinned,
            favorite: item.favorite,
            title: item.title,
            label: item.label,
            summary: item.summary,
            detail: item.detail,
            is_new_session: false,
        }
    };
    let mut state = SessionHubPresentation::empty();
    let mut new = convert(0);
    new.label = if chinese {
        "+ 新建会话"
    } else {
        "+ New session"
    }
    .into();
    new.is_new_session = true;
    new.summary.clear();
    state.set_catalog(
        vec![
            SessionHubDirectory {
                workspace_id: 1,
                path: "/projects/agena".into(),
                expanded: true,
                visible_limit: 20,
                items: (1..=20).map(convert).collect(),
            },
            SessionHubDirectory {
                workspace_id: 2,
                path: "/projects/website".into(),
                expanded: false,
                visible_limit: 20,
                items: vec![],
            },
        ],
        vec![
            SessionHubSection::new(SessionHubSectionKind::New, vec![new]),
            SessionHubSection::new(SessionHubSectionKind::Pinned, vec![convert(3)]),
            SessionHubSection::new(
                SessionHubSectionKind::Favorites,
                vec![convert(3), convert(6)],
            ),
            SessionHubSection::new(
                SessionHubSectionKind::Recent,
                (1..=20).map(convert).collect(),
            ),
        ],
    );
    state.set_pagination(
        [(
            HubPageTarget::Directory(1),
            HubPagination {
                page: 0,
                has_more: true,
                total: Some(184),
            },
        )]
        .into(),
    );
    let index = state
        .rows()
        .iter()
        .position(|row| matches!(row, HubRow::Item(item) if item.session_id == 3))
        .unwrap();
    state.select_row(index);
    state
}

fn plain(buffer: &Buffer) -> String {
    let mut text = String::new();
    for y in buffer.area.y..buffer.area.bottom() {
        let mut x = buffer.area.x;
        while x < buffer.area.right() {
            let symbol = buffer[(x, y)].symbol();
            text.push_str(symbol);
            x += Line::from(symbol).width().max(1) as u16;
        }
        text.push('\n');
    }
    text
}

fn click(x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn session_search_uses_terminal_height_and_pointer_rows_follow_visual_pages() {
    for (width, height) in [(80, 24), (120, 36), (40, 12), (18, 6), (1, 1)] {
        let mut state = search(false);
        state.move_selection_home();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut targets = Default::default();
        terminal
            .draw(|frame| {
                targets = pointer::capture(|| {
                    render_overlay(frame, frame.area(), &state, &I18n::english())
                })
            })
            .unwrap();
        if height >= 24 {
            assert!(state.page_size() >= 16);
        }
        let page_size = state.page_size();
        state.move_selection_page(1);
        terminal
            .draw(|frame| {
                targets = pointer::capture(|| {
                    render_overlay(frame, frame.area(), &state, &I18n::english())
                })
            })
            .unwrap();
        let mut ids = std::collections::BTreeSet::new();
        for y in 0..height {
            for x in 0..width {
                if let Some(PointerAction::PickerRow(index)) = targets.action(click(x, y)) {
                    assert!(index >= page_size);
                    assert!(index < 2 * page_size);
                    ids.insert(state.items[index].session_id);
                }
            }
        }
        if height >= 12 {
            assert_eq!(ids.len(), page_size.min(45 - page_size));
        }
    }
}

#[test]
fn session_mark_updates_survive_subtree_filtering_and_late_pages() {
    let mut state = search(false);
    state.meta.all_items = state.items.clone();
    state.meta.remember_marks(3, Some(true), Some(true));
    state.meta.remember_marks(3, Some(false), None);
    let mut stale = item(3, false);
    state.meta.apply_marks(&mut stale);
    assert!(!stale.pinned && stale.favorite);
    let saved = state
        .meta
        .all_items
        .iter()
        .find(|item| item.session_id == 3)
        .unwrap();
    assert!(!saved.pinned && saved.favorite);
    state.meta.reset_for_query();
    state.meta.apply_marks(&mut stale);
    assert!(!stale.pinned && stale.favorite);
}

#[test]
fn session_hub_page_target_and_refresh_preserve_the_selected_bucket() {
    let mut state = hub(false);
    assert_eq!(
        state.page_request(true),
        Some((HubPageTarget::Directory(1), 1))
    );
    assert_eq!(state.page_request(false), None);
    let favorite = state
        .rows()
        .iter()
        .position(|row| matches!(row, HubRow::Header(SessionHubSectionKind::Favorites)))
        .unwrap()
        + 1;
    state.select_row(favorite);
    state.set_item_pinned(3, false);
    assert_eq!(
        state.selected_page_target(),
        HubPageTarget::Section(SessionHubSectionKind::Favorites)
    );
    state.set_item_favorite(3, false);
    assert_eq!(state.selected_item().map(|item| item.session_id), Some(6));
    state.select_page_target(HubPageTarget::Directory(1), false);
    assert_eq!(state.selected_item().unwrap().session_id, 1);
    state.select_page_target(HubPageTarget::Directory(1), true);
    assert_eq!(state.selected_item().unwrap().session_id, 20);
}

#[test]
fn session_browsing_locales_and_long_titles_remain_compact() {
    for locale in [
        "en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "de-DE", "es-ES", "fr-FR",
    ] {
        let i18n = I18n::resolve(Some(locale), None);
        let state = search(true);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| render_overlay(frame, frame.area(), &state, &i18n))
            .unwrap();
        let text = plain(terminal.backend().buffer());
        assert!(!text.contains("session-search-") && !text.contains("session-page-"));
        assert!(text.contains("会话 16"));
        assert!(text.contains('◆') && text.contains('★'));
    }
}

#[test]
#[ignore = "writes production renderer fixtures for visual review"]
fn session_browsing_write_previews() {
    use agena_tui_components::theme::{self, ColorScheme, ThemePalette};
    let directory = std::path::PathBuf::from(
        std::env::var("AGENA_SESSION_PREVIEW_DIR").expect("preview directory"),
    );
    std::fs::create_dir_all(&directory).unwrap();
    for (name, locale, width, height, light, is_hub) in [
        ("sessions-search-80", "en-US", 80, 24, false, false),
        ("sessions-search-before", "en-US", 80, 24, false, false),
        ("sessions-search-chinese", "zh-CN", 80, 24, false, false),
        ("sessions-hub-wide", "en-US", 120, 32, false, true),
        ("sessions-hub-chinese", "zh-CN", 80, 24, false, true),
        ("sessions-hub-light", "zh-CN", 120, 32, true, true),
        ("sessions-search-narrow", "zh-CN", 40, 16, false, false),
    ] {
        theme::set_active_palette(ThemePalette::for_scheme(if light {
            ColorScheme::Light
        } else {
            ColorScheme::Dark
        }));
        let i18n = I18n::resolve(Some(locale), None);
        let mut state = search(locale == "zh-CN");
        state.prompt = i18n.text("overlay-resume-prompt");
        state.footer = "scope all · 45 loaded · end".into();
        let hub = hub(locale == "zh-CN");
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                if is_hub {
                    render_session_hub(frame, frame.area(), &hub, false, None, false, "", &i18n);
                } else if name == "sessions-search-before" {
                    let spec = agena_tui_components::SearchPickerDialogSpec::new(
                        i18n.text("overlay-picker-loading").into(),
                        i18n.text("overlay-attach-matches").into(),
                    )
                    .with_text(&i18n);
                    agena_tui_components::render_search_picker_dialog(
                        frame,
                        frame.area(),
                        &state,
                        &spec,
                        agena_tui::sanitize_picker_text,
                    );
                } else {
                    render_overlay(frame, frame.area(), &state, &i18n);
                }
            })
            .unwrap();
        if !is_hub {
            println!("{name}: {} visible rows", state.page_size());
        }
        let buffer = terminal.backend().buffer();
        std::fs::write(directory.join(format!("{name}.txt")), plain(buffer)).unwrap();
        export_svg(&directory, name, buffer, light);
    }
}

fn export_svg(directory: &std::path::Path, name: &str, buffer: &Buffer, light: bool) {
    use ratatui::style::{Color, Modifier};
    use std::fmt::Write;
    let width = buffer.area.width;
    let height = buffer.area.height;
    let background = if light { "#fafafa" } else { "#18181b" };
    let foreground = if light { "#1f2328" } else { "#f0f6fc" };
    let color = |color, fallback: &str| match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.to_owned(),
    };
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"{background}\"/>",
        width * 10 + 32,
        height * 22 + 32,
        width * 10 + 32,
        height * 22 + 32
    );
    for y in 0..height {
        for x in 0..width {
            let cell = &buffer[(x, y)];
            let left = x * 10 + 16;
            let top = y * 22 + 16;
            if cell.bg != Color::Reset {
                write!(
                    svg,
                    "<rect x=\"{left}\" y=\"{top}\" width=\"10\" height=\"22\" fill=\"{}\"/>",
                    color(cell.bg, background)
                )
                .unwrap();
            }
        }
    }
    svg.push_str("<g font-family=\"Menlo, 'PingFang SC', monospace\" font-size=\"16\">");
    for y in 0..height {
        for x in 0..width {
            let cell = &buffer[(x, y)];
            if cell.symbol().trim().is_empty() {
                continue;
            }
            write!(
                svg,
                "<text x=\"{}\" y=\"{}\" fill=\"{}\" font-weight=\"{}\">{}</text>",
                x * 10 + 16,
                y * 22 + 33,
                color(cell.fg, foreground),
                if cell.modifier.contains(Modifier::BOLD) {
                    "700"
                } else {
                    "400"
                },
                escape(cell.symbol())
            )
            .unwrap();
        }
    }
    svg.push_str("</g></svg>");
    std::fs::write(directory.join(format!("{name}.svg")), svg).unwrap();
}
