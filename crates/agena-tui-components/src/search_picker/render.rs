use super::*;

/// Resolves the picker's own chrome through the active locale catalog.
///
/// The component never loads a Fluent catalog itself: the TUI passes its
/// `I18n` through [`SearchPickerDialogSpec::with_text`] and keeps the
/// `search-picker-*` keys translated. Callers that install no resolver keep
/// the built-in English wording so the component stays usable on its own.
pub trait SearchPickerText {
    /// Looks `key` up in the active catalog. `args` carries the arguments the
    /// message declares; plain messages pass an empty slice.
    fn text(&self, key: &str, args: &[(&str, String)]) -> String;
}

#[derive(Debug, Clone)]
/// View state of the search picker.
pub enum SearchPickerViewState<'a, TItem> {
    Loading { message: &'a str },
    Empty { message: &'a str },
    Error { message: &'a str },
    Selected(&'a TItem),
}

/// Spec of the search picker dialog.
pub struct SearchPickerDialogSpec<'a> {
    loading_message: Cow<'a, str>,
    results_title: Cow<'a, str>,
    preview_title: Option<Cow<'a, str>>,
    highlight_style: Style,
    highlight_symbol: Cow<'a, str>,
    checked_symbol: Cow<'a, str>,
    unchecked_symbol: Cow<'a, str>,
    search_label: Cow<'a, str>,
    text: Option<&'a dyn SearchPickerText>,
}

impl<'a> SearchPickerDialogSpec<'a> {
    pub fn new(loading_message: Cow<'a, str>, results_title: Cow<'a, str>) -> Self {
        Self {
            loading_message,
            results_title,
            preview_title: None,
            highlight_style: theme::selection_style(),
            highlight_symbol: Cow::Borrowed(">> "),
            checked_symbol: Cow::Borrowed("[x] "),
            unchecked_symbol: Cow::Borrowed("[ ] "),
            search_label: Cow::Borrowed(""),
            text: None,
        }
    }

    pub fn with_search_label(mut self, label: Cow<'a, str>) -> Self {
        self.search_label = label;
        self
    }

    /// Overrides the preview pane title. Without an override the title is the
    /// `search-picker-preview-title` catalog message.
    pub fn with_preview_title(mut self, title: Cow<'a, str>) -> Self {
        self.preview_title = Some(title);
        self
    }

    /// Routes the picker's own chrome through the active locale catalog. The
    /// TUI passes its `I18n`; callers that do not localize keep the built-in
    /// English wording.
    pub fn with_text(mut self, text: &'a dyn SearchPickerText) -> Self {
        self.text = Some(text);
        self
    }

    /// Localized chrome for `key`, falling back to `english` when no catalog
    /// is installed.
    fn chrome(&self, key: &str, english: &str) -> String {
        self.text
            .map(|text| text.text(key, &[]))
            .unwrap_or_else(|| english.to_owned())
    }

    /// Localized arg-bearing chrome. `english` uses `{name}` placeholders that
    /// mirror the catalog argument names and is only rendered when no catalog
    /// is installed.
    fn chrome_args(&self, key: &str, english: &str, args: &[(&str, String)]) -> String {
        self.text
            .map(|text| text.text(key, args))
            .unwrap_or_else(|| {
                let mut rendered = english.to_owned();
                for (name, value) in args {
                    rendered = rendered.replace(&format!("{{{name}}}"), value);
                }
                rendered
            })
    }

    fn preview_title(&self) -> Cow<'_, str> {
        self.preview_title
            .clone()
            .unwrap_or_else(|| Cow::Owned(self.chrome("search-picker-preview-title", "Preview")))
    }
}

pub fn render_search_picker_dialog<TItem, TCustom, TMeta, F>(
    frame: &mut Frame,
    area: Rect,
    picker: &SearchPicker<TItem, TCustom, TMeta, Editor>,
    spec: &SearchPickerDialogSpec<'_>,
    normalize_text: F,
) where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'a> Fn(&'a str) -> String,
{
    render_search_picker_dialog_with_preview(frame, area, picker, spec, normalize_text, |_| {
        Vec::new()
    });
}

pub fn render_search_picker_dialog_with_preview<TItem, TCustom, TMeta, F, P>(
    frame: &mut Frame,
    area: Rect,
    picker: &SearchPicker<TItem, TCustom, TMeta, Editor>,
    spec: &SearchPickerDialogSpec<'_>,
    normalize_text: F,
    build_preview: P,
) where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'a> Fn(&'a str) -> String,
    P: Fn(SearchPickerViewState<'_, TItem>) -> Vec<WorkbenchTextSection<'static>>,
{
    let area = search_picker_dialog_area(area);
    let content_width = area.width.saturating_sub(2);
    let prompt_height = optional_overlay_text_height(&picker.prompt, content_width, 1, 2);
    let input_height = if picker.config.input_mode.is_visible() {
        if picker.config.wrap_input {
            let input_width = content_width.saturating_sub(2).max(1);
            u16::try_from(picker.input.wrapped_line_count(input_width))
                .unwrap_or(u16::MAX)
                .clamp(1, 6)
                .saturating_add(2)
        } else {
            editor_input_panel_height(&picker.input, false)
        }
    } else {
        0
    };
    let footer = picker_footer(picker, spec, &normalize_text);
    let footer_height = optional_overlay_text_height(&footer, content_width, 1, 2);
    let list_height = 6;
    let mut sections = Vec::new();
    if prompt_height > 0 {
        sections.push(VerticalSectionSize::Fixed(prompt_height));
    }
    if input_height > 0 {
        sections.push(VerticalSectionSize::Fixed(input_height));
    }
    sections.push(VerticalSectionSize::Flexible(list_height));
    if footer_height > 0 {
        sections.push(VerticalSectionSize::Fixed(footer_height));
    }
    // Search pickers pre-compute their centered geometry and therefore use
    // Route layout inside that rectangle. They are still modal surfaces and
    // must not lose the visual hierarchy applied to ordinary Overlay layout.
    let framed = crate::frame::render_modal_framed_surface(
        frame,
        area,
        SurfaceMode::Route,
        &FramedSurfaceSpec {
            title: picker_title(picker, spec, &normalize_text).into(),
            target_width: area.width,
            target_height: framed_sections_target_height(&sections),
        },
    );
    let rows = split_vertical_sections(framed.inner, &sections);
    let mut row_index = 0;
    if prompt_height > 0 {
        frame.render_widget(
            Paragraph::new(normalize_text(&picker.prompt)).wrap(Wrap { trim: false }),
            rows[row_index],
        );
        row_index += 1;
    }
    let input_result = if input_height > 0 {
        crate::pointer::register(
            rows[row_index],
            Some(crate::pointer::PointerAction::PickerInput),
            None,
        );
        let editor_spec = EditorPanelSpec {
            title: (!spec.search_label.trim().is_empty()).then(|| spec.search_label.clone()),
            borders: Borders::ALL,
        };
        let result = if picker.config.wrap_input {
            crate::render_wrapped_editor_panel(frame, rows[row_index], &editor_spec, &picker.input)
        } else {
            render_editor_panel(frame, rows[row_index], &editor_spec, &picker.input)
        };
        row_index += 1;
        Some(result)
    } else {
        None
    };

    let panels_area = rows[row_index];
    row_index += 1;
    let show_preview = match picker.config.preview_mode {
        SearchPickerPreviewMode::Hidden => false,
        SearchPickerPreviewMode::Responsive {
            min_total_width, ..
        } => panels_area.width >= min_total_width,
    };
    let (list_area, preview_area) = if show_preview {
        let SearchPickerPreviewMode::Responsive {
            left_min_width,
            right_min_width,
            ..
        } = picker.config.preview_mode
        else {
            unreachable!()
        };
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(adaptive_detail_split(
                panels_area.width,
                left_min_width,
                right_min_width,
            ))
            .split(panels_area);
        (split[0], Some(split[1]))
    } else {
        (panels_area, None)
    };

    render_picker_results(frame, list_area, picker, spec, &normalize_text);
    if let Some(preview_area) = preview_area {
        crate::pointer::register(
            preview_area,
            None,
            Some((
                crate::pointer::PointerAction::PickerPreview(-3),
                crate::pointer::PointerAction::PickerPreview(3),
            )),
        );
        let search_failed = spec.chrome("search-picker-search-failed", "Search failed");
        let empty_body = spec.chrome("search-picker-no-preview", "No preview available");
        let view_state = picker_view_state(picker, spec.loading_message.as_ref(), &search_failed);
        let sections = build_preview(view_state);
        render_preview_sections(
            frame,
            preview_area,
            sections,
            picker.preview_scroll,
            spec.preview_title(),
            empty_body,
        );
    }
    if footer_height > 0 {
        crate::render_shortcut_footer(frame, rows[row_index], &footer);
    }
    if let Some(result) = input_result
        && picker.focus == SearchPickerFocus::Input
    {
        frame.set_cursor_position(result.cursor);
    }
}

/// Canonical centered window for every searchable selection surface.
/// Keep only a four-cell gutter so catalogs and preview panes can use nearly
/// the entire terminal instead of being constrained by a fixed-size cap.
/// Tiny terminals use the complete area rather than sacrificing usability.
pub fn search_picker_dialog_area(area: Rect) -> Rect {
    if area.width < 48 || area.height < 18 {
        return area;
    }
    Rect::new(
        area.x.saturating_add(4),
        area.y.saturating_add(4),
        area.width.saturating_sub(8),
        area.height.saturating_sub(8),
    )
}

fn picker_view_state<'a, TItem, TCustom, TMeta>(
    picker: &'a SearchPicker<TItem, TCustom, TMeta, Editor>,
    loading_message: &'a str,
    search_failed: &'a str,
) -> SearchPickerViewState<'a, TItem>
where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
{
    if picker.phase.hides_results() {
        SearchPickerViewState::Loading {
            message: loading_message,
        }
    } else if let SearchPickerPhase::Error {
        keep_results: false,
    } = picker.phase
    {
        SearchPickerViewState::Error {
            message: picker.error_message.as_deref().unwrap_or(search_failed),
        }
    } else if let Some(item) = picker.selected_item() {
        SearchPickerViewState::Selected(item)
    } else {
        SearchPickerViewState::Empty {
            message: &picker.empty_message,
        }
    }
}

fn render_picker_results<TItem, TCustom, TMeta, F>(
    frame: &mut Frame,
    area: Rect,
    picker: &SearchPicker<TItem, TCustom, TMeta, Editor>,
    spec: &SearchPickerDialogSpec<'_>,
    normalize_text: &F,
) where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'a> Fn(&'a str) -> String,
{
    let page_size = area.height.saturating_sub(2).max(1) as usize;
    picker.set_visible_page_size(page_size);
    let block_title = format!(
        " {} · {} · {} ",
        normalize_text(spec.results_title.as_ref()),
        picker.result_count(),
        spec.chrome_args(
            "search-picker-page",
            "Page {current}/{total}",
            &[
                ("current", (picker.current_page() + 1).to_string()),
                ("total", picker.page_count().to_string()),
            ],
        ),
    );
    let block = Block::default().borders(Borders::ALL).title(block_title);
    if picker.phase.hides_results() {
        frame.render_widget(
            Paragraph::new(normalize_text(spec.loading_message.as_ref()))
                .style(theme::muted_style())
                .block(block),
            area,
        );
        return;
    }
    if let SearchPickerPhase::Error {
        keep_results: false,
    } = picker.phase
    {
        let message = picker
            .error_message
            .clone()
            .unwrap_or_else(|| spec.chrome("search-picker-search-failed", "Search failed"));
        frame.render_widget(
            Paragraph::new(normalize_text(&message))
                .style(Style::default().fg(theme::danger_color()))
                .block(block),
            area,
        );
        return;
    }
    if picker.is_empty() {
        frame.render_widget(
            Paragraph::new(normalize_text(&picker.empty_message))
                .style(theme::muted_style())
                .wrap(Wrap { trim: false })
                .block(block),
            area,
        );
        return;
    }

    let (start, end) = picker.visible_page_bounds();
    let inner = block.inner(area);
    crate::pointer::register(
        inner,
        Some(crate::pointer::PointerAction::PickerResults),
        None,
    );
    for (line, index) in (start..end).enumerate().take(usize::from(inner.height)) {
        crate::pointer::register(
            Rect::new(inner.x, inner.y + line as u16, inner.width, 1),
            Some(crate::pointer::PointerAction::PickerRow(index)),
            None,
        );
    }
    let row_width = area.width.saturating_sub(5).max(1) as usize;
    let list_items = (start..end)
        .map(|row| {
            build_picker_row(
                picker,
                row,
                row_width,
                spec.checked_symbol.as_ref(),
                spec.unchecked_symbol.as_ref(),
                normalize_text,
            )
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state.select(Some(picker.selected.saturating_sub(start)));
    let focused = picker.focus == SearchPickerFocus::Results;
    let highlight_symbol = if focused {
        spec.highlight_symbol.to_string()
    } else {
        " ".repeat(UnicodeWidthStr::width(spec.highlight_symbol.as_ref()))
    };
    let list = List::new(list_items)
        .block(block)
        .highlight_style(if focused {
            spec.highlight_style
        } else {
            Style::default()
        })
        .highlight_symbol(highlight_symbol);
    frame.render_stateful_widget(list, area, &mut state);
}

fn build_picker_row<'a, TItem, TCustom, TMeta, F>(
    picker: &'a SearchPicker<TItem, TCustom, TMeta, Editor>,
    row: usize,
    width: usize,
    checked_symbol: &str,
    unchecked_symbol: &str,
    normalize_text: &F,
) -> ListItem<'static>
where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'b> Fn(&'b str) -> String,
{
    let mut index = row;
    if let Some(clear) = picker.clear_action.as_ref() {
        if index == 0 {
            let label = if clear.current {
                format!("✓ {}", clear.label)
            } else {
                clear.label.clone()
            };
            return row_item(
                normalize_text(&label),
                Some(normalize_text(&clear.detail)),
                Style::default().fg(theme::warning_color()),
                theme::muted_style(),
            );
        }
        index -= 1;
    }
    if let Some(custom) = picker.custom_value() {
        if index == 0 {
            return row_item(
                normalize_text(custom.search_picker_label(&picker.meta).as_ref()),
                custom
                    .search_picker_detail(&picker.meta)
                    .map(|detail| normalize_text(detail.as_ref())),
                custom.search_picker_label_style(),
                custom.search_picker_detail_style(),
            );
        }
        index -= 1;
    }
    let Some(matched) = picker.matches.get(index) else {
        return ListItem::new("");
    };
    let Some(item) = picker.items.get(matched.item_index) else {
        return ListItem::new("");
    };
    let checked_prefix = if picker.config.selection_mode == SearchPickerSelectionMode::Multiple {
        if picker
            .checked_keys
            .iter()
            .any(|key| key == item.search_picker_key().as_ref())
        {
            checked_symbol
        } else {
            unchecked_symbol
        }
    } else {
        ""
    };
    let prefix = item
        .search_picker_prefix()
        .map(|prefix| normalize_text(prefix.as_ref()))
        .unwrap_or_default();
    let leading_width = UnicodeWidthStr::width(checked_prefix)
        .saturating_add(UnicodeWidthStr::width(prefix.as_str()));
    let label = normalize_text(item.search_picker_label().as_ref());
    let label = truncate_display_text(&label, width.saturating_sub(leading_width));
    let mut first_line = Line::from(Vec::<Span<'static>>::new());
    if !checked_prefix.is_empty() {
        first_line.spans.push(Span::raw(checked_prefix.to_string()));
    }
    if !prefix.is_empty() {
        first_line
            .spans
            .push(Span::styled(prefix, item.search_picker_prefix_style()));
    }
    first_line.spans.extend(
        highlighted_line(
            "",
            &label,
            &matched.label_ranges,
            item.search_picker_label_style(),
        )
        .spans,
    );
    let detail = item
        .search_picker_disabled_reason()
        .or_else(|| item.search_picker_detail());
    let detail_style = if item.search_picker_disabled_reason().is_some() {
        Style::default().fg(theme::warning_color())
    } else {
        item.search_picker_detail_style()
    };
    if let Some(detail) = detail {
        let used = UnicodeWidthStr::width(label.as_str()).saturating_add(leading_width);
        let detail = truncate_display_text(
            &normalize_text(detail.as_ref()),
            width.saturating_sub(used).saturating_sub(2),
        );
        if !detail.is_empty() {
            first_line.spans.push(Span::raw("  "));
            first_line.spans.push(Span::styled(detail, detail_style));
        }
    }
    ListItem::new(first_line)
}

fn row_item(
    label: String,
    detail: Option<String>,
    label_style: Style,
    detail_style: Style,
) -> ListItem<'static> {
    let mut spans = vec![Span::styled(label, label_style)];
    if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(detail, detail_style));
    }
    ListItem::new(Line::from(spans))
}

fn highlighted_line(
    prefix: &str,
    label: &str,
    ranges: &[(usize, usize)],
    base: Style,
) -> Line<'static> {
    let chars = label.chars().collect::<Vec<_>>();
    let mut spans = Vec::new();
    if !prefix.is_empty() {
        spans.push(Span::styled(prefix.to_string(), theme::muted_style()));
    }
    let mut cursor = 0usize;
    for &(start, end) in ranges {
        let start = start.min(chars.len()).max(cursor);
        let end = end.min(chars.len()).max(start);
        if start > cursor {
            spans.push(Span::styled(
                chars[cursor..start].iter().collect::<String>(),
                base,
            ));
        }
        if end > start {
            spans.push(Span::styled(
                chars[start..end].iter().collect::<String>(),
                base.fg(theme::accent_color()).add_modifier(Modifier::BOLD),
            ));
        }
        cursor = end;
    }
    if cursor < chars.len() {
        spans.push(Span::styled(
            chars[cursor..].iter().collect::<String>(),
            base,
        ));
    }
    Line::from(spans)
}

fn render_preview_sections(
    frame: &mut Frame,
    area: Rect,
    sections: Vec<PreviewSection<'static>>,
    scroll: u16,
    default_title: Cow<'_, str>,
    empty_body: String,
) {
    if sections.is_empty() {
        let empty = Text::from(empty_body);
        render_text_panel(
            frame,
            area,
            &TextPanelSpec {
                title: Some(default_title),
                body: &empty,
                wrap: true,
                scroll: None,
                alignment: None,
            },
        );
        return;
    }
    let constraints = sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            if index + 1 == sections.len() {
                ratatui::layout::Constraint::Min(section.min_body_height.saturating_add(2))
            } else {
                ratatui::layout::Constraint::Length(section.max_body_height.saturating_add(2))
            }
        })
        .collect::<Vec<_>>();
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    for (index, (section, section_area)) in sections.into_iter().zip(areas.iter()).enumerate() {
        render_text_panel(
            frame,
            *section_area,
            &TextPanelSpec {
                title: Some(section.title),
                body: &section.body,
                wrap: true,
                scroll: (index + 1 == areas.len()).then_some((scroll, 0)),
                alignment: None,
            },
        );
    }
}

fn picker_title<TItem, TCustom, TMeta, F>(
    picker: &SearchPicker<TItem, TCustom, TMeta, Editor>,
    spec: &SearchPickerDialogSpec<'_>,
    normalize_text: &F,
) -> String
where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'a> Fn(&'a str) -> String,
{
    let status = match picker.phase {
        SearchPickerPhase::Searching { .. } => spec.chrome("search-picker-searching", "searching…"),
        SearchPickerPhase::Appending => spec.chrome("search-picker-loading-more", "loading more…"),
        SearchPickerPhase::Error { .. } => spec.chrome("search-picker-error", "error"),
        _ => spec.chrome_args(
            "search-picker-results",
            "{count} results",
            &[("count", picker.result_count().to_string())],
        ),
    };
    normalize_text(&format!("{} · {}", picker.title, status))
}

fn picker_footer<TItem, TCustom, TMeta, F>(
    picker: &SearchPicker<TItem, TCustom, TMeta, Editor>,
    spec: &SearchPickerDialogSpec<'_>,
    normalize_text: &F,
) -> String
where
    TItem: SearchPickerItem,
    TCustom: SearchPickerCustomValue<TMeta>,
    F: for<'a> Fn(&'a str) -> String,
{
    let mut parts = Vec::new();
    if picker.config.selection_mode == SearchPickerSelectionMode::Multiple {
        parts.push(spec.chrome_args(
            "search-picker-footer-space",
            "Space toggle · {count} selected",
            &[("count", picker.checked_keys.len().to_string())],
        ));
        parts.push(spec.chrome("search-picker-footer-enter", "Enter confirm"));
    }
    if picker.config.input_mode.is_visible() {
        parts.push(spec.chrome(
            "search-picker-footer-navigate-page",
            "↑/↓ navigate · ←/→ page",
        ));
    } else {
        parts.push(spec.chrome("search-picker-footer-page", "←/→ page"));
    }
    if !picker.footer.trim().is_empty() {
        parts.push(normalize_text(&picker.footer));
    } else {
        parts.push(format!(
            "{} · {} · {}",
            spec.chrome("search-picker-footer-navigate", "↑/↓ navigate"),
            spec.chrome("search-picker-footer-enter-select", "Enter select"),
            spec.chrome("search-picker-footer-close", "Esc close"),
        ));
    }
    parts.join(" · ")
}

pub(super) fn normalize_search_text(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric()
                || matches!(character, '/' | '\\' | '.' | '-' | '_' | '#' | ':')
            {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn score_entry(entry: &SearchIndexEntry, tokens: &[&str]) -> Option<i64> {
    if entry.always_visible {
        return Some(10_000);
    }
    let mut total = 0i64;
    for token in tokens {
        let label_score = score_field(&entry.normalized_label, token, true);
        let document_score = score_field(&entry.normalized_document, token, false);
        let score = match (label_score, document_score) {
            (Some(label), Some(document)) => label.max(document),
            (Some(label), None) => label,
            (None, Some(document)) => document,
            (None, None) => return None,
        };
        total += score;
    }
    Some(total)
}

fn score_field(field: &str, token: &str, primary: bool) -> Option<i64> {
    let weight = if primary { 1_000 } else { 100 };
    if field == token {
        return Some(weight + 500);
    }
    if field.starts_with(token) {
        return Some(weight + 400 - field.len().saturating_sub(token.len()) as i64);
    }
    if field
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| word.starts_with(token))
    {
        return Some(weight + 300);
    }
    if let Some(position) = field.find(token) {
        return Some(weight + 200 - position.min(100) as i64);
    }
    subsequence_score(field, token).map(|score| weight + score)
}

fn subsequence_score(field: &str, token: &str) -> Option<i64> {
    let mut field_chars = field.chars().enumerate();
    let mut last = None;
    let mut gaps = 0usize;
    for wanted in token.chars() {
        let (position, _) = field_chars.find(|(_, character)| *character == wanted)?;
        if let Some(previous) = last {
            gaps += position.saturating_sub(previous + 1);
        }
        last = Some(position);
    }
    Some(100 - gaps.min(100) as i64)
}

pub(super) fn find_label_ranges(label: &str, tokens: &[&str]) -> Vec<(usize, usize)> {
    let normalized_chars = label
        .chars()
        .map(|character| character.to_lowercase().collect::<String>())
        .collect::<Vec<_>>();
    let mut ranges = Vec::new();
    for token in tokens {
        let token_chars = token.chars().collect::<Vec<_>>();
        if token_chars.is_empty() {
            continue;
        }
        for start in 0..normalized_chars.len() {
            let mut normalized = String::new();
            let mut end = start;
            while end < normalized_chars.len() && normalized.chars().count() < token_chars.len() {
                normalized.push_str(&normalized_chars[end]);
                end += 1;
            }
            if normalized == *token {
                ranges.push((start, end));
                break;
            }
        }
    }
    ranges.sort_unstable_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.0 <= last.1
        {
            last.1 = last.1.max(range.1);
        } else {
            merged.push(range);
        }
    }
    merged
}
