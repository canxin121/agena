use std::sync::{OnceLock, RwLock};

use ratatui::style::{Color, Modifier, Style};

// Small terminal text must meet WCAG AA. The palette seeds already clear this
// threshold on the reference canvases; detected custom backgrounds are
// corrected at runtime.
const MINIMUM_TEXT_CONTRAST: f64 = 4.5;
// The border remains the primary modal boundary, but the fill also needs to
// read as a separate surface. 1.5:1 is deliberately stronger than a barely
// perceptible tint while remaining quiet enough for a terminal-sized dialog.
const MINIMUM_SURFACE_CONTRAST: f64 = 1.5;
const DARK_REFERENCE_BACKGROUND: TerminalRgb = TerminalRgb::new(24, 24, 27);
const LIGHT_REFERENCE_BACKGROUND: TerminalRgb = TerminalRgb::new(250, 250, 250);

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

impl ColorScheme {
    /// Stable backing color for content that must be rasterized before the
    /// terminal can draw it. Ordinary cells should continue to use the
    /// terminal defaults, but opaque formula canvases and other generated
    /// graphics need a concrete light/dark color.
    pub const fn reference_background(self) -> TerminalRgb {
        match self {
            Self::Dark => DARK_REFERENCE_BACKGROUND,
            Self::Light => LIGHT_REFERENCE_BACKGROUND,
        }
    }
}

/// Semantic colors shared by the TUI and its reusable components. The
/// terminal's own default foreground/background remain in charge of ordinary
/// text and empty cells; [`ThemePalette::for_background`] adapts the colored
/// roles and selection surface to the detected canvas.
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
    pub code_fg: Color,
    pub code_bg: Color,
    /// Foreground used by modal surfaces such as permission and destructive
    /// action confirmations.
    pub modal_fg: Color,
    /// An elevated surface color that remains visibly separate from the
    /// terminal's chat canvas.
    pub modal_bg: Color,
    pub modal_border: Color,
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
        Self::for_scheme_and_background(scheme, scheme.reference_background())
    }

    /// Conservative palette for terminals that do not report their default
    /// background. Code surfaces keep the terminal's own foreground and
    /// background instead of guessing a dark or light canvas.
    pub fn for_unknown_background() -> Self {
        let mut palette = Self::for_scheme(ColorScheme::Dark);
        palette.code_fg = Color::Reset;
        palette.code_bg = Color::Reset;
        palette
    }

    fn for_scheme_and_background(scheme: ColorScheme, background: TerminalRgb) -> Self {
        let modal_background = modal_background(scheme, background);
        match scheme {
            ColorScheme::Dark => Self {
                scheme,
                // Apple's guidance favors semantic, appearance-adaptive colors
                // over copying fixed system RGB values. Primer supplies
                // concrete light/dark tokens designed for a code-heavy UI.
                muted: rgb(145, 152, 161),
                accent: rgb(68, 147, 248),
                info: rgb(68, 147, 248),
                success: rgb(63, 185, 80),
                warning: rgb(210, 153, 34),
                danger: rgb(248, 81, 73),
                special: rgb(171, 125, 248),
                selection_fg: rgb(240, 246, 252),
                selection_bg: selection_background(scheme, background),
                code_fg: rgb(230, 237, 243),
                code_bg: rgb(13, 17, 23),
                modal_fg: readable_text_color(rgb(240, 246, 252), modal_background),
                modal_bg: rgb_color_value(modal_background),
                modal_border: readable_text_color(rgb(68, 147, 248), modal_background),
            },
            ColorScheme::Light => Self {
                scheme,
                muted: rgb(89, 99, 110),
                accent: rgb(9, 105, 218),
                info: rgb(9, 105, 218),
                success: rgb(26, 127, 55),
                warning: rgb(154, 103, 0),
                danger: rgb(209, 36, 47),
                special: rgb(130, 80, 223),
                selection_fg: rgb(31, 35, 40),
                selection_bg: selection_background(scheme, background),
                code_fg: rgb(31, 35, 40),
                code_bg: rgb(246, 248, 250),
                modal_fg: readable_text_color(rgb(31, 35, 40), modal_background),
                modal_bg: rgb_color_value(modal_background),
                modal_border: readable_text_color(rgb(9, 105, 218), modal_background),
            },
        }
    }

    pub fn for_background(background: TerminalRgb) -> Self {
        let scheme = if background.is_light() {
            ColorScheme::Light
        } else {
            ColorScheme::Dark
        };
        let mut palette = Self::for_scheme_and_background(scheme, background);
        palette.muted = readable_text_color(palette.muted, background);
        palette.accent = readable_text_color(palette.accent, background);
        palette.info = readable_text_color(palette.info, background);
        palette.success = readable_text_color(palette.success, background);
        palette.warning = readable_text_color(palette.warning, background);
        palette.danger = readable_text_color(palette.danger, background);
        palette.special = readable_text_color(palette.special, background);
        palette.selection_fg =
            readable_text_color(palette.selection_fg, rgb_color(palette.selection_bg));
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

fn selection_background(scheme: ColorScheme, background: TerminalRgb) -> Color {
    // Primer uses a translucent accent selection rather than a solid, glaring
    // control fill. Composite it here because terminal cells have no alpha.
    let (overlay, alpha) = match scheme {
        ColorScheme::Dark => (TerminalRgb::new(31, 111, 235), 0.70),
        ColorScheme::Light => (TerminalRgb::new(9, 105, 218), 0.20),
    };
    let blend = |top: u8, bottom: u8| {
        (f64::from(top) * alpha + f64::from(bottom) * (1.0 - alpha)).round() as u8
    };
    rgb(
        blend(overlay.red, background.red),
        blend(overlay.green, background.green),
        blend(overlay.blue, background.blue),
    )
}

fn modal_background(scheme: ColorScheme, background: TerminalRgb) -> TerminalRgb {
    // Terminals have no alpha channel, so pre-composite a neutral elevation.
    // Dark canvases are lifted and light canvases are lowered. Start with the
    // preferred tint, then strengthen it when a custom or mid-tone terminal
    // background would otherwise make the dialog disappear into the chat
    // canvas.
    let (preferred_target, fallback_target, initial_step) = match scheme {
        ColorScheme::Dark => (255.0, 0.0, 14),
        ColorScheme::Light => (0.0, 255.0, 8),
    };
    for (target, first_step) in [(preferred_target, initial_step), (fallback_target, 1)] {
        for step in first_step..=100 {
            let amount = f64::from(step) / 100.0;
            let mix = |channel: u8| {
                (f64::from(channel) + (target - f64::from(channel)) * amount).round() as u8
            };
            let candidate = TerminalRgb::new(
                mix(background.red),
                mix(background.green),
                mix(background.blue),
            );
            if contrast_ratio(candidate, background) >= MINIMUM_SURFACE_CONTRAST {
                return candidate;
            }
        }
    }

    unreachable!("black or white must contrast with every sRGB background")
}

fn rgb_color_value(color: TerminalRgb) -> Color {
    rgb(color.red, color.green, color.blue)
}

fn rgb_color(color: Color) -> TerminalRgb {
    let Color::Rgb(red, green, blue) = color else {
        unreachable!("built-in palette colors are RGB")
    };
    TerminalRgb::new(red, green, blue)
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

pub fn modal_surface_style() -> Style {
    let palette = active_palette();
    Style::default().fg(palette.modal_fg).bg(palette.modal_bg)
}

pub fn modal_border_style() -> Style {
    let palette = active_palette();
    Style::default()
        .fg(palette.modal_border)
        .bg(palette.modal_bg)
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

/// Adjust an RGB foreground just enough to meet the minimum text contrast on a
/// terminal background while preserving its hue as much as possible.
pub fn readable_text_color(color: Color, background: TerminalRgb) -> Color {
    let Color::Rgb(red, green, blue) = color else {
        return color;
    };
    let source = TerminalRgb::new(red, green, blue);
    if contrast_ratio(source, background) >= MINIMUM_TEXT_CONTRAST {
        return color;
    }

    let target = if background.is_light() { 0.0 } else { 255.0 };
    for step in 1..=100 {
        let amount = f64::from(step) / 100.0;
        let mix = |channel: u8| {
            (f64::from(channel) + (target - f64::from(channel)) * amount).round() as u8
        };
        let candidate = TerminalRgb::new(mix(red), mix(green), mix(blue));
        if contrast_ratio(candidate, background) >= MINIMUM_TEXT_CONTRAST {
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
    fn appearance_reference_backgrounds_are_stable_raster_canvases() {
        assert_eq!(
            ColorScheme::Dark.reference_background(),
            TerminalRgb::new(24, 24, 27)
        );
        assert_eq!(
            ColorScheme::Light.reference_background(),
            TerminalRgb::new(250, 250, 250)
        );
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
                    contrast_ratio(rgb_from_color(color), background) >= MINIMUM_TEXT_CONTRAST,
                    "{scheme:?} color {color:?} lacks text contrast"
                );
            }
            assert!(
                contrast_ratio(
                    rgb_from_color(palette.selection_fg),
                    rgb_from_color(palette.selection_bg)
                ) >= MINIMUM_TEXT_CONTRAST
            );
            assert!(
                contrast_ratio(
                    rgb_from_color(palette.code_fg),
                    rgb_from_color(palette.code_bg)
                ) >= MINIMUM_TEXT_CONTRAST
            );
            assert_eq!(
                rgb_from_color(palette.code_bg).is_light(),
                scheme == ColorScheme::Light,
                "{scheme:?} code surface uses the wrong appearance"
            );
            assert!(
                contrast_ratio(rgb_from_color(palette.modal_bg), background)
                    >= MINIMUM_SURFACE_CONTRAST,
                "{scheme:?} modal surface does not separate from its canvas"
            );
            assert!(
                contrast_ratio(
                    rgb_from_color(palette.modal_fg),
                    rgb_from_color(palette.modal_bg)
                ) >= MINIMUM_TEXT_CONTRAST
            );
            assert!(
                contrast_ratio(
                    rgb_from_color(palette.modal_border),
                    rgb_from_color(palette.modal_bg)
                ) >= MINIMUM_TEXT_CONTRAST
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
                assert!(contrast_ratio(rgb_from_color(color), background) >= MINIMUM_TEXT_CONTRAST);
            }
            assert!(
                contrast_ratio(
                    rgb_from_color(palette.selection_fg),
                    rgb_from_color(palette.selection_bg)
                ) >= MINIMUM_TEXT_CONTRAST
            );
            assert!(
                contrast_ratio(rgb_from_color(palette.modal_bg), background)
                    >= MINIMUM_SURFACE_CONTRAST
            );
            assert!(
                contrast_ratio(
                    rgb_from_color(palette.modal_fg),
                    rgb_from_color(palette.modal_bg)
                ) >= MINIMUM_TEXT_CONTRAST
            );
            assert!(
                contrast_ratio(
                    rgb_from_color(palette.modal_border),
                    rgb_from_color(palette.modal_bg)
                ) >= MINIMUM_TEXT_CONTRAST
            );
        }
    }

    #[test]
    fn modal_surface_contrast_survives_a_mismatched_scheme_hint() {
        for (scheme, background) in [
            (ColorScheme::Dark, TerminalRgb::new(250, 250, 250)),
            (ColorScheme::Light, TerminalRgb::new(5, 5, 5)),
        ] {
            let modal = modal_background(scheme, background);
            assert!(contrast_ratio(modal, background) >= MINIMUM_SURFACE_CONTRAST);
        }
    }

    #[test]
    fn arbitrary_rgb_text_is_corrected_to_minimum_contrast() {
        for background in [
            TerminalRgb::new(0, 0, 0),
            TerminalRgb::new(250, 250, 250),
            TerminalRgb::new(96, 96, 96),
        ] {
            let corrected = readable_text_color(rgb(147, 161, 161), background);
            assert!(contrast_ratio(rgb_from_color(corrected), background) >= MINIMUM_TEXT_CONTRAST);
        }
    }

    #[test]
    fn unknown_terminal_background_keeps_native_code_colors() {
        let palette = ThemePalette::for_unknown_background();
        assert_eq!(palette.code_fg, Color::Reset);
        assert_eq!(palette.code_bg, Color::Reset);
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
