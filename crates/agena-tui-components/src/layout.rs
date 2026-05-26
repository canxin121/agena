use std::cmp::{max, min};

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use unicode_width::UnicodeWidthStr;

use crate::Editor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalSectionSize {
    Fixed(u16),
    Flexible(u16),
}

#[derive(Clone, Copy)]
pub enum SurfaceMode {
    Overlay,
    Route,
}

impl SurfaceMode {
    pub fn outer_width(self, area: Rect, target_width: u16) -> u16 {
        match self {
            Self::Overlay => adaptive_modal_width(area.width, target_width),
            Self::Route => area.width,
        }
    }

    pub fn content_width(self, area: Rect, target_width: u16) -> u16 {
        self.outer_width(area, target_width).saturating_sub(2)
    }

    pub fn outer_rect(self, area: Rect, target_width: u16, target_height: u16) -> Rect {
        match self {
            Self::Overlay => preferred_overlay_rect(area, target_width, target_height),
            Self::Route => area,
        }
    }
}

pub fn preferred_overlay_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = adaptive_modal_width(area.width, width);
    let height = adaptive_modal_height(area.height, height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

pub fn framed_overlay_height(content_height: u16) -> u16 {
    content_height.saturating_add(2)
}

pub fn top_aligned_panel_rect(area: Rect, panel_height: u16) -> Rect {
    top_aligned_vertical_areas(area, &[panel_height])
        .into_iter()
        .next()
        .unwrap_or(area)
}

pub fn inset_rect(area: Rect, horizontal: u16, vertical: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(horizontal),
        y: area.y.saturating_add(vertical),
        width: area.width.saturating_sub(horizontal.saturating_mul(2)),
        height: area.height.saturating_sub(vertical.saturating_mul(2)),
    }
}

pub fn adaptive_modal_width(total_width: u16, target: u16) -> u16 {
    let max_width = total_width.saturating_sub(2).max(1);
    if total_width <= 72 {
        min(max_width, max(36, target))
    } else if total_width <= 96 {
        min(max_width, max(44, target.saturating_sub(10)))
    } else {
        min(target, max_width)
    }
}

pub fn adaptive_modal_height(total_height: u16, target: u16) -> u16 {
    let max_height = total_height.saturating_sub(2).max(1);
    if total_height <= 18 {
        min(max_height, max(6, target))
    } else if total_height <= 28 {
        min(max_height, max(8, target))
    } else {
        min(target, max_height)
    }
}

pub fn adaptive_detail_split(total_width: u16, left_min: u16, right_min: u16) -> [Constraint; 2] {
    if should_stack_detail_layout(total_width, left_min, right_min) {
        [Constraint::Percentage(50), Constraint::Percentage(50)]
    } else {
        let [left_pct, right_pct] = proportional_percentages(left_min, right_min);
        [
            Constraint::Percentage(left_pct),
            Constraint::Percentage(right_pct),
        ]
    }
}

pub fn should_stack_detail_layout(total_width: u16, left_min: u16, right_min: u16) -> bool {
    let available = total_width.saturating_sub(2);
    available < left_min.saturating_add(right_min).saturating_add(8)
}

pub fn estimated_horizontal_panel_widths(
    total_width: u16,
    left_min: u16,
    right_min: u16,
) -> (u16, u16) {
    if should_stack_detail_layout(total_width, left_min, right_min) {
        return (total_width.max(1), total_width.max(1));
    }

    let [left_pct, _right_pct] = proportional_percentages(left_min, right_min);
    let left_width = ((u32::from(total_width) * u32::from(left_pct)) / 100) as u16;
    let right_width = total_width.saturating_sub(left_width);
    (left_width.max(1), right_width.max(1))
}

pub fn wrapped_text_height(text: &str, width: u16) -> u16 {
    let usable_width = usize::from(width.max(1));
    text.lines()
        .map(|line| {
            let display_width = UnicodeWidthStr::width(line);
            let rows = display_width.max(1).div_ceil(usable_width);
            u16::try_from(rows).unwrap_or(u16::MAX)
        })
        .sum::<u16>()
        .max(1)
}

pub fn bordered_paragraph_height(text: &str, width: u16, min_body: u16, max_body: u16) -> u16 {
    wrapped_text_height(text, width.saturating_sub(2))
        .clamp(min_body, max_body)
        .saturating_add(2)
}

pub fn overlay_text_height(text: &str, width: u16, min_body: u16, max_body: u16) -> u16 {
    wrapped_text_height(text, width).clamp(min_body, max_body)
}

pub fn optional_overlay_text_height(text: &str, width: u16, min_body: u16, max_body: u16) -> u16 {
    if text.trim().is_empty() {
        0
    } else {
        overlay_text_height(text, width, min_body, max_body)
    }
}

pub fn list_panel_height(
    item_count: usize,
    lines_per_item: u16,
    min_body: u16,
    max_body: u16,
) -> u16 {
    let natural_lines = u16::try_from(item_count)
        .unwrap_or(u16::MAX)
        .saturating_mul(lines_per_item)
        .max(1);
    let relaxed_min_body =
        min_body.min(natural_lines.saturating_add(lines_per_item.saturating_sub(1)));
    let lines = natural_lines.clamp(relaxed_min_body, max_body);
    lines.saturating_add(2)
}

pub fn editor_input_panel_height(editor: &Editor, multiline: bool) -> u16 {
    if !multiline {
        return 3;
    }
    u16::try_from(max(1, editor.logical_line_count()))
        .unwrap_or(u16::MAX)
        .clamp(4, 8)
        .saturating_add(2)
}

impl VerticalSectionSize {
    fn base_height(self) -> u16 {
        match self {
            Self::Fixed(height) | Self::Flexible(height) => height,
        }
    }

    fn constraint(self) -> Constraint {
        match self {
            Self::Fixed(height) => Constraint::Length(height),
            Self::Flexible(height) => Constraint::Min(height),
        }
    }
}

pub fn vertical_sections_base_height(sections: &[VerticalSectionSize]) -> u16 {
    sections
        .iter()
        .copied()
        .map(VerticalSectionSize::base_height)
        .fold(0_u16, u16::saturating_add)
}

pub fn framed_sections_target_height(sections: &[VerticalSectionSize]) -> u16 {
    framed_overlay_height(vertical_sections_base_height(sections))
}

pub fn split_vertical_sections(area: Rect, sections: &[VerticalSectionSize]) -> Vec<Rect> {
    let has_flexible_section = sections
        .iter()
        .any(|section| matches!(section, VerticalSectionSize::Flexible(_)));
    let mut constraints = sections
        .iter()
        .copied()
        .map(VerticalSectionSize::constraint)
        .collect::<Vec<_>>();
    if !has_flexible_section {
        constraints.push(Constraint::Min(0));
    }

    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
        .iter()
        .take(sections.len())
        .copied()
        .collect()
}

pub fn top_aligned_vertical_areas(area: Rect, heights: &[u16]) -> Vec<Rect> {
    let sections = heights
        .iter()
        .copied()
        .map(VerticalSectionSize::Fixed)
        .collect::<Vec<_>>();
    split_vertical_sections(area, &sections)
}

fn proportional_percentages(first: u16, second: u16) -> [u16; 2] {
    let total = u32::from(first.max(1)).saturating_add(u32::from(second.max(1)));
    let first_pct = ((u32::from(first.max(1)) * 100) / total).clamp(1, 99) as u16;
    [first_pct, 100_u16.saturating_sub(first_pct)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_modal_width_never_exceeds_available_width() {
        assert_eq!(adaptive_modal_width(30, 96), 28);
    }

    #[test]
    fn adaptive_modal_height_preserves_requested_height_when_space_is_available() {
        assert_eq!(adaptive_modal_height(24, 19), 19);
    }

    #[test]
    fn adaptive_modal_height_clamps_to_available_height() {
        assert_eq!(adaptive_modal_height(24, 40), 22);
    }

    #[test]
    fn framed_overlay_height_accounts_for_outer_borders() {
        assert_eq!(framed_overlay_height(7), 9);
    }

    #[test]
    fn optional_overlay_text_height_returns_zero_for_blank_text() {
        assert_eq!(optional_overlay_text_height("   ", 40, 1, 2), 0);
    }

    #[test]
    fn estimated_horizontal_panel_widths_follow_stacked_layout() {
        let (left, right) = estimated_horizontal_panel_widths(40, 34, 48);
        assert_eq!((left, right), (40, 40));
    }

    #[test]
    fn estimated_horizontal_panel_widths_preserve_total_width() {
        let (left, right) = estimated_horizontal_panel_widths(114, 48, 34);
        assert_eq!(left + right, 114);
        assert!(left > right);
    }

    #[test]
    fn multiline_editor_input_panel_height_grows_with_content() {
        let mut editor = Editor::default();
        editor.set_text("first\nsecond\nthird".to_string());

        assert_eq!(editor_input_panel_height(&editor, false), 3);
        assert_eq!(editor_input_panel_height(&editor, true), 6);
    }

    #[test]
    fn split_vertical_sections_top_aligns_fixed_sections() {
        let areas = split_vertical_sections(
            Rect::new(0, 0, 20, 12),
            &[VerticalSectionSize::Fixed(2), VerticalSectionSize::Fixed(3)],
        );

        assert_eq!(areas[0], Rect::new(0, 0, 20, 2));
        assert_eq!(areas[1], Rect::new(0, 2, 20, 3));
    }

    #[test]
    fn split_vertical_sections_gives_extra_space_to_flexible_section() {
        let areas = split_vertical_sections(
            Rect::new(0, 0, 20, 12),
            &[
                VerticalSectionSize::Fixed(2),
                VerticalSectionSize::Flexible(3),
            ],
        );

        assert_eq!(areas[0], Rect::new(0, 0, 20, 2));
        assert_eq!(areas[1], Rect::new(0, 2, 20, 10));
    }

    #[test]
    fn framed_sections_target_height_uses_section_base_heights() {
        assert_eq!(
            framed_sections_target_height(&[
                VerticalSectionSize::Fixed(2),
                VerticalSectionSize::Flexible(5),
            ]),
            9
        );
    }
}
