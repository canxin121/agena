use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use agena_plugin_sdk::{
    PluginTerminalColor, PluginTerminalThemeColors, PluginTerminalThemePalette,
};
use serde_json::Value;

use super::{MAX_COMPONENT_ENTRIES, Manifest, manifest_path, read_package_object, text_field};

pub(super) fn discover(
    root: &Path,
    manifest: &Manifest,
    watch: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<String>,
) -> Vec<PluginTerminalThemePalette> {
    let declaration = manifest
        .document
        .get("experimental")
        .and_then(|experimental| experimental.get("themes"))
        .or_else(|| manifest.document.get("themes"));
    let default = Value::String("./themes".into());
    let source = declaration.unwrap_or(&default);
    let sources = source
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(source));
    let mut files = BTreeSet::new();
    for value in sources {
        let path = match value
            .as_str()
            .ok_or_else(|| "Claude themes require local paths".to_owned())
            .and_then(|source| manifest_path(root, source))
        {
            Ok(path) => path,
            Err(error) => {
                diagnostics.push(error);
                continue;
            }
        };
        watch.insert(path.clone());
        if path.is_dir() {
            match std::fs::read_dir(&path) {
                Ok(entries) => {
                    files.extend(entries.take(MAX_COMPONENT_ENTRIES).filter_map(|entry| {
                        let entry = entry.ok()?;
                        let path = entry.path();
                        (path.extension().is_some_and(|ext| ext == "json")
                            && entry.file_type().is_ok_and(|kind| kind.is_file()))
                        .then_some(path)
                    }))
                }
                Err(error) => diagnostics.push(format!("reading Claude themes: {error}")),
            }
        } else if path.extension().is_some_and(|ext| ext == "json") {
            files.insert(path);
        } else if declaration.is_some() {
            diagnostics.push(format!("invalid Claude theme path {path:?}"));
        }
        if files.len() >= MAX_COMPONENT_ENTRIES {
            diagnostics.push("too many Claude theme files".into());
            break;
        }
    }
    let mut themes = BTreeMap::new();
    for path in files.into_iter().take(MAX_COMPONENT_ENTRIES) {
        watch.insert(path.clone());
        let relative = path
            .strip_prefix(root)
            .expect("theme path under plugin root");
        match load(root, relative) {
            Ok(theme) => match themes.entry(theme.id.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(theme);
                }
                std::collections::btree_map::Entry::Occupied(_) => {
                    diagnostics.push(format!("duplicate Claude theme slug {:?}", theme.id));
                }
            },
            Err(error) => diagnostics.push(format!("Claude theme {relative:?}: {error}")),
        }
    }
    themes.into_values().collect()
}

fn load(root: &Path, relative: &Path) -> Result<PluginTerminalThemePalette, String> {
    let document =
        read_package_object(root, relative, 64 * 1024)?.ok_or("theme file does not exist")?;
    let id = relative
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|slug| {
            !slug.is_empty()
                && slug.len() <= 128
                && slug
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        })
        .ok_or("invalid theme filename")?;
    let display_name = text_field(document.get("name"))
        .filter(|name| name.len() <= 256 && !name.chars().any(char::is_control))
        .unwrap_or_else(|| id.to_owned());
    let overrides = document.get("overrides");
    let color = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| claude_color(overrides?.get(*key)?.as_str()?))
    };
    Ok(PluginTerminalThemePalette {
        id: id.to_owned(),
        display_name,
        colors: PluginTerminalThemeColors {
            muted: color(&["inactive", "subtle"]),
            accent: color(&["claude", "promptBorder"]),
            info: color(&["permission", "ide"]),
            success: color(&["success"]),
            warning: color(&["warning"]),
            danger: color(&["error"]),
            special: color(&["planMode"]),
            selection_fg: color(&["inverseText"]),
            selection_bg: color(&["suggestion"]),
        },
    })
}

fn claude_color(value: &str) -> Option<PluginTerminalColor> {
    let rgb = |red, green, blue| {
        PluginTerminalColor::new(format!("#{red:02x}{green:02x}{blue:02x}")).ok()
    };
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 3 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            let mut expanded = String::from("#");
            for digit in hex.chars() {
                expanded.push(digit);
                expanded.push(digit);
            }
            return PluginTerminalColor::new(expanded).ok();
        }
        return PluginTerminalColor::new(value).ok();
    }
    if let Some(values) = value
        .strip_prefix("rgb(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let mut parts = values.split(',');
        let red = parts.next()?.trim().parse::<u8>().ok()?;
        let green = parts.next()?.trim().parse::<u8>().ok()?;
        let blue = parts.next()?.trim().parse::<u8>().ok()?;
        return (parts.next().is_none())
            .then(|| rgb(red, green, blue))
            .flatten();
    }

    const CLAUDE_ANSI: [&str; 16] = [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "blackBright",
        "redBright",
        "greenBright",
        "yellowBright",
        "blueBright",
        "magentaBright",
        "cyanBright",
        "whiteBright",
    ];
    const AGENA_ANSI: [&str; 16] = [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "gray",
        "dark_gray",
        "light_red",
        "light_green",
        "light_yellow",
        "light_blue",
        "light_magenta",
        "light_cyan",
        "white",
    ];
    if let Some(name) = value.strip_prefix("ansi:") {
        let index = CLAUDE_ANSI.iter().position(|color| *color == name)?;
        return PluginTerminalColor::new(AGENA_ANSI[index]).ok();
    }
    let index = value
        .strip_prefix("ansi256(")?
        .strip_suffix(')')?
        .trim()
        .parse::<u8>()
        .ok()?;
    match index {
        0..=15 => PluginTerminalColor::new(AGENA_ANSI[index as usize]).ok(),
        16..=231 => {
            let coordinate = |value: u8| if value == 0 { 0 } else { 55 + 40 * value };
            let index = index - 16;
            rgb(
                coordinate(index / 36),
                coordinate(index / 6 % 6),
                coordinate(index % 6),
            )
        }
        _ => {
            let level = 8 + 10 * (index - 232);
            rgb(level, level, level)
        }
    }
}
