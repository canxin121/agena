use std::sync::{OnceLock, RwLock};

use ratatui::style::{Color, Modifier, Style};

/// The terminal appearance Agena should optimize its semantic colors for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColorScheme {
    #[default]
    Dark,
    Light,
}

/// An RGB color reported by the terminal emulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalRgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl TerminalRgb {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    pub fn is_light(self) -> bool {
        // At this crossover either black or white can still reach WCAG's 4.5:1
        // normal-text contrast target. It is more reliable than classifying
        // medium gray backgrounds by their encoded RGB brightness.
        relative_luminance(self) >= 0.179
    }
}

/// Background-independent semantic colors shared by the TUI and its reusable
/// components. The terminal's own default foreground/background remain in
/// charge of ordinary text and empty cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemePalette {
    pub scheme: ColorScheme,
    pub muted: Color,
    pub accent: Color,
    pub info: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub special: Color,
    pub selection_fg: Color,
    pub selection_bg: Color,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ThemeOverrides {
    pub muted: Option<Color>,
    pub accent: Option<Color>,
    pub info: Option<Color>,
    pub success: Option<Color>,
    pub warning: Option<Color>,
    pub danger: Option<Color>,
    pub special: Option<Color>,
    pub selection_fg: Option<Color>,
    pub selection_bg: Option<Color>,
}

impl Default for ThemePalette {
    fn default() -> Self {
        Self::for_scheme(ColorScheme::Dark)
    }
}

impl ThemePalette {
    pub fn for_scheme(scheme: ColorScheme) -> Self {
        match scheme {
            ColorScheme::Dark => Self {
                scheme,
                muted: rgb(166, 173, 186),
                accent: rgb(116, 192, 252),
                info: rgb(102, 217, 239),
                success: rgb(101, 211, 145),
                warning: rgb(245, 194, 107),
                danger: rgb(255, 123, 114),
                special: rgb(203, 166, 247),
                selection_fg: rgb(248, 250, 252),
                selection_bg: rgb(36, 88, 128),
            },
            ColorScheme::Light => Self {
                scheme,
                muted: rgb(82, 89, 102),
                accent: rgb(0, 91, 161),
                info: rgb(0, 105, 120),
                success: rgb(18, 119, 61),
                warning: rgb(128, 82, 0),
                danger: rgb(180, 35, 24),
                special: rgb(111, 66, 193),
                selection_fg: rgb(15, 35, 55),
                selection_bg: rgb(190, 222, 248),
            },
        }
    }

    pub fn for_background(background: TerminalRgb) -> Self {
        let mut palette = Self::for_scheme(if background.is_light() {
            ColorScheme::Light
        } else {
            ColorScheme::Dark
        });
        palette.muted = ensure_text_contrast(palette.muted, background);
        palette.accent = ensure_text_contrast(palette.accent, background);
        palette.info = ensure_text_contrast(palette.info, background);
        palette.success = ensure_text_contrast(palette.success, background);
        palette.warning = ensure_text_contrast(palette.warning, background);
        palette.danger = ensure_text_contrast(palette.danger, background);
        palette.special = ensure_text_contrast(palette.special, background);
        palette
    }

    pub fn with_overrides(mut self, overrides: ThemeOverrides) -> Self {
        apply_color_override(&mut self.muted, overrides.muted);
        apply_color_override(&mut self.accent, overrides.accent);
        apply_color_override(&mut self.info, overrides.info);
        apply_color_override(&mut self.success, overrides.success);
        apply_color_override(&mut self.warning, overrides.warning);
        apply_color_override(&mut self.danger, overrides.danger);
        apply_color_override(&mut self.special, overrides.special);
        apply_color_override(&mut self.selection_fg, overrides.selection_fg);
        apply_color_override(&mut self.selection_bg, overrides.selection_bg);
        self
    }
}

fn apply_color_override(target: &mut Color, value: Option<Color>) {
    if let Some(color) = value {
        *target = color;
    }
}

fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color::Rgb(red, green, blue)
}

static ACTIVE_PALETTE: OnceLock<RwLock<ThemePalette>> = OnceLock::new();

fn palette_lock() -> &'static RwLock<ThemePalette> {
    ACTIVE_PALETTE.get_or_init(|| RwLock::new(ThemePalette::for_scheme(ColorScheme::Dark)))
}

pub fn set_active_palette(palette: ThemePalette) {
    match palette_lock().write() {
        Ok(mut active) => *active = palette,
        Err(poisoned) => *poisoned.into_inner() = palette,
    }
}

pub fn active_palette() -> ThemePalette {
    match palette_lock().read() {
        Ok(active) => *active,
        Err(poisoned) => *poisoned.into_inner(),
    }
}

pub fn muted_color() -> Color {
    active_palette().muted
}

pub fn accent_color() -> Color {
    active_palette().accent
}

pub fn info_color() -> Color {
    active_palette().info
}

pub fn success_color() -> Color {
    active_palette().success
}

pub fn warning_color() -> Color {
    active_palette().warning
}

pub fn danger_color() -> Color {
    active_palette().danger
}

pub fn special_color() -> Color {
    active_palette().special
}

pub fn muted_style() -> Style {
    Style::default().fg(muted_color())
}

pub fn selection_style() -> Style {
    let palette = active_palette();
    Style::default()
        .fg(palette.selection_fg)
        .bg(palette.selection_bg)
        .add_modifier(Modifier::BOLD)
}

pub fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();
    match value {
        "reset" => Some(Color::Reset),
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" => Some(Color::Gray),
        "dark_gray" => Some(Color::DarkGray),
        "light_red" => Some(Color::LightRed),
        "light_green" => Some(Color::LightGreen),
        "light_yellow" => Some(Color::LightYellow),
        "light_blue" => Some(Color::LightBlue),
        "light_magenta" => Some(Color::LightMagenta),
        "light_cyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => parse_hex_color(value),
    }
}

fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(red, green, blue))
}

fn relative_luminance(color: TerminalRgb) -> f64 {
    fn channel(value: u8) -> f64 {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    0.2126 * channel(color.red) + 0.7152 * channel(color.green) + 0.0722 * channel(color.blue)
}

fn contrast_ratio(a: TerminalRgb, b: TerminalRgb) -> f64 {
    let (lighter, darker) = {
        let a = relative_luminance(a);
        let b = relative_luminance(b);
        if a >= b { (a, b) } else { (b, a) }
    };
    (lighter + 0.05) / (darker + 0.05)
}

fn ensure_text_contrast(color: Color, background: TerminalRgb) -> Color {
    let Color::Rgb(red, green, blue) = color else {
        return color;
    };
    let source = TerminalRgb::new(red, green, blue);
    if contrast_ratio(source, background) >= 4.5 {
        return color;
    }

    let target = if background.is_light() { 0.0 } else { 255.0 };
    for step in 1..=100 {
        let amount = f64::from(step) / 100.0;
        let mix = |channel: u8| {
            (f64::from(channel) + (target - f64::from(channel)) * amount).round() as u8
        };
        let candidate = TerminalRgb::new(mix(red), mix(green), mix(blue));
        if contrast_ratio(candidate, background) >= 4.5 {
            return rgb(candidate.red, candidate.green, candidate.blue);
        }
    }

    rgb(target as u8, target as u8, target as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb_from_color(color: Color) -> TerminalRgb {
        let Color::Rgb(red, green, blue) = color else {
            panic!("expected RGB color, got {color:?}");
        };
        TerminalRgb::new(red, green, blue)
    }

    #[test]
    fn detects_light_and_dark_backgrounds_by_luminance() {
        assert!(!TerminalRgb::new(30, 30, 30).is_light());
        assert!(TerminalRgb::new(245, 245, 245).is_light());
    }

    #[test]
    fn semantic_colors_have_readable_contrast_on_reference_backgrounds() {
        for (scheme, background) in [
            (ColorScheme::Dark, TerminalRgb::new(24, 24, 27)),
            (ColorScheme::Light, TerminalRgb::new(250, 250, 250)),
        ] {
            let palette = ThemePalette::for_scheme(scheme);
            for color in [
                palette.muted,
                palette.accent,
                palette.info,
                palette.success,
                palette.warning,
                palette.danger,
                palette.special,
            ] {
                assert!(
                    contrast_ratio(rgb_from_color(color), background) >= 4.5,
                    "{scheme:?} color {color:?} lacks text contrast"
                );
            }
            assert!(
                contrast_ratio(
                    rgb_from_color(palette.selection_fg),
                    rgb_from_color(palette.selection_bg)
                ) >= 4.5
            );
        }
    }

    #[test]
    fn detected_colored_and_mid_tone_backgrounds_are_contrast_corrected() {
        for background in [
            TerminalRgb::new(96, 96, 96),
            TerminalRgb::new(130, 105, 70),
            TerminalRgb::new(30, 70, 90),
            TerminalRgb::new(235, 225, 190),
        ] {
            let palette = ThemePalette::for_background(background);
            for color in [
                palette.muted,
                palette.accent,
                palette.info,
                palette.success,
                palette.warning,
                palette.danger,
                palette.special,
            ] {
                assert!(contrast_ratio(rgb_from_color(color), background) >= 4.5);
            }
        }
    }

    #[test]
    fn plugin_overrides_use_only_the_new_canonical_schema() {
        let original = ThemePalette::for_scheme(ColorScheme::Dark);
        let palette = original.with_overrides(ThemeOverrides {
            accent: parse_color("#123456"),
            danger: parse_color("light_red"),
            ..ThemeOverrides::default()
        });
        assert_eq!(palette.accent, Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(palette.danger, Color::LightRed);
        assert_eq!(parse_color("default"), None);
        assert_eq!(parse_color("light-red"), None);
    }
}
