//! Theme configuration, parsing, custom themes, and UI color tokens.

use std::{
    collections::{HashMap, HashSet},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    rc::Rc,
    sync::LazyLock,
};

use anyhow::{Result, anyhow};
use gpui_kit::App;
use gpui_kit::component::ColorTokens;
use gpui_kit::component::{Theme as KitTheme, ThemeConfig};
use serde::Deserialize;
use yomu::library::{DataDirs, ThemeModePreference};

pub const DEFAULT_THEME_NAME: &str = "Catppuccin Mocha";

#[derive(Clone, Copy)]
pub struct UiColors {
    pub background: gpui_kit::Hsla,
    pub surface: gpui_kit::Hsla,
    pub surface_hover: gpui_kit::Hsla,
    pub border: gpui_kit::Hsla,
    pub foreground: gpui_kit::Hsla,
    pub muted: gpui_kit::Hsla,
    pub accent: gpui_kit::Hsla,
    pub accent_foreground: gpui_kit::Hsla,
    pub danger: gpui_kit::Hsla,
    pub danger_hover: gpui_kit::Hsla,
    pub widget_opacity: f32,
}

impl UiColors {
    pub fn from_tokens(tokens: ColorTokens) -> Self {
        Self {
            background: tokens.background,
            surface: tokens.surface,
            surface_hover: tokens.secondary,
            border: tokens.border,
            foreground: tokens.foreground,
            muted: tokens.muted_foreground,
            accent: tokens.primary,
            accent_foreground: tokens.primary_foreground,
            danger: tokens.destructive,
            danger_hover: tokens.destructive,
            widget_opacity: 1.0,
        }
    }

    pub fn from_app(cx: &App) -> Self {
        cx.try_global::<KitTheme>()
            .map(|theme| {
                let mut colors = Self::from_tokens(theme.semantic_tokens().colors);
                colors.danger = theme.danger;
                colors.danger_hover = theme.danger_hover;
                colors
            })
            .unwrap_or_else(|| Self::from_tokens(ColorTokens::dark()))
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct CustomThemeFile {
    name: String,
    themes: Vec<CustomThemeDefinition>,
}

impl Default for CustomThemeFile {
    fn default() -> Self {
        Self {
            name: "yomu custom themes".to_string(),
            themes: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct CustomThemeDefinition {
    name: String,
    mode: String,
    colors: std::collections::BTreeMap<String, String>,
    font_size: Option<f32>,
    font_family: Option<String>,
    mono_font_size: Option<f32>,
    mono_font_family: Option<String>,
    radius: Option<usize>,
    radius_lg: Option<usize>,
    shadow: Option<bool>,
}

impl Default for CustomThemeDefinition {
    fn default() -> Self {
        Self {
            name: String::new(),
            mode: "dark".to_string(),
            colors: std::collections::BTreeMap::new(),
            font_size: None,
            font_family: None,
            mono_font_size: None,
            mono_font_family: None,
            radius: None,
            radius_lg: None,
            shadow: None,
        }
    }
}

pub const CUSTOM_THEME_COLOR_KEYS: &[&str] = &[
    "accent.background",
    "accent.foreground",
    "background",
    "border",
    "button.background",
    "button.foreground",
    "button.hover.background",
    "button.primary.background",
    "button.primary.foreground",
    "button.primary.hover.background",
    "danger.background",
    "danger.foreground",
    "foreground",
    "input.border",
    "link",
    "muted.background",
    "muted.foreground",
    "popover.background",
    "popover.foreground",
    "primary.background",
    "primary.foreground",
    "primary.hover.background",
    "ring",
    "scrollbar.background",
    "secondary.background",
    "secondary.foreground",
    "secondary.hover.background",
    "tab.active.background",
    "tab.active.foreground",
    "tab.background",
    "tab.foreground",
    "tab_bar.background",
    "tab_bar.segmented.background",
];

pub fn parse_custom_themes(
    contents: &str,
    built_in_names: &HashSet<String>,
) -> Result<HashMap<String, Rc<ThemeConfig>>> {
    let file: CustomThemeFile =
        toml::from_str(contents).map_err(|error| anyhow!("parsing themes.toml: {error}"))?;
    let mut names = HashSet::new();
    let mut themes = HashMap::new();

    for definition in file.themes {
        let name = definition.name.trim();
        if name.is_empty() {
            return Err(anyhow!("each custom theme needs a non-empty name"));
        }
        if built_in_names.contains(name) {
            return Err(anyhow!(
                "custom theme name “{name}” conflicts with a built-in theme"
            ));
        }
        if !names.insert(name.to_string()) {
            return Err(anyhow!("duplicate custom theme name “{name}”"));
        }
        if definition.mode != "dark" && definition.mode != "light" {
            return Err(anyhow!("theme “{name}” mode must be `dark` or `light`"));
        }
        if definition.colors.is_empty() {
            return Err(anyhow!("theme “{name}” needs at least one color override"));
        }
        for (key, value) in &definition.colors {
            if !CUSTOM_THEME_COLOR_KEYS.contains(&key.as_str()) {
                return Err(anyhow!("unsupported color token `{key}` in theme “{name}”"));
            }
            let digits = value.strip_prefix('#').unwrap_or("");
            if !matches!(digits.len(), 6 | 8)
                || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(anyhow!(
                    "color `{key}` in theme “{name}” must use #RRGGBB or #RRGGBBAA"
                ));
            }
        }

        let config_value = serde_json::json!({
            "name": name,
            "mode": definition.mode,
            "colors": definition.colors,
            "font.size": definition.font_size,
            "font.family": definition.font_family,
            "mono_font.size": definition.mono_font_size,
            "mono_font.family": definition.mono_font_family,
            "radius": definition.radius,
            "radius.lg": definition.radius_lg,
            "shadow": definition.shadow,
        });
        let config: ThemeConfig = serde_json::from_value(config_value)
            .map_err(|error| anyhow!("invalid theme “{name}”: {error}"))?;
        themes.insert(name.to_string(), Rc::new(config));
    }

    if themes.is_empty() {
        let set_name = file.name.trim();
        let set_name = if set_name.is_empty() {
            "custom theme set"
        } else {
            set_name
        };
        return Err(anyhow!(
            "{set_name} must define at least one [[themes]] entry"
        ));
    }
    Ok(themes)
}

pub fn custom_theme_path(_data_dirs: &DataDirs) -> Result<PathBuf> {
    #[cfg(test)]
    {
        Ok(_data_dirs
            .library_file()
            .parent()
            .unwrap_or(Path::new("."))
            .join("themes.toml"))
    }
    #[cfg(not(test))]
    {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow!("could not determine the user's configuration directory"))?;
        Ok(config_dir.join("yomu").join("themes.toml"))
    }
}

#[allow(dead_code)]
pub fn custom_theme_fingerprint(path: &Path) -> Option<u64> {
    let contents = fs::read(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    contents.hash(&mut hasher);
    Some(hasher.finish())
}

pub fn ensure_custom_theme_file(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("custom theme path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            use std::io::Write as _;
            file.write_all(include_bytes!("../../themes.example.toml"))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn parse_theme_file(contents: &str) -> Result<HashMap<String, std::sync::Arc<ThemeConfig>>> {
    let file: CustomThemeFile =
        toml::from_str(contents).map_err(|error| anyhow!("parsing themes: {error}"))?;
    let mut themes = HashMap::new();
    for definition in file.themes {
        let name = definition.name.trim();
        let config_value = serde_json::json!({
            "name": name,
            "mode": definition.mode,
            "colors": definition.colors,
            "font.size": definition.font_size,
            "font.family": definition.font_family,
            "mono_font.size": definition.mono_font_size,
            "mono_font.family": definition.mono_font_family,
            "radius": definition.radius,
            "radius.lg": definition.radius_lg,
            "shadow": definition.shadow,
        });
        let config: ThemeConfig = serde_json::from_value(config_value)
            .map_err(|error| anyhow!("invalid theme “{name}”: {error}"))?;
        themes.insert(name.to_string(), std::sync::Arc::new(config));
    }
    Ok(themes)
}

pub static BUILT_IN_THEMES: LazyLock<HashMap<String, std::sync::Arc<ThemeConfig>>> =
    LazyLock::new(|| {
        parse_theme_file(include_str!("builtin_themes.toml"))
            .expect("failed to parse built-in themes")
    });

pub fn built_in_theme(name: &str) -> Option<Rc<ThemeConfig>> {
    BUILT_IN_THEMES.get(name).map(|c| Rc::new((**c).clone()))
}

pub fn built_in_theme_names(_cx: &App) -> HashSet<String> {
    BUILT_IN_THEMES.keys().cloned().collect()
}

#[allow(dead_code)]
pub fn load_custom_themes(path: &Path, cx: &App) -> Result<HashMap<String, Rc<ThemeConfig>>> {
    let contents =
        fs::read_to_string(path).map_err(|error| anyhow!("reading {}: {error}", path.display()))?;
    let built_in_names = built_in_theme_names(cx);
    parse_custom_themes(&contents, &built_in_names)
}

pub fn resolve_theme_name(
    color_scheme: &str,
    mode: ThemeModePreference,
    system_dark: bool,
    custom_themes: &HashMap<String, Rc<ThemeConfig>>,
) -> String {
    let effective_dark = match mode {
        ThemeModePreference::System => system_dark,
        ThemeModePreference::Dark => true,
        ThemeModePreference::Light => false,
    };

    match color_scheme {
        "Catppuccin" => {
            if effective_dark {
                "Catppuccin Mocha".to_string()
            } else {
                "Catppuccin Latte".to_string()
            }
        }
        "Gruvbox" => {
            if effective_dark {
                "Gruvbox Dark".to_string()
            } else {
                "Gruvbox Light".to_string()
            }
        }
        other => {
            // Check if there is an exact custom theme match
            if custom_themes.contains_key(other) {
                return other.to_string();
            }
            // Check for explicit Dark/Light suffix in custom themes
            let variant = if effective_dark {
                format!("{other} Dark")
            } else {
                format!("{other} Light")
            };
            if custom_themes.contains_key(&variant) {
                return variant;
            }
            // Fallback to built-in themes if an exact name was supplied
            if BUILT_IN_THEMES.contains_key(other) {
                return other.to_string();
            }
            DEFAULT_THEME_NAME.to_string()
        }
    }
}

pub fn available_color_schemes(custom_themes: &HashMap<String, Rc<ThemeConfig>>) -> Vec<String> {
    let mut schemes = vec!["Catppuccin".to_string(), "Gruvbox".to_string()];
    for name in custom_themes.keys() {
        let base_name = name
            .strip_suffix(" Dark")
            .or_else(|| name.strip_suffix(" Light"))
            .unwrap_or(name.as_str());
        if !schemes.iter().any(|s| s == base_name) {
            schemes.push(base_name.to_string());
        }
    }
    schemes
}

pub fn is_standalone_scheme(color_scheme: &str) -> bool {
    color_scheme != "Catppuccin" && color_scheme != "Gruvbox"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_themes_contain_catppuccin_and_gruvbox() {
        let themes = &*BUILT_IN_THEMES;
        assert_eq!(DEFAULT_THEME_NAME, "Catppuccin Mocha");
        assert!(themes.contains_key("Catppuccin Mocha"));
        assert!(themes.contains_key("Catppuccin Macchiato"));
        assert!(themes.contains_key("Catppuccin Frappé"));
        assert!(themes.contains_key("Catppuccin Latte"));
        assert!(themes.contains_key("Gruvbox Dark"));
        assert!(themes.contains_key("Gruvbox Light"));

        // Ensure old default themes are completely gone
        assert!(!themes.contains_key("Default Dark"));
        assert!(!themes.contains_key("Default Light"));
        assert!(!themes.contains_key("Soft Plum"));
    }

    #[test]
    fn built_in_themes_have_valid_tokens() {
        for (name, config) in BUILT_IN_THEMES.iter() {
            assert!(
                config.mode == gpui_kit::component::ThemeMode::Dark
                    || config.mode == gpui_kit::component::ThemeMode::Light,
                "Theme {name} has unexpected mode {:?}",
                config.mode
            );
            assert!(
                config.colors.background.is_some(),
                "Theme {name} missing background color"
            );
            assert!(
                config.colors.foreground.is_some(),
                "Theme {name} missing foreground color"
            );
        }
    }

    #[test]
    fn test_resolve_theme_name() {
        let custom_themes = HashMap::new();
        // Catppuccin
        assert_eq!(
            resolve_theme_name(
                "Catppuccin",
                ThemeModePreference::Dark,
                false,
                &custom_themes
            ),
            "Catppuccin Mocha"
        );
        assert_eq!(
            resolve_theme_name(
                "Catppuccin",
                ThemeModePreference::Light,
                true,
                &custom_themes
            ),
            "Catppuccin Latte"
        );
        assert_eq!(
            resolve_theme_name(
                "Catppuccin",
                ThemeModePreference::System,
                true,
                &custom_themes
            ),
            "Catppuccin Mocha"
        );
        assert_eq!(
            resolve_theme_name(
                "Catppuccin",
                ThemeModePreference::System,
                false,
                &custom_themes
            ),
            "Catppuccin Latte"
        );

        // Gruvbox
        assert_eq!(
            resolve_theme_name("Gruvbox", ThemeModePreference::Dark, false, &custom_themes),
            "Gruvbox Dark"
        );
        assert_eq!(
            resolve_theme_name("Gruvbox", ThemeModePreference::Light, true, &custom_themes),
            "Gruvbox Light"
        );
        assert_eq!(
            resolve_theme_name("Gruvbox", ThemeModePreference::System, true, &custom_themes),
            "Gruvbox Dark"
        );
        assert_eq!(
            resolve_theme_name(
                "Gruvbox",
                ThemeModePreference::System,
                false,
                &custom_themes
            ),
            "Gruvbox Light"
        );

        // Standalone Custom Theme like Noctalia Dynamic
        let mut custom = HashMap::new();
        let dummy_cfg: Rc<ThemeConfig> = Rc::new(
            serde_json::from_value(serde_json::json!({
                "name": "Noctalia Dynamic",
                "mode": "dark"
            }))
            .unwrap(),
        );
        custom.insert("Noctalia Dynamic".to_string(), dummy_cfg);

        assert_eq!(
            resolve_theme_name(
                "Noctalia Dynamic",
                ThemeModePreference::System,
                false,
                &custom
            ),
            "Noctalia Dynamic"
        );
        assert_eq!(
            resolve_theme_name(
                "Noctalia Dynamic",
                ThemeModePreference::Light,
                false,
                &custom
            ),
            "Noctalia Dynamic"
        );
        assert_eq!(
            resolve_theme_name(
                "Noctalia Dynamic",
                ThemeModePreference::Dark,
                false,
                &custom
            ),
            "Noctalia Dynamic"
        );

        assert!(is_standalone_scheme("Noctalia Dynamic"));
        assert!(!is_standalone_scheme("Catppuccin"));
        assert!(!is_standalone_scheme("Gruvbox"));
    }
}
