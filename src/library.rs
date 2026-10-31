//! The library: which books exist, where reading progress is, and how new ones get imported.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    archive::{self, SourceKind},
    imaging,
    layout::{self, FitMode},
    util::{fnv1a, natural_cmp, now_secs},
};

const MAX_SCAN_DEPTH: usize = 6;

/// Where yomu keeps its data (`$XDG_DATA_HOME/yomu`, or `$YOMU_DATA_DIR` if set).
#[derive(Debug, Clone)]
pub struct DataDirs {
    root: PathBuf,
}

impl DataDirs {
    pub fn new() -> Result<Self> {
        let root = match std::env::var_os("YOMU_DATA_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => dirs::data_dir()
                .context("could not determine the user data directory")?
                .join("yomu"),
        };
        Self::at(root)
    }

    pub fn at(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join("covers"))
            .with_context(|| format!("creating {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn library_file(&self) -> PathBuf {
        self.root.join("library.json")
    }

    pub fn cover(&self, id: &str) -> PathBuf {
        self.cover_for_size(id, ThumbnailSize::Medium)
    }

    pub fn cover_for_size(&self, id: &str, size: ThumbnailSize) -> PathBuf {
        let size_path = self
            .root
            .join("covers")
            .join(size.dir_name())
            .join(format!("{id}.jpg"));
        if size_path.exists() {
            return size_path;
        }
        let legacy_path = self.root.join("covers").join(format!("{id}.jpg"));
        if legacy_path.exists() {
            return legacy_path;
        }
        size_path
    }

    pub fn cover_dir_for_size(&self, size: ThumbnailSize) -> PathBuf {
        self.root.join("covers").join(size.dir_name())
    }

    pub fn clear_covers(&self) -> Result<()> {
        let covers_dir = self.root.join("covers");
        if covers_dir.exists() {
            fs::remove_dir_all(&covers_dir).context("removing covers directory")?;
        }
        fs::create_dir_all(&covers_dir).context("recreating covers directory")?;
        Ok(())
    }

    pub fn cache_size_bytes(&self) -> u64 {
        let covers_dir = self.root.join("covers");
        dir_size_recursive(&covers_dir)
    }
}

fn dir_size_recursive(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    total += dir_size_recursive(&entry.path());
                } else {
                    total += meta.len();
                }
            }
        }
    }
    total
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    pub id: String,
    pub title: String,
    /// Optional user-assigned series. Missing values in existing libraries remain ungrouped.
    #[serde(default)]
    pub series: Option<String>,
    pub path: PathBuf,
    pub kind: SourceKind,
    pub page_count: usize,
    /// Zero-based index of the page the reader was last on.
    #[serde(default)]
    pub current_page: usize,
    /// Whether the book has ever been opened. Unopened books show 0% instead of 1 page.
    #[serde(default)]
    pub opened: bool,
    pub added_at: u64,
    #[serde(default)]
    pub last_read: Option<u64>,
    /// Vim-style marks: letter to zero-based page index.
    #[serde(default)]
    pub marks: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStatus {
    Unread,
    InProgress,
    Finished,
}

impl Book {
    pub fn status(&self) -> ReadStatus {
        if self.is_finished() {
            ReadStatus::Finished
        } else if self.opened && self.current_page > 0 {
            ReadStatus::InProgress
        } else {
            ReadStatus::Unread
        }
    }

    /// True if there is reading progress that resetting or removing would throw away.
    pub fn has_progress(&self) -> bool {
        self.status() != ReadStatus::Unread
    }

    pub fn reset_progress(&mut self) {
        self.current_page = 0;
        self.opened = false;
        self.last_read = None;
    }

    pub fn mark_read(&mut self) {
        self.opened = true;
        self.current_page = self.page_count.saturating_sub(1);
        self.last_read = Some(now_secs());
    }

    /// Put the reading position back on the first page, ready to be opened.
    pub fn start_over(&mut self) {
        self.opened = true;
        self.current_page = 0;
        self.last_read = Some(now_secs());
    }

    /// Reading progress in `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        if !self.opened || self.page_count == 0 {
            return 0.0;
        }
        ((self.current_page + 1) as f32 / self.page_count as f32).clamp(0.0, 1.0)
    }

    pub fn is_finished(&self) -> bool {
        self.opened && self.page_count > 0 && self.current_page + 1 >= self.page_count
    }
}

/// Lowest background opacity the settings allow. Below this the window becomes hard to see.
pub const OPACITY_MIN: f32 = 0.3;
pub const OPACITY_MAX: f32 = 1.0;

/// Clamp to the allowed range and round to a whole percent. Non-finite values fall back to opaque.
pub fn clamp_opacity(value: f32) -> f32 {
    if !value.is_finite() {
        return OPACITY_MAX;
    }
    (value.clamp(OPACITY_MIN, OPACITY_MAX) * 100.0).round() / 100.0
}

/// Version of the library file format. Bumped when a stored default changes meaning.
///
/// * 1 (and files without a version): reading direction defaulted to right-to-left.
/// * 2: reading direction defaults to left-to-right (Western comics); right-to-left is a preference.
/// * 3: library grouping defaults to on, with directory-based folder groups.
pub const LIBRARY_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThumbnailSize {
    #[serde(alias = "extra-small", alias = "xsmall", alias = "extrasmall")]
    Small,
    #[default]
    Medium,
    #[serde(
        alias = "extra-large",
        alias = "xlarge",
        alias = "extralarge",
        alias = "huge",
        alias = "extra-huge",
        alias = "xhuge"
    )]
    Large,
}

impl ThumbnailSize {
    pub const ALL: [ThumbnailSize; 3] = [
        ThumbnailSize::Small,
        ThumbnailSize::Medium,
        ThumbnailSize::Large,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ThumbnailSize::Small => "Small",
            ThumbnailSize::Medium => "Medium",
            ThumbnailSize::Large => "Large",
        }
    }

    pub fn card_width(&self) -> f32 {
        match self {
            ThumbnailSize::Small => 180.0,
            ThumbnailSize::Medium => 220.0,
            ThumbnailSize::Large => 270.0,
        }
    }

    pub fn card_height(&self) -> f32 {
        match self {
            ThumbnailSize::Small => 270.0,
            ThumbnailSize::Medium => 330.0,
            ThumbnailSize::Large => 405.0,
        }
    }

    pub fn pixel_dimensions(&self) -> (u32, u32) {
        match self {
            ThumbnailSize::Small => (360, 540),
            ThumbnailSize::Medium => (440, 660),
            ThumbnailSize::Large => (540, 810),
        }
    }

    pub fn dir_name(&self) -> &'static str {
        match self {
            ThumbnailSize::Small => "small",
            ThumbnailSize::Medium => "medium",
            ThumbnailSize::Large => "large",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            ThumbnailSize::Small => ThumbnailSize::Medium,
            ThumbnailSize::Medium => ThumbnailSize::Large,
            ThumbnailSize::Large => ThumbnailSize::Small,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum BorderSize {
    None,
    #[default]
    Thin,
    #[serde(alias = "medium", alias = "extra-thick", alias = "heavy")]
    Thick,
}

impl BorderSize {
    pub const ALL: [BorderSize; 3] = [BorderSize::None, BorderSize::Thin, BorderSize::Thick];

    pub fn label(&self) -> &'static str {
        match self {
            BorderSize::None => "None",
            BorderSize::Thin => "Thin",
            BorderSize::Thick => "Thick",
        }
    }

    pub fn pixels(&self) -> f32 {
        match self {
            BorderSize::None => 0.0,
            BorderSize::Thin => 1.0,
            BorderSize::Thick => 2.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CornerRounding {
    #[serde(alias = "sharp")]
    None,
    #[default]
    #[serde(alias = "medium", alias = "subtle")]
    Standard,
    #[serde(alias = "round", alias = "soft", alias = "extra-soft")]
    Rounded,
}

impl CornerRounding {
    pub const ALL: [CornerRounding; 3] = [
        CornerRounding::None,
        CornerRounding::Standard,
        CornerRounding::Rounded,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            CornerRounding::None => "None",
            CornerRounding::Standard => "Standard",
            CornerRounding::Rounded => "Rounded",
        }
    }

    /// Base radius for general elements (GPUI Kit `theme.radius`).
    pub fn radius_pixels(&self) -> f32 {
        match self {
            CornerRounding::None => 0.0,
            CornerRounding::Standard => 6.0,
            CornerRounding::Rounded => 12.0,
        }
    }

    /// Radius for cards and dialogs (GPUI Kit `theme.radius_lg`).
    pub fn radius_lg_pixels(&self) -> f32 {
        match self {
            CornerRounding::None => 0.0,
            CornerRounding::Standard => 10.0,
            CornerRounding::Rounded => 16.0,
        }
    }

    /// Radius for large card containers and modal dialogs.
    pub fn radius_xl_pixels(&self) -> f32 {
        match self {
            CornerRounding::None => 0.0,
            CornerRounding::Standard => 14.0,
            CornerRounding::Rounded => 24.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeModePreference {
    System,
    Light,
    #[default]
    Dark,
}

impl ThemeModePreference {
    pub const ALL: [ThemeModePreference; 3] = [
        ThemeModePreference::System,
        ThemeModePreference::Light,
        ThemeModePreference::Dark,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::System => "Auto",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

#[allow(dead_code)]
fn default_color_scheme() -> String {
    "Catppuccin".to_string()
}

/// Preferences that carry over from book to book and between launches.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settings {
    /// Name of the GPUI Kit theme selected in Appearance settings.
    pub theme_name: String,
    /// Theme appearance preference: System (Auto), Light, or Dark.
    #[serde(default)]
    pub theme_mode: ThemeModePreference,
    /// Color scheme family: Catppuccin, Gruvbox, Tokyo Night, Noctalia Dynamic, etc.
    #[serde(default = "default_color_scheme")]
    pub color_scheme: String,
    pub fit: FitMode,
    /// Retain custom zoom and pinch magnification across all books and sessions.
    pub remember_zoom: bool,
    /// Stored zoom magnification multiplier on top of the fit mode.
    pub zoom: f32,
    /// Slide between pages and glide when scrolling. Reduced-motion preferences of the desktop
    /// switch this off regardless of the stored value.
    pub animate: bool,
    /// Disable nonessential animations across the application.
    #[serde(default)]
    pub disable_animations: bool,
    /// Stack pages vertically and scroll through the whole book instead of turning pages.
    pub continuous: bool,
    /// Force a connected, gapless vertical strip of pages, regardless of the normal reading mode.
    pub webcomics: bool,
    /// Group books into directory folders, with manual series assignments taking precedence.
    pub group_series: bool,
    /// Thumbnail card size in library grid view.
    #[serde(default)]
    pub thumbnail_size: ThumbnailSize,
    /// Border thickness across cards, dialogs, and controls.
    #[serde(default)]
    pub border_size: BorderSize,
    /// Corner rounding across cards, dialogs, and controls.
    #[serde(default)]
    pub corner_rounding: CornerRounding,
    /// View mode for library cards: true for grid view (default), false for list view.
    pub view_mode_grid: bool,
    /// Whether Vim keyboard navigation is enabled. Disabled by default.
    #[serde(default)]
    pub vim_mode: bool,
    /// Opacity of the window background, `OPACITY_MIN..=OPACITY_MAX`. Raised widget surfaces use
    /// a small opacity lift to stay distinct; page images, covers, and text remain opaque.
    pub opacity: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_name: "Catppuccin Mocha".to_string(),
            theme_mode: ThemeModePreference::Dark,
            color_scheme: "Catppuccin".to_string(),
            fit: FitMode::Page,
            remember_zoom: true,
            zoom: 1.0,
            animate: true,
            disable_animations: false,
            continuous: false,
            webcomics: false,
            group_series: false,
            thumbnail_size: ThumbnailSize::Medium,
            border_size: BorderSize::Thin,
            corner_rounding: CornerRounding::Standard,
            view_mode_grid: true,
            vim_mode: false,
            opacity: OPACITY_MAX,
        }
    }
}

impl Settings {
    pub fn set_opacity(&mut self, value: f32) {
        self.opacity = clamp_opacity(value);
    }

    /// Read settings leniently: any missing, mistyped or out-of-range field falls back to its
    /// default (or is clamped) instead of making the whole library file unreadable.
    pub fn from_value(value: &serde_json::Value) -> Self {
        let mut settings = Self::default();
        let Some(object) = value.as_object() else {
            return settings;
        };
        if let Some(mode) = object
            .get("theme_mode")
            .and_then(|v| serde_json::from_value::<ThemeModePreference>(v.clone()).ok())
        {
            settings.theme_mode = mode;
        }
        if let Some(scheme) = object.get("color_scheme").and_then(|v| v.as_str()) {
            settings.color_scheme = scheme.to_string();
        }
        if let Some(theme_name) = object.get("theme_name").and_then(|v| v.as_str())
            && !theme_name.trim().is_empty()
        {
            let name = match theme_name.trim() {
                "Default Dark" | "Soft Plum" => "Catppuccin Mocha",
                "Default Light" => "Catppuccin Latte",
                other => other,
            };
            settings.theme_name = name.to_string();
            if object.get("color_scheme").is_none() {
                if name.starts_with("Catppuccin") {
                    settings.color_scheme = "Catppuccin".to_string();
                } else if name.starts_with("Gruvbox") {
                    settings.color_scheme = "Gruvbox".to_string();
                } else {
                    settings.color_scheme = name.to_string();
                }
            }
        }
        let disabled = match (
            object.get("disable_animations").and_then(|v| v.as_bool()),
            object.get("animate").and_then(|v| v.as_bool()),
        ) {
            (Some(true), _) | (_, Some(false)) => true,
            (Some(false), _) => false,
            (None, Some(true)) => false,
            (None, None) => false,
        };
        settings.disable_animations = disabled;
        settings.animate = !disabled;
        if let Some(continuous) = object.get("continuous").and_then(|v| v.as_bool()) {
            settings.continuous = continuous;
        }
        if let Some(webcomics) = object.get("webcomics").and_then(|v| v.as_bool()) {
            settings.webcomics = webcomics;
        }
        if let Some(group_series) = object.get("group_series").and_then(|v| v.as_bool()) {
            settings.group_series = group_series;
        }
        if let Some(size) = object
            .get("thumbnail_size")
            .and_then(|v| serde_json::from_value::<ThumbnailSize>(v.clone()).ok())
        {
            settings.thumbnail_size = size;
        }
        if let Some(border_size) = object
            .get("border_size")
            .and_then(|v| serde_json::from_value::<BorderSize>(v.clone()).ok())
        {
            settings.border_size = border_size;
        }
        if let Some(corner_rounding) = object
            .get("corner_rounding")
            .and_then(|v| serde_json::from_value::<CornerRounding>(v.clone()).ok())
        {
            settings.corner_rounding = corner_rounding;
        }
        if let Some(view_mode_grid) = object.get("view_mode_grid").and_then(|v| v.as_bool()) {
            settings.view_mode_grid = view_mode_grid;
        }
        if let Some(vim_mode) = object.get("vim_mode").and_then(|v| v.as_bool()) {
            settings.vim_mode = vim_mode;
        }
        if let Some(fit) = object
            .get("fit")
            .and_then(|v| serde_json::from_value::<FitMode>(v.clone()).ok())
        {
            settings.fit = fit;
        }
        if let Some(remember_zoom) = object.get("remember_zoom").and_then(|v| v.as_bool()) {
            settings.remember_zoom = remember_zoom;
        }
        if let Some(zoom) = object.get("zoom").and_then(|v| v.as_f64()) {
            settings.zoom = (zoom as f32).clamp(layout::ZOOM_MIN, layout::ZOOM_MAX);
        }
        if let Some(opacity) = object.get("opacity").and_then(|v| v.as_f64()) {
            settings.set_opacity(opacity as f32);
        }
        settings
    }
}

fn lenient_settings<'de, D>(deserializer: D) -> Result<Settings, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(Settings::from_value(&value))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    /// File format version; files from before versioning read as 0.
    #[serde(default)]
    version: u32,
    #[serde(default)]
    books: Vec<Book>,
    #[serde(default, deserialize_with = "lenient_settings")]
    settings: Settings,
    #[serde(default)]
    pub collections: Vec<crate::collections::Collection>,
    #[serde(default)]
    pub favorites: HashSet<String>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            version: LIBRARY_VERSION,
            books: Vec::new(),
            settings: Settings::default(),
            collections: Vec::new(),
            favorites: HashSet::new(),
        }
    }
}

impl Library {
    /// Bring a library written by an older version up to date.
    fn migrate(&mut self) {
        // Directory folders are the default library presentation from version 3 onward. Keep
        // an explicit value from an existing library so users who disabled grouping retain that
        // choice; missing values inherit the new default through `Settings::default`.
        self.version = LIBRARY_VERSION;
    }

    pub fn load(dirs: &DataDirs) -> Result<Self> {
        let file = dirs.library_file();
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err).with_context(|| format!("reading {}", file.display())),
        };
        match serde_json::from_str::<Library>(&text) {
            Ok(mut lib) => {
                lib.migrate();
                Ok(lib)
            }
            Err(err) => {
                // Keep the unreadable file around instead of silently overwriting it.
                let backup = file.with_extension("json.corrupt");
                let _ = fs::rename(&file, &backup);
                bail!(
                    "library.json was unreadable ({err}); moved it to {} and started fresh",
                    backup.display()
                )
            }
        }
    }

    /// Write atomically (temp file + rename) so a crash cannot leave a half-written library.
    pub fn save(&self, dirs: &DataDirs) -> Result<()> {
        let file = dirs.library_file();
        let tmp = file.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(self)?;
        fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, &file).with_context(|| format!("replacing {}", file.display()))?;
        Ok(())
    }

    pub fn books(&self) -> &[Book] {
        &self.books
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut Settings {
        &mut self.settings
    }

    pub fn ids(&self) -> HashSet<String> {
        self.books.iter().map(|b| b.id.clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<&Book> {
        self.books.iter().find(|b| b.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Book> {
        self.books.iter_mut().find(|b| b.id == id)
    }

    /// Returns `false` (and changes nothing) if a book with the same id already exists.
    pub fn add(&mut self, book: Book) -> bool {
        if self.get(&book.id).is_some() {
            return false;
        }
        self.books.push(book);
        true
    }

    /// Remove from the library and delete the cached cover across all size tiers. The comic file itself is untouched.
    pub fn remove(&mut self, id: &str, dirs: &DataDirs) {
        self.books.retain(|b| b.id != id);
        for size in ThumbnailSize::ALL {
            let _ = fs::remove_file(
                dirs.root
                    .join("covers")
                    .join(size.dir_name())
                    .join(format!("{id}.jpg")),
            );
        }
        let _ = fs::remove_file(dirs.root.join("covers").join(format!("{id}.jpg")));
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorites.contains(id)
    }

    pub fn toggle_favorite(&mut self, id: &str) -> bool {
        if self.favorites.contains(id) {
            self.favorites.remove(id);
            false
        } else {
            self.favorites.insert(id.to_string());
            true
        }
    }

    pub fn collections(&self) -> &[crate::collections::Collection] {
        &self.collections
    }

    pub fn collections_mut(&mut self) -> &mut Vec<crate::collections::Collection> {
        &mut self.collections
    }

    pub fn add_collection(&mut self, name: &str) -> String {
        let id = format!("col_{}", fnv1a(name.as_bytes()));
        if !self.collections.iter().any(|c| c.id == id) {
            self.collections
                .push(crate::collections::Collection::new(id.clone(), name));
        }
        id
    }

    pub fn delete_collection(&mut self, id: &str) {
        self.collections.retain(|c| c.id != id);
    }

    pub fn rename_collection(&mut self, id: &str, new_name: &str) {
        if let Some(col) = self.collections.iter_mut().find(|c| c.id == id) {
            col.name = new_name.to_string();
        }
    }

    pub fn add_book_to_collection(&mut self, collection_id: &str, book_id: &str) {
        if let Some(col) = self.collections.iter_mut().find(|c| c.id == collection_id) {
            col.add(book_id);
        }
    }

    pub fn remove_book_from_collection(&mut self, collection_id: &str, book_id: &str) {
        if let Some(col) = self.collections.iter_mut().find(|c| c.id == collection_id) {
            col.remove(book_id);
        }
    }

    /// Clear all cached covers for all sizes.
    pub fn clear_cover_cache(&self, dirs: &DataDirs) -> Result<()> {
        dirs.clear_covers()
    }

    /// Clear all in-memory library data: books, reading history, marks, collections, favorites,
    /// and reset settings to default.
    pub fn clear_all_data(&mut self) {
        self.books.clear();
        self.collections.clear();
        self.favorites.clear();
        self.settings = Settings::default();
    }

    /// Rebuild/ensure cover thumbnails exist for the requested size. Returns count of newly generated covers.
    pub fn ensure_covers_for_size(&self, dirs: &DataDirs, size: ThumbnailSize) -> usize {
        let target_dir = dirs.cover_dir_for_size(size);
        let _ = fs::create_dir_all(&target_dir);
        let (max_w, max_h) = size.pixel_dimensions();
        let mut built = 0;

        for book in &self.books {
            let cover_path = target_dir.join(format!("{}.jpg", book.id));
            if cover_path.exists() {
                continue;
            }
            if let Ok(pages) = archive::list_pages(&book.path, book.kind) {
                for name in pages.iter().take(3) {
                    if let Ok(bytes) = archive::read_page(&book.path, book.kind, name)
                        && let Ok(jpeg) = imaging::make_cover_jpeg(&bytes, max_w, max_h)
                        && fs::write(&cover_path, jpeg).is_ok()
                    {
                        built += 1;
                        break;
                    }
                }
            }
        }
        built
    }

    /// Assign or clear a series for several books. Names are trimmed and matching existing
    /// names are reused case-insensitively so groups do not split over capitalization.
    pub fn set_series(&mut self, ids: &[String], series: Option<String>) {
        let series = series
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|name| !name.is_empty())
            .map(|name| {
                self.books
                    .iter()
                    .filter_map(|book| book.series.as_ref())
                    .find(|existing| existing.eq_ignore_ascii_case(&name))
                    .cloned()
                    .unwrap_or(name)
            });
        for id in ids {
            if let Some(book) = self.get_mut(id) {
                book.series = series.clone();
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ImportReport {
    pub added: Vec<Book>,
    /// Already in the library.
    pub skipped: usize,
    pub errors: Vec<String>,
}

impl ImportReport {
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.added.is_empty() {
            parts.push(format!("Imported {}", self.added.len()));
        }
        if self.skipped > 0 {
            parts.push(format!("{} already in library", self.skipped));
        }
        if let Some(first) = self.errors.first() {
            let more = self.errors.len() - 1;
            if more > 0 {
                parts.push(format!("{first} (+{more} more errors)"));
            } else {
                parts.push(first.clone());
            }
        }
        if parts.is_empty() {
            "Nothing to import".to_string()
        } else {
            parts.join(" · ")
        }
    }
}

enum Candidate {
    Archive(PathBuf),
    Folder(PathBuf),
}

#[inline]
fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}

/// Import files and folders. Folders are scanned recursively: every `.cbz`/`.zip` becomes a book,
/// and every folder that directly contains images becomes a book too.
///
/// Slow (decodes a cover per book), so call it from a background thread.
pub fn import_paths(
    paths: &[PathBuf],
    dirs: &DataDirs,
    existing: &HashSet<String>,
) -> ImportReport {
    import_paths_with_progress_cancellable(paths, dirs, existing, None, |_| {})
}

/// Import volumes in scan order and report each completed book immediately. The callback runs
/// on the caller's thread, so UI callers should send the book to their UI task rather than
/// touching UI state directly.
pub fn import_paths_with_progress<F>(
    paths: &[PathBuf],
    dirs: &DataDirs,
    existing: &HashSet<String>,
    on_book: F,
) -> ImportReport
where
    F: FnMut(&Book),
{
    import_paths_with_progress_cancellable(paths, dirs, existing, None, on_book)
}

/// Import volumes in scan order with cooperative cancellation support. Reports each completed
/// book immediately. If `cancel` is set during scanning, archive extraction, or thumbnail
/// generation, the worker stops early without starting expensive new work and returns a partial
/// report of books completed prior to cancellation.
pub fn import_paths_with_progress_cancellable<F>(
    paths: &[PathBuf],
    dirs: &DataDirs,
    existing: &HashSet<String>,
    cancel: Option<&Arc<AtomicBool>>,
    mut on_book: F,
) -> ImportReport
where
    F: FnMut(&Book),
{
    let cancel = cancel.map(Arc::as_ref);
    let mut report = ImportReport::default();
    let mut candidates = Vec::new();

    for path in paths {
        if is_cancelled(cancel) {
            return report;
        }
        collect(path, 0, true, &mut candidates, &mut report.errors, cancel);
    }

    if is_cancelled(cancel) {
        return report;
    }

    let mut seen = existing.clone();
    for candidate in candidates {
        if is_cancelled(cancel) {
            return report;
        }
        let (path, kind) = match &candidate {
            Candidate::Archive(p) => (p.as_path(), SourceKind::Archive),
            Candidate::Folder(p) => (p.as_path(), SourceKind::Folder),
        };
        let id = book_id(path);
        if !seen.insert(id) {
            report.skipped += 1;
            continue;
        }
        match build_book(path, kind, dirs, cancel) {
            Ok(book) => {
                if is_cancelled(cancel) {
                    return report;
                }
                on_book(&book);
                report.added.push(book);
            }
            Err(err) => {
                if is_cancelled(cancel) {
                    return report;
                }
                report
                    .errors
                    .push(format!("{}: {err:#}", display_name(path)));
            }
        }
    }
    report
}

fn collect(
    path: &Path,
    depth: usize,
    explicit: bool,
    out: &mut Vec<Candidate>,
    errors: &mut Vec<String>,
    cancel: Option<&AtomicBool>,
) {
    if is_cancelled(cancel) {
        return;
    }
    let path = match path.canonicalize() {
        Ok(p) => p,
        Err(err) => {
            if is_cancelled(cancel) {
                return;
            }
            errors.push(format!("{}: {err}", display_name(path)));
            return;
        }
    };

    if is_cancelled(cancel) {
        return;
    }

    if path.is_file() {
        let name = display_name(&path);
        if archive::is_archive_name(&name) {
            out.push(Candidate::Archive(path));
        } else if explicit {
            errors.push(format!(
                "{name}: unsupported file type (CBZ/CBR/ZIP/RAR archives and image folders only)"
            ));
        }
    } else if path.is_dir() {
        if is_cancelled(cancel) {
            return;
        }
        if archive::folder_has_images(&path) {
            out.push(Candidate::Folder(path));
        } else if depth < MAX_SCAN_DEPTH {
            let mut children: Vec<PathBuf> = match fs::read_dir(&path) {
                Ok(entries) => entries.flatten().map(|e| e.path()).collect(),
                Err(err) => {
                    if is_cancelled(cancel) {
                        return;
                    }
                    errors.push(format!("{}: {err}", display_name(&path)));
                    return;
                }
            };
            if is_cancelled(cancel) {
                return;
            }
            children.sort_by(|a, b| natural_cmp(&display_name(a), &display_name(b)));
            for child in children {
                if is_cancelled(cancel) {
                    return;
                }
                if display_name(&child).starts_with('.') {
                    continue;
                }
                collect(&child, depth + 1, false, out, errors, cancel);
            }
        }
    }
}

fn build_book(
    path: &Path,
    kind: SourceKind,
    dirs: &DataDirs,
    cancel: Option<&AtomicBool>,
) -> Result<Book> {
    if is_cancelled(cancel) {
        bail!("import cancelled");
    }
    let id = book_id(path);
    let pages = archive::list_pages(path, kind)?;
    if is_cancelled(cancel) {
        bail!("import cancelled");
    }
    if pages.is_empty() {
        bail!("no images found");
    }

    // The first page is usually the cover, but fall back to the next few if it will not decode.
    let mut cover_written = false;
    for name in pages.iter().take(3) {
        if is_cancelled(cancel) {
            bail!("import cancelled");
        }
        let Ok(bytes) = archive::read_page(path, kind, name) else {
            continue;
        };
        if is_cancelled(cancel) {
            bail!("import cancelled");
        }
        if let Ok(jpeg) =
            imaging::make_cover_jpeg(&bytes, imaging::COVER_MAX_W, imaging::COVER_MAX_H)
        {
            if is_cancelled(cancel) {
                bail!("import cancelled");
            }
            let _ = fs::write(dirs.root.join("covers").join(format!("{id}.jpg")), &jpeg);
            let _ = fs::create_dir_all(dirs.cover_dir_for_size(ThumbnailSize::Medium));
            let _ = fs::write(
                dirs.cover_dir_for_size(ThumbnailSize::Medium)
                    .join(format!("{id}.jpg")),
                &jpeg,
            );
            cover_written = true;
            break;
        }
    }
    if is_cancelled(cancel) {
        bail!("import cancelled");
    }
    if !cover_written {
        bail!("could not decode a cover image");
    }

    Ok(Book {
        id,
        title: title_for(path),
        series: None,
        path: path.to_path_buf(),
        kind,
        page_count: pages.len(),
        current_page: 0,
        opened: false,
        added_at: now_secs(),
        last_read: None,
        marks: BTreeMap::new(),
    })
}

/// Stable id derived from the canonical path, so re-importing the same file is a no-op and
/// progress survives the file being replaced by an updated copy.
fn book_id(path: &Path) -> String {
    format!("{:016x}", fnv1a(path.to_string_lossy().as_bytes()))
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn title_for(path: &Path) -> String {
    let raw = if path.is_dir() {
        display_name(path)
    } else {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| display_name(path))
    };
    let cleaned = raw.replace('_', " ");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        raw
    } else {
        cleaned.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgba, RgbaImage};
    use std::io::{Cursor, Write};

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, Rgba([120, 60, 200, 255]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn write_cbz(path: &Path, pages: &[&str]) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for name in pages {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(&png(64, 96)).unwrap();
        }
        zip.finish().unwrap();
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yomu-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn imports_cbz_with_natural_page_order_and_cover() {
        let tmp = temp_dir("cbz");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let cbz = tmp.join("Blue_Period_v01.cbz");
        write_cbz(
            &cbz,
            &["10.png", "2.png", "1.png", "__MACOSX/._1.png", "notes.txt"],
        );

        let report = import_paths(std::slice::from_ref(&cbz), &dirs, &HashSet::new());
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.added.len(), 1);

        let book = &report.added[0];
        assert_eq!(book.title, "Blue Period v01");
        assert_eq!(book.page_count, 3);
        assert!(dirs.cover(&book.id).exists());

        let pages = archive::list_pages(&book.path, book.kind).unwrap();
        assert_eq!(pages, vec!["1.png", "2.png", "10.png"]);
        let bytes = archive::read_page(&book.path, book.kind, &pages[0]).unwrap();
        assert!(!bytes.is_empty());

        let again = import_paths(
            &[cbz],
            &dirs,
            &report.added.iter().map(|b| b.id.clone()).collect(),
        );
        assert_eq!(again.skipped, 1);
        assert!(again.added.is_empty());
    }

    #[test]
    fn scans_folders_recursively() {
        let tmp = temp_dir("scan");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let root = tmp.join("manga");
        fs::create_dir_all(root.join("Series A")).unwrap();
        fs::create_dir_all(root.join("Loose Chapter")).unwrap();
        write_cbz(&root.join("Series A").join("v1.cbz"), &["1.png"]);
        write_cbz(&root.join("Series A").join("v2.cbz"), &["1.png"]);
        fs::write(root.join("Loose Chapter").join("001.png"), png(32, 48)).unwrap();
        fs::write(root.join("readme.txt"), "hi").unwrap();

        let report = import_paths(&[root], &dirs, &HashSet::new());
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.added.len(), 3);
        assert!(report.added.iter().any(|b| b.kind == SourceKind::Folder));
    }

    #[test]
    fn progress_and_persistence_roundtrip() {
        let tmp = temp_dir("persist");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let mut lib = Library::default();
        let mut book = Book {
            id: "abc".into(),
            title: "Test".into(),
            series: None,
            path: tmp.join("x.cbz"),
            kind: SourceKind::Archive,
            page_count: 10,
            current_page: 0,
            opened: false,
            added_at: 1,
            last_read: None,
            marks: BTreeMap::new(),
        };
        assert_eq!(book.progress(), 0.0);
        book.opened = true;
        book.current_page = 4;
        assert!((book.progress() - 0.5).abs() < 1e-6);
        book.current_page = 9;
        assert!(book.is_finished());

        book.marks.insert("a".into(), 3);
        assert!(lib.add(book.clone()));
        assert!(!lib.add(book));
        lib.settings_mut().animate = false;
        lib.settings_mut().fit = FitMode::Width;
        lib.save(&dirs).unwrap();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.books().len(), 1);
        assert_eq!(loaded.get("abc").unwrap().current_page, 9);
        assert_eq!(loaded.get("abc").unwrap().marks.get("a"), Some(&3));
        assert!(!loaded.settings().animate);
        assert_eq!(loaded.settings().fit, FitMode::Width);
    }

    #[test]
    fn old_library_files_without_settings_still_load() {
        let tmp = temp_dir("compat");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let json = r#"{"books":[{"id":"x","title":"Old","path":"/tmp/old.cbz","kind":"Archive","page_count":5,"added_at":1}]}"#;
        fs::write(dirs.library_file(), json).unwrap();
        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.books().len(), 1);
        assert!(loaded.settings().animate);
        assert_eq!(loaded.settings().fit, FitMode::Page);
        assert!(loaded.get("x").unwrap().marks.is_empty());
    }

    fn sample_book() -> Book {
        Book {
            id: "b".into(),
            title: "Sample".into(),
            series: None,
            path: PathBuf::from("/tmp/sample.cbz"),
            kind: SourceKind::Archive,
            page_count: 10,
            current_page: 0,
            opened: false,
            added_at: 1,
            last_read: None,
            marks: BTreeMap::new(),
        }
    }

    #[test]
    fn read_status_and_progress_changes() {
        let mut book = sample_book();
        assert_eq!(book.status(), ReadStatus::Unread);
        assert!(!book.has_progress());

        // Opening a book without turning a page is still "unread"
        book.opened = true;
        assert_eq!(book.status(), ReadStatus::Unread);

        book.current_page = 4;
        assert_eq!(book.status(), ReadStatus::InProgress);
        assert!(book.has_progress());

        book.mark_read();
        assert_eq!(book.status(), ReadStatus::Finished);
        assert_eq!(book.current_page, 9);
        assert!((book.progress() - 1.0).abs() < 1e-6);

        book.reset_progress();
        assert_eq!(book.status(), ReadStatus::Unread);
        assert_eq!(book.progress(), 0.0);
        assert!(book.last_read.is_none());

        book.current_page = 6;
        book.opened = true;
        book.start_over();
        assert_eq!(book.current_page, 0);
        assert!(book.opened);
    }

    #[test]
    fn reset_keeps_marks() {
        let mut book = sample_book();
        book.marks.insert("a".into(), 3);
        book.mark_read();
        book.reset_progress();
        assert_eq!(book.marks.get("a"), Some(&3));
    }

    #[test]
    fn opacity_is_clamped_and_rounded() {
        assert_eq!(clamp_opacity(2.0), OPACITY_MAX);
        assert_eq!(clamp_opacity(0.0), OPACITY_MIN);
        assert_eq!(clamp_opacity(-1.0), OPACITY_MIN);
        assert_eq!(clamp_opacity(f32::NAN), OPACITY_MAX);
        assert_eq!(clamp_opacity(0.8049), 0.80);
        assert_eq!(Settings::default().opacity, OPACITY_MAX);
    }

    #[test]
    fn invalid_stored_settings_fall_back_instead_of_breaking_the_library() {
        let tmp = temp_dir("badsettings");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let json = r#"{"books":[],"settings":{"rtl":"yes","fit":"Sideways","opacity":"lots"}}"#;
        fs::write(dirs.library_file(), json).unwrap();
        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(*loaded.settings(), Settings::default());
        assert!(!dirs.library_file().with_extension("json.corrupt").exists());

        let json =
            r#"{"version":2,"books":[],"settings":{"animate":"yes","fit":"Width","opacity":7.5}}"#;
        fs::write(dirs.library_file(), json).unwrap();
        let loaded = Library::load(&dirs).unwrap();
        assert!(
            loaded.settings().animate,
            "a mistyped value falls back to the default"
        );
        assert_eq!(loaded.settings().fit, FitMode::Width);
        assert_eq!(loaded.settings().opacity, OPACITY_MAX);

        let json = r#"{"books":[],"settings":{"opacity":0.1}}"#;
        fs::write(dirs.library_file(), json).unwrap();
        assert_eq!(
            Library::load(&dirs).unwrap().settings().opacity,
            OPACITY_MIN
        );

        let json = r#"{"books":[],"settings":[1,2,3]}"#;
        fs::write(dirs.library_file(), json).unwrap();
        assert_eq!(
            *Library::load(&dirs).unwrap().settings(),
            Settings::default()
        );
    }

    #[test]
    fn opacity_persists_across_save_and_load() {
        let tmp = temp_dir("opacity");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let mut lib = Library::default();
        lib.settings_mut().set_opacity(0.65);
        lib.save(&dirs).unwrap();
        assert_eq!(Library::load(&dirs).unwrap().settings().opacity, 0.65);
    }

    #[test]
    fn unsupported_file_reports_error() {
        let tmp = temp_dir("bad");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let pdf = tmp.join("book.pdf");
        fs::write(&pdf, b"%PDF-1.4").unwrap();
        let report = import_paths(&[pdf], &dirs, &HashSet::new());
        assert!(report.added.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("unsupported"));
    }

    #[test]
    fn imports_cbr_archive() {
        let tmp = temp_dir("cbr");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let cbr = tmp.join("Sample_Manga.cbr");
        let file = fs::File::create(&cbr).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("001.png", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&png(64, 96)).unwrap();
        zip.finish().unwrap();

        let report = import_paths(std::slice::from_ref(&cbr), &dirs, &HashSet::new());
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.added.len(), 1);
        let book = &report.added[0];
        assert_eq!(book.title, "Sample Manga");
        assert_eq!(book.page_count, 1);
        assert!(dirs.cover(&book.id).exists());
    }

    #[test]
    fn border_size_and_corner_rounding_persistence() {
        let tmp = temp_dir("border-rounding");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let mut lib = Library::default();
        lib.settings_mut().border_size = BorderSize::Thick;
        lib.settings_mut().corner_rounding = CornerRounding::Rounded;
        lib.save(&dirs).unwrap();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().border_size, BorderSize::Thick);
        assert_eq!(loaded.settings().corner_rounding, CornerRounding::Rounded);

        // Lenient loading with missing or deprecated fields defaults to Thin / Standard
        let json = r#"{"books":[],"settings":{"border_size":"bogus"}}"#;
        fs::write(dirs.library_file(), json).unwrap();
        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().border_size, BorderSize::Thin);
        assert_eq!(loaded.settings().corner_rounding, CornerRounding::Standard);

        // Extended options like none and round persist and deserialize
        let json = r#"{"books":[],"settings":{"border_size":"none","corner_rounding":"round","thumbnail_size":"huge"}}"#;
        fs::write(dirs.library_file(), json).unwrap();
        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().border_size, BorderSize::None);
        assert_eq!(loaded.settings().corner_rounding, CornerRounding::Rounded);
        assert_eq!(loaded.settings().thumbnail_size, ThumbnailSize::Large);
    }

    #[test]
    fn legacy_theme_names_are_migrated() {
        let tmp = temp_dir("legacy-themes");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();

        let json_dark = r#"{"books":[],"settings":{"theme_name":"Default Dark"}}"#;
        fs::write(dirs.library_file(), json_dark).unwrap();
        assert_eq!(
            Library::load(&dirs).unwrap().settings().theme_name,
            "Catppuccin Mocha"
        );

        let json_light = r#"{"books":[],"settings":{"theme_name":"Default Light"}}"#;
        fs::write(dirs.library_file(), json_light).unwrap();
        assert_eq!(
            Library::load(&dirs).unwrap().settings().theme_name,
            "Catppuccin Latte"
        );

        let json_plum = r#"{"books":[],"settings":{"theme_name":"Soft Plum"}}"#;
        fs::write(dirs.library_file(), json_plum).unwrap();
        assert_eq!(
            Library::load(&dirs).unwrap().settings().theme_name,
            "Catppuccin Mocha"
        );
    }

    #[test]
    fn remember_zoom_and_zoom_persistence() {
        let tmp = temp_dir("zoom-persist");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let mut lib = Library::default();
        assert!(lib.settings().remember_zoom);
        assert_eq!(lib.settings().zoom, 1.0);

        lib.settings_mut().remember_zoom = false;
        lib.settings_mut().zoom = 1.75;
        lib.save(&dirs).unwrap();

        let loaded = Library::load(&dirs).unwrap();
        assert!(!loaded.settings().remember_zoom);
        assert_eq!(loaded.settings().zoom, 1.75);
    }

    #[test]
    fn import_cancellable_stops_early_when_token_set_initially() {
        let tmp = temp_dir("cancel-init");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let cbz1 = tmp.join("vol1.cbz");
        let cbz2 = tmp.join("vol2.cbz");
        write_cbz(&cbz1, &["1.png"]);
        write_cbz(&cbz2, &["1.png"]);

        let cancel = Arc::new(AtomicBool::new(true));
        let mut reported = Vec::new();
        let report = import_paths_with_progress_cancellable(
            &[cbz1, cbz2],
            &dirs,
            &HashSet::new(),
            Some(&cancel),
            |b| reported.push(b.id.clone()),
        );

        assert!(report.added.is_empty(), "no books should be added");
        assert!(report.errors.is_empty(), "cancellation is not an error");
        assert_eq!(report.skipped, 0);
        assert!(reported.is_empty());
    }

    #[test]
    fn import_cancellable_returns_partial_results_on_midway_cancellation() {
        let tmp = temp_dir("cancel-midway");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let cbz1 = tmp.join("a_vol1.cbz");
        let cbz2 = tmp.join("b_vol2.cbz");
        let cbz3 = tmp.join("c_vol3.cbz");
        write_cbz(&cbz1, &["1.png"]);
        write_cbz(&cbz2, &["1.png"]);
        write_cbz(&cbz3, &["1.png"]);

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_hook = Arc::clone(&cancel);
        let mut reported = Vec::new();

        let report = import_paths_with_progress_cancellable(
            &[cbz1, cbz2, cbz3],
            &dirs,
            &HashSet::new(),
            Some(&cancel),
            |book| {
                reported.push(book.id.clone());
                // Cancel immediately after the first book is reported
                cancel_hook.store(true, Ordering::Relaxed);
            },
        );

        assert_eq!(report.added.len(), 1, "only first book should be added");
        assert_eq!(reported.len(), 1);
        assert_eq!(report.added[0].title, "a vol1");
        assert!(
            dirs.cover(&report.added[0].id).exists(),
            "first book cover must exist"
        );
        assert!(
            report.errors.is_empty(),
            "cancellation must not add errors: {:?}",
            report.errors
        );
    }

    #[test]
    fn import_cancellable_stops_during_recursive_folder_scan() {
        let tmp = temp_dir("cancel-scan");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let root = tmp.join("manga");
        fs::create_dir_all(root.join("Sub1")).unwrap();
        fs::create_dir_all(root.join("Sub2")).unwrap();
        write_cbz(&root.join("Sub1").join("v1.cbz"), &["1.png"]);
        write_cbz(&root.join("Sub2").join("v2.cbz"), &["1.png"]);

        let cancel = Arc::new(AtomicBool::new(true));
        let report = import_paths_with_progress_cancellable(
            &[root],
            &dirs,
            &HashSet::new(),
            Some(&cancel),
            |_| {},
        );

        assert!(report.added.is_empty());
        assert!(report.errors.is_empty());
    }
}
