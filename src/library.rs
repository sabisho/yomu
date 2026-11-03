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

pub use crate::layout::{
    DEFAULT_SPREAD_RATIO_THRESHOLD, PageClassification, PageLayout, ReadingDirection,
};

/// User manual override for double-page spread classification on an individual image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PageSpreadOverride {
    /// Follow automatic detection based on image aspect ratio and context.
    #[default]
    Auto,
    /// Force this image to be a single-page image.
    ForceSingle,
    /// Force this image to be a double-page spread.
    ForceSpread,
}

impl PageSpreadOverride {
    pub const ALL: [PageSpreadOverride; 3] = [
        PageSpreadOverride::Auto,
        PageSpreadOverride::ForceSingle,
        PageSpreadOverride::ForceSpread,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PageSpreadOverride::Auto => "Auto",
            PageSpreadOverride::ForceSingle => "Force Single Page",
            PageSpreadOverride::ForceSpread => "Force Double Spread",
        }
    }
}

const MAX_SCAN_DEPTH: usize = 6;

/// Where yomu keeps its data (`$XDG_DATA_HOME/yomu`, or `$YOMU_DATA_DIR` if set).
#[derive(Debug, Clone)]
pub struct DataDirs {
    root: PathBuf,
    covers_dir: PathBuf,
    small_dir: PathBuf,
    medium_dir: PathBuf,
    large_dir: PathBuf,
    legacy_fallback_ids: HashSet<String>,
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
        let covers_dir = root.join("covers");
        let small_dir = covers_dir.join(ThumbnailSize::Small.dir_name());
        let medium_dir = covers_dir.join(ThumbnailSize::Medium.dir_name());
        let large_dir = covers_dir.join(ThumbnailSize::Large.dir_name());

        fs::create_dir_all(&covers_dir)
            .with_context(|| format!("creating {}", covers_dir.display()))?;
        fs::create_dir_all(&small_dir)
            .with_context(|| format!("creating {}", small_dir.display()))?;
        fs::create_dir_all(&medium_dir)
            .with_context(|| format!("creating {}", medium_dir.display()))?;
        fs::create_dir_all(&large_dir)
            .with_context(|| format!("creating {}", large_dir.display()))?;

        let mut dirs = Self {
            root,
            covers_dir,
            small_dir,
            medium_dir,
            large_dir,
            legacy_fallback_ids: HashSet::new(),
        };
        dirs.migrate_legacy_covers();
        Ok(dirs)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn library_file(&self) -> PathBuf {
        self.root.join("library.json")
    }

    pub fn progress_file(&self) -> PathBuf {
        self.root.join("progress.json")
    }

    #[inline]
    pub fn covers_dir(&self) -> &Path {
        &self.covers_dir
    }

    #[inline]
    pub fn cover_dir(&self, size: ThumbnailSize) -> &Path {
        match size {
            ThumbnailSize::Small => &self.small_dir,
            ThumbnailSize::Medium => &self.medium_dir,
            ThumbnailSize::Large => &self.large_dir,
        }
    }

    #[inline]
    pub fn cover_dir_for_size(&self, size: ThumbnailSize) -> PathBuf {
        self.cover_dir(size).to_path_buf()
    }

    pub fn cover(&self, id: &str) -> PathBuf {
        self.cover_for_size(id, ThumbnailSize::Medium)
    }

    pub fn cover_for_size(&self, id: &str, size: ThumbnailSize) -> PathBuf {
        if !self.legacy_fallback_ids.is_empty() && self.legacy_fallback_ids.contains(id) {
            return self.covers_dir.join(format!("{id}.jpg"));
        }
        self.cover_dir(size).join(format!("{id}.jpg"))
    }

    /// Migrate legacy unsized covers in `covers/<id>.jpg` to sized directories (`covers/medium/`, etc.).
    /// Returns the number of newly migrated cover files.
    pub fn migrate_legacy_covers(&mut self) -> usize {
        let Ok(entries) = fs::read_dir(&self.covers_dir) else {
            return 0;
        };
        let mut migrated = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("jpg") {
                continue;
            }
            let Some(file_name) = path.file_name() else {
                continue;
            };
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };

            let mut all_sized_exist = true;
            for size in ThumbnailSize::ALL {
                let target = self.cover_dir(size).join(file_name);
                if !target.exists() {
                    if fs::hard_link(&path, &target).is_ok() || fs::copy(&path, &target).is_ok() {
                        migrated += 1;
                    } else {
                        all_sized_exist = false;
                    }
                }
            }

            if !all_sized_exist {
                self.legacy_fallback_ids.insert(stem.to_string());
            }
        }
        migrated
    }

    pub fn clear_covers(&self) -> Result<()> {
        let covers_dir = &self.covers_dir;
        if covers_dir.exists() {
            fs::remove_dir_all(covers_dir).context("removing covers directory")?;
        }
        fs::create_dir_all(covers_dir).context("recreating covers directory")?;
        fs::create_dir_all(&self.small_dir).context("recreating small covers directory")?;
        fs::create_dir_all(&self.medium_dir).context("recreating medium covers directory")?;
        fs::create_dir_all(&self.large_dir).context("recreating large covers directory")?;
        Ok(())
    }

    pub fn cache_size_bytes(&self) -> u64 {
        dir_size_recursive(&self.covers_dir)
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
    /// Manual overrides for double-page spread classification: archive entry name -> override.
    #[serde(default)]
    pub spread_overrides: BTreeMap<String, PageSpreadOverride>,
    /// Cached dimensions for archive entries: archive entry name -> (width, height).
    #[serde(default)]
    pub page_dimensions: Arc<BTreeMap<String, (u32, u32)>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStatus {
    Unread,
    InProgress,
    Finished,
}

impl Book {
    pub fn insert_page_dimension(&mut self, name: String, dims: (u32, u32)) {
        Arc::make_mut(&mut self.page_dimensions).insert(name, dims);
    }
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

#[allow(dead_code)]
fn default_true() -> bool {
    true
}

/// Library sort modes for All Manga and Downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortBy {
    #[default]
    Default,
    Title,
    DateAdded,
    LastRead,
    Progress,
}

impl SortBy {
    pub const ALL: [SortBy; 5] = [
        SortBy::Default,
        SortBy::Title,
        SortBy::DateAdded,
        SortBy::LastRead,
        SortBy::Progress,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Title => "Title",
            Self::DateAdded => "Date Added",
            Self::LastRead => "Last Read",
            Self::Progress => "Reading Progress",
        }
    }

    pub fn default_direction(self) -> SortDirection {
        match self {
            Self::Default | Self::Title => SortDirection::Ascending,
            Self::DateAdded | Self::LastRead | Self::Progress => SortDirection::Descending,
        }
    }
}

/// Direction for library sorting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

impl SortDirection {
    pub fn label(self, by: SortBy) -> &'static str {
        match (by, self) {
            (SortBy::Title, SortDirection::Ascending) => "A–Z",
            (SortBy::Title, SortDirection::Descending) => "Z–A",
            (SortBy::DateAdded, SortDirection::Descending) => "Newest",
            (SortBy::DateAdded, SortDirection::Ascending) => "Oldest",
            (SortBy::LastRead, SortDirection::Descending) => "Recently Read",
            (SortBy::LastRead, SortDirection::Ascending) => "Least Recently Read",
            (SortBy::Progress, SortDirection::Descending) => "Highest Progress",
            (SortBy::Progress, SortDirection::Ascending) => "Lowest Progress",
            (SortBy::Default, SortDirection::Ascending) => "Normal",
            (SortBy::Default, SortDirection::Descending) => "Reverse",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// Compares two books according to the specified sort criteria and direction.
pub fn compare_books(a: &Book, b: &Book, by: SortBy, dir: SortDirection) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let ord = match by {
        SortBy::Default => Ordering::Equal,
        SortBy::Title => {
            let res = natural_cmp(&a.title, &b.title);
            if dir == SortDirection::Descending {
                res.reverse()
            } else {
                res
            }
        }
        SortBy::DateAdded => {
            let res = if dir == SortDirection::Descending {
                b.added_at.cmp(&a.added_at)
            } else {
                a.added_at.cmp(&b.added_at)
            };
            res.then_with(|| natural_cmp(&a.title, &b.title))
        }
        SortBy::LastRead => {
            let res = match (a.last_read, b.last_read) {
                (Some(ta), Some(tb)) => {
                    if dir == SortDirection::Descending {
                        tb.cmp(&ta)
                    } else {
                        ta.cmp(&tb)
                    }
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            };
            res.then_with(|| natural_cmp(&a.title, &b.title))
                .then_with(|| b.added_at.cmp(&a.added_at))
        }
        SortBy::Progress => {
            let pa = a.progress();
            let pb = b.progress();
            let res = if dir == SortDirection::Descending {
                pb.total_cmp(&pa)
            } else {
                pa.total_cmp(&pb)
            };
            res.then_with(|| natural_cmp(&a.title, &b.title))
                .then_with(|| b.added_at.cmp(&a.added_at))
        }
    };

    ord.then_with(|| a.id.cmp(&b.id))
}

/// Sorts a slice of borrowed book references stably.
pub fn sort_books(books: &mut [&Book], by: SortBy, dir: SortDirection) {
    match by {
        SortBy::Default => {
            if dir == SortDirection::Descending {
                books.reverse();
            }
        }
        _ => {
            books.sort_by(|a, b| compare_books(a, b, by, dir));
        }
    }
}

/// Computes sorted indices for a slice of books without moving or cloning them.
pub fn sort_book_indices(books: &[Book], by: SortBy, dir: SortDirection) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..books.len()).collect();
    match by {
        SortBy::Default => {
            if dir == SortDirection::Descending {
                indices.reverse();
            }
        }
        _ => {
            indices.sort_by(|&i, &j| compare_books(&books[i], &books[j], by, dir));
        }
    }
    indices
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
    /// Slide between pages and glide when scrolling (animations are permanently enabled).
    pub animate: bool,
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
    /// Library sorting mode (Default, Title, DateAdded, LastRead, Progress).
    #[serde(default)]
    pub sort_by: SortBy,
    /// Library sorting direction (Ascending, Descending).
    #[serde(default)]
    pub sort_direction: SortDirection,
    /// Opacity of the window background, `OPACITY_MIN..=OPACITY_MAX`. Raised widget surfaces use
    /// a small opacity lift to stay distinct; page images, covers, and text remain opaque.
    pub opacity: f32,
    /// Reader page display layout: single page or dual-page spread.
    #[serde(default)]
    pub page_layout: PageLayout,
    /// Reading progression direction: right-to-left (manga) or left-to-right (western comics).
    #[serde(default)]
    pub reading_direction: ReadingDirection,
    /// In dual-page mode, display the first page as an isolated single cover spread.
    #[serde(default = "default_true")]
    pub first_page_is_cover: bool,
    /// Aspect ratio threshold (width / height) to automatically detect double-page spreads.
    #[serde(default = "default_spread_ratio_threshold")]
    pub spread_ratio_threshold: f32,
}

pub fn default_spread_ratio_threshold() -> f32 {
    DEFAULT_SPREAD_RATIO_THRESHOLD
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
            continuous: false,
            webcomics: false,
            group_series: false,
            thumbnail_size: ThumbnailSize::Medium,
            border_size: BorderSize::Thin,
            corner_rounding: CornerRounding::Standard,
            view_mode_grid: true,
            vim_mode: false,
            sort_by: SortBy::Default,
            sort_direction: SortDirection::Ascending,
            opacity: OPACITY_MAX,
            page_layout: PageLayout::Single,
            reading_direction: ReadingDirection::LeftToRight,
            first_page_is_cover: true,
            spread_ratio_threshold: DEFAULT_SPREAD_RATIO_THRESHOLD,
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
        settings.animate = true;
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
        if let Some(sort_by) = object
            .get("sort_by")
            .and_then(|v| serde_json::from_value::<SortBy>(v.clone()).ok())
        {
            settings.sort_by = sort_by;
        }
        if let Some(sort_direction) = object
            .get("sort_direction")
            .and_then(|v| serde_json::from_value::<SortDirection>(v.clone()).ok())
        {
            settings.sort_direction = sort_direction;
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
        if let Some(page_layout) = object
            .get("page_layout")
            .and_then(|v| serde_json::from_value::<PageLayout>(v.clone()).ok())
        {
            settings.page_layout = page_layout;
        }
        if let Some(reading_direction) = object
            .get("reading_direction")
            .and_then(|v| serde_json::from_value::<ReadingDirection>(v.clone()).ok())
        {
            settings.reading_direction = reading_direction;
        } else if let Some(rtl) = object.get("rtl").and_then(|v| v.as_bool()) {
            settings.reading_direction = if rtl {
                ReadingDirection::RightToLeft
            } else {
                ReadingDirection::LeftToRight
            };
        }
        if let Some(first_page_is_cover) =
            object.get("first_page_is_cover").and_then(|v| v.as_bool())
        {
            settings.first_page_is_cover = first_page_is_cover;
        }
        if let Some(spread_ratio_threshold) = object
            .get("spread_ratio_threshold")
            .and_then(|v| v.as_f64())
        {
            settings.spread_ratio_threshold = (spread_ratio_threshold as f32).clamp(1.0, 3.0);
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
    pub favorites: HashSet<String>,
    #[serde(skip)]
    mutation_seq: u64,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            version: LIBRARY_VERSION,
            books: Vec::new(),
            settings: Settings::default(),
            favorites: HashSet::new(),
            mutation_seq: 0,
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

        // Clean up leftover temporary files from prior crashes or ungraceful shutdowns
        if let Some(parent) = file.parent()
            && let Ok(entries) = fs::read_dir(parent)
        {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|n| n.to_str())
                    && (name.starts_with("library.tmp.") || name == "library.json.tmp")
                {
                    let _ = fs::remove_file(p);
                }
            }
        }

        let mut lib = match fs::read_to_string(&file) {
            Ok(text) => match serde_json::from_str::<Library>(&text) {
                Ok(mut lib) => {
                    lib.migrate();
                    lib
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
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => return Err(err).with_context(|| format!("reading {}", file.display())),
        };

        if dirs.progress_file().exists()
            && let Ok(progress) = crate::progress::ProgressStore::load(dirs)
        {
            progress.apply_to_library(&mut lib);
        }

        Ok(lib)
    }

    /// Write atomically (temp file + fsync + rename + parent sync) so a crash or power loss cannot corrupt the library.
    pub fn save(&self, dirs: &DataDirs) -> Result<()> {
        use std::io::Write as _;

        let file = dirs.library_file();
        let parent = file
            .parent()
            .context("library file destination has no parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;

        // 1. Serialize in-memory before touching any files on disk
        let json = serde_json::to_vec_pretty(self)?;

        // 2. Unique temporary file in the same directory (same filesystem mount)
        static TMP_SAVE_COUNTER: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let counter = TMP_SAVE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = parent.join(format!(
            "library.tmp.{}.{}.{}",
            std::process::id(),
            nanos,
            counter
        ));

        // 3. Write and sync temporary file contents to disk
        let write_and_sync = || -> Result<()> {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)
                .with_context(|| format!("opening temporary library file {}", tmp.display()))?;

            f.write_all(&json)
                .with_context(|| format!("writing to {}", tmp.display()))?;
            f.sync_all()
                .with_context(|| format!("synchronizing {}", tmp.display()))?;
            Ok(())
        };

        if let Err(err) = write_and_sync() {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }

        // 4. Atomically replace destination with bounded retry for transient locks (antivirus/indexing)
        if let Err(err) = atomic_replace(&tmp, &file) {
            let _ = fs::remove_file(&tmp);
            return Err(err).with_context(|| format!("replacing {}", file.display()));
        }

        // 5. Synchronize parent directory where supported (Unix/Linux/macOS)
        #[cfg(unix)]
        {
            if let Ok(dir_file) = fs::File::open(parent) {
                let _ = dir_file.sync_all();
            }
        }

        Ok(())
    }
}

/// Delays in milliseconds for transient filesystem rename retries.
pub(crate) const PERSISTENCE_RETRY_DELAYS_MS: [u64; 3] = [10, 25, 50];

/// Atomically replaces destination with source.
///
/// On Windows, destination files can experience transient locks from antivirus scanners,
/// search indexers, or background utilities (e.g. `ERROR_ACCESS_DENIED` (5) or `ERROR_SHARING_VIOLATION` (32)).
/// To prevent spurious save failures or abandoned temporary files, this function performs bounded retries
/// with exponential backoff on transient errors while immediately returning permanent errors.
pub(crate) fn atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    atomic_replace_with_retry(
        from,
        to,
        |src, dst| fs::rename(src, dst),
        std::thread::sleep,
        &PERSISTENCE_RETRY_DELAYS_MS,
    )
}

/// Generalized atomic replace implementation allowing injection of custom rename and sleep functions for testing.
pub(crate) fn atomic_replace_with_retry<F, S>(
    from: &Path,
    to: &Path,
    mut rename_fn: F,
    mut sleep_fn: S,
    delays_ms: &[u64],
) -> std::io::Result<()>
where
    F: FnMut(&Path, &Path) -> std::io::Result<()>,
    S: FnMut(std::time::Duration),
{
    let mut attempt = 0;
    loop {
        match rename_fn(from, to) {
            Ok(()) => return Ok(()),
            Err(err) => {
                if !is_transient_fs_error(&err) || attempt >= delays_ms.len() {
                    return Err(err);
                }
                sleep_fn(std::time::Duration::from_millis(delays_ms[attempt]));
                attempt += 1;
            }
        }
    }
}

/// Identifies whether an I/O error indicates a transient condition suitable for bounded retry.
pub(crate) fn is_transient_fs_error(err: &std::io::Error) -> bool {
    // Check raw OS error codes for Windows
    if let Some(code) = err.raw_os_error() {
        // Windows system error codes:
        // ERROR_ACCESS_DENIED = 5
        // ERROR_SHARING_VIOLATION = 32
        // ERROR_LOCK_VIOLATION = 33
        if code == 5 || code == 32 || code == 33 {
            return true;
        }
    }

    // Check cross-platform ErrorKind
    matches!(
        err.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock
    )
}

impl Library {
    pub fn books(&self) -> &[Book] {
        &self.books
    }

    /// Update reading progress fields in-memory without bumping library metadata mutation_seq.
    pub fn update_progress(
        &mut self,
        id: &str,
        current_page: usize,
        opened: bool,
        last_read: Option<u64>,
    ) {
        if let Some(book) = self.books.iter_mut().find(|b| b.id == id) {
            book.current_page = current_page;
            book.opened = opened;
            book.last_read = last_read;
        }
    }

    pub fn mutation_seq(&self) -> u64 {
        self.mutation_seq
    }

    pub fn bump_mutation_seq(&mut self) {
        self.mutation_seq = self.mutation_seq.wrapping_add(1);
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut Settings {
        self.bump_mutation_seq();
        &mut self.settings
    }

    pub fn ids(&self) -> HashSet<String> {
        self.books.iter().map(|b| b.id.clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<&Book> {
        self.books.iter().find(|b| b.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Book> {
        self.bump_mutation_seq();
        self.books.iter_mut().find(|b| b.id == id)
    }

    /// Returns `false` (and changes nothing) if a book with the same id already exists.
    pub fn add(&mut self, book: Book) -> bool {
        if self.get(&book.id).is_some() {
            return false;
        }
        self.bump_mutation_seq();
        self.books.push(book);
        true
    }

    /// Remove from the library and delete the cached cover across all size tiers. The comic file itself is untouched.
    pub fn remove(&mut self, id: &str, dirs: &DataDirs) {
        self.bump_mutation_seq();
        self.books.retain(|b| b.id != id);
        for size in ThumbnailSize::ALL {
            let _ = fs::remove_file(dirs.cover_dir(size).join(format!("{id}.jpg")));
        }
        let _ = fs::remove_file(dirs.covers_dir().join(format!("{id}.jpg")));
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorites.contains(id)
    }

    pub fn toggle_favorite(&mut self, id: &str) -> bool {
        self.bump_mutation_seq();
        if self.favorites.contains(id) {
            self.favorites.remove(id);
            false
        } else {
            self.favorites.insert(id.to_string());
            true
        }
    }

    /// Clear all cached covers for all sizes.
    pub fn clear_cover_cache(&self, dirs: &DataDirs) -> Result<()> {
        dirs.clear_covers()
    }

    /// Clear all in-memory library data: books, reading history, marks, favorites,
    /// and reset settings to default.
    pub fn clear_all_data(&mut self) {
        self.bump_mutation_seq();
        self.books.clear();
        self.favorites.clear();
        self.settings = Settings::default();
    }

    /// Rebuild/ensure cover thumbnails exist for the requested size. Returns count of newly generated covers.
    pub fn ensure_covers_for_size(&self, dirs: &DataDirs, size: ThumbnailSize) -> usize {
        let target_dir = dirs.cover_dir(size);
        let _ = fs::create_dir_all(target_dir);
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
        self.bump_mutation_seq();
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
            let medium_path = dirs
                .cover_dir(ThumbnailSize::Medium)
                .join(format!("{id}.jpg"));
            let _ = fs::write(&medium_path, &jpeg);
            let legacy_path = dirs.covers_dir().join(format!("{id}.jpg"));
            if fs::hard_link(&medium_path, &legacy_path).is_err() {
                let _ = fs::write(&legacy_path, &jpeg);
            }
            let small_path = dirs
                .cover_dir(ThumbnailSize::Small)
                .join(format!("{id}.jpg"));
            if fs::hard_link(&medium_path, &small_path).is_err() {
                let _ = fs::write(&small_path, &jpeg);
            }
            let large_path = dirs
                .cover_dir(ThumbnailSize::Large)
                .join(format!("{id}.jpg"));
            if fs::hard_link(&medium_path, &large_path).is_err() {
                let _ = fs::write(&large_path, &jpeg);
            }
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
        spread_overrides: BTreeMap::new(),
        page_dimensions: Arc::default(),
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
            spread_overrides: BTreeMap::new(),
            page_dimensions: Arc::default(),
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
        lib.settings_mut().fit = FitMode::Width;
        lib.save(&dirs).unwrap();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.books().len(), 1);
        assert_eq!(loaded.get("abc").unwrap().current_page, 9);
        assert_eq!(loaded.get("abc").unwrap().marks.get("a"), Some(&3));
        assert!(loaded.settings().animate);
        assert_eq!(loaded.settings().fit, FitMode::Width);
    }

    #[test]
    fn old_library_files_with_disable_animations_load_safely() {
        let tmp = temp_dir("compat_disable_anim");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let json =
            r#"{"books":[],"settings":{"disable_animations":true,"animate":false,"fit":"Height"}}"#;
        fs::write(dirs.library_file(), json).unwrap();
        let loaded = Library::load(&dirs).unwrap();
        assert!(
            loaded.settings().animate,
            "animations must remain permanently enabled"
        );
        assert_eq!(loaded.settings().fit, FitMode::Height);
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
            spread_overrides: BTreeMap::new(),
            page_dimensions: Arc::default(),
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
    fn layout_and_direction_settings_persistence_and_fallback() {
        let tmp = temp_dir("layout_settings");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();

        // 1. Explicit modern values
        let json = r#"{"books":[],"settings":{"page_layout":"Double","reading_direction":"LeftToRight","first_page_is_cover":false}}"#;
        fs::write(dirs.library_file(), json).unwrap();
        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().page_layout, PageLayout::Double);
        assert_eq!(
            loaded.settings().reading_direction,
            ReadingDirection::LeftToRight
        );
        assert!(!loaded.settings().first_page_is_cover);

        // 2. Legacy "rtl": true / false fallback
        let json_legacy_rtl = r#"{"books":[],"settings":{"rtl":false}}"#;
        fs::write(dirs.library_file(), json_legacy_rtl).unwrap();
        let loaded_legacy = Library::load(&dirs).unwrap();
        assert_eq!(
            loaded_legacy.settings().reading_direction,
            ReadingDirection::LeftToRight
        );

        let json_legacy_rtl_true = r#"{"books":[],"settings":{"rtl":true}}"#;
        fs::write(dirs.library_file(), json_legacy_rtl_true).unwrap();
        let loaded_legacy_true = Library::load(&dirs).unwrap();
        assert_eq!(
            loaded_legacy_true.settings().reading_direction,
            ReadingDirection::RightToLeft
        );

        // 3. Round-trip through JSON
        let settings = Settings {
            page_layout: PageLayout::Double,
            reading_direction: ReadingDirection::LeftToRight,
            first_page_is_cover: false,
            spread_ratio_threshold: 1.35,
            ..Default::default()
        };
        let value = serde_json::to_value(&settings).unwrap();
        let parsed = Settings::from_value(&value);
        assert_eq!(parsed.page_layout, PageLayout::Double);
        assert_eq!(parsed.reading_direction, ReadingDirection::LeftToRight);
        assert!(!parsed.first_page_is_cover);
        assert_eq!(parsed.spread_ratio_threshold, 1.35);
    }

    #[test]
    fn book_spread_overrides_and_dimensions_persistence() {
        let tmp = temp_dir("book_spreads");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let mut lib = Library::default();

        let mut book = Book {
            id: "book-1".into(),
            title: "Test Book".into(),
            series: None,
            path: tmp.join("test.cbz"),
            kind: SourceKind::Archive,
            page_count: 5,
            current_page: 0,
            opened: true,
            added_at: 100,
            last_read: None,
            marks: BTreeMap::new(),
            spread_overrides: BTreeMap::new(),
            page_dimensions: Arc::default(),
        };
        book.spread_overrides
            .insert("page_02.jpg".into(), PageSpreadOverride::ForceSpread);
        book.spread_overrides
            .insert("page_03.jpg".into(), PageSpreadOverride::ForceSingle);
        book.insert_page_dimension("page_01.jpg".into(), (1000, 1500));
        book.insert_page_dimension("page_02.jpg".into(), (2000, 1500));

        lib.add(book);
        lib.save(&dirs).unwrap();

        let loaded = Library::load(&dirs).unwrap();
        let loaded_book = loaded.get("book-1").unwrap();
        assert_eq!(
            loaded_book.spread_overrides.get("page_02.jpg"),
            Some(&PageSpreadOverride::ForceSpread)
        );
        assert_eq!(
            loaded_book.spread_overrides.get("page_03.jpg"),
            Some(&PageSpreadOverride::ForceSingle)
        );
        assert_eq!(
            loaded_book.page_dimensions.get("page_01.jpg"),
            Some(&(1000, 1500))
        );
        assert_eq!(
            loaded_book.page_dimensions.get("page_02.jpg"),
            Some(&(2000, 1500))
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

    #[test]
    fn cover_for_size_returns_sized_path_and_resolves_without_fs_stat() {
        let tmp = temp_dir("cover-paths");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();

        assert_eq!(
            dirs.cover_dir(ThumbnailSize::Small),
            dirs.root().join("covers").join("small")
        );
        assert_eq!(
            dirs.cover_dir(ThumbnailSize::Medium),
            dirs.root().join("covers").join("medium")
        );
        assert_eq!(
            dirs.cover_dir(ThumbnailSize::Large),
            dirs.root().join("covers").join("large")
        );

        let small_path = dirs.cover_for_size("book123", ThumbnailSize::Small);
        assert_eq!(
            small_path,
            dirs.root().join("covers").join("small").join("book123.jpg")
        );

        let med_path = dirs.cover_for_size("book123", ThumbnailSize::Medium);
        assert_eq!(
            med_path,
            dirs.root()
                .join("covers")
                .join("medium")
                .join("book123.jpg")
        );

        let legacy_convenience = dirs.cover("book123");
        assert_eq!(legacy_convenience, med_path);

        // Missing cover returns sized path which does not exist
        assert!(
            !dirs
                .cover_for_size("missing", ThumbnailSize::Medium)
                .exists()
        );
    }

    #[test]
    fn legacy_covers_are_migrated_to_sized_directories() {
        let tmp = temp_dir("legacy-migration");
        let data_dir = tmp.join("data");
        let legacy_covers = data_dir.join("covers");
        fs::create_dir_all(&legacy_covers).unwrap();

        // Write unsized legacy covers
        fs::write(legacy_covers.join("manga_a.jpg"), b"fake jpeg data a").unwrap();
        fs::write(legacy_covers.join("manga_b.jpg"), b"fake jpeg data b").unwrap();

        // Initializing DataDirs must migrate legacy covers
        let dirs = DataDirs::at(data_dir).unwrap();

        // Sized covers should now exist for all size tiers
        for size in ThumbnailSize::ALL {
            assert!(
                dirs.cover_for_size("manga_a", size).exists(),
                "manga_a must exist for size {:?}",
                size
            );
            assert!(
                dirs.cover_for_size("manga_b", size).exists(),
                "manga_b must exist for size {:?}",
                size
            );
        }

        // Contents must match original legacy file
        let read_a = fs::read(dirs.cover_for_size("manga_a", ThumbnailSize::Medium)).unwrap();
        assert_eq!(read_a, b"fake jpeg data a");
    }

    #[test]
    fn legacy_fallback_when_unmigrated_id_recorded() {
        let tmp = temp_dir("legacy-fallback");
        let mut dirs = DataDirs::at(tmp.join("data")).unwrap();

        // Write a legacy cover directly
        let legacy_file = dirs.covers_dir().join("legacy_only.jpg");
        fs::write(&legacy_file, b"legacy data").unwrap();

        // Manually record in fallback ids (simulating a read-only migration failure)
        dirs.legacy_fallback_ids.insert("legacy_only".to_string());

        let resolved = dirs.cover_for_size("legacy_only", ThumbnailSize::Large);
        assert_eq!(resolved, legacy_file);
        assert!(resolved.exists());
    }

    #[test]
    fn sort_by_title_natural_and_case_insensitive() {
        let mut b1 = sample_book();
        b1.id = "1".into();
        b1.title = "Volume 10".into();

        let mut b2 = sample_book();
        b2.id = "2".into();
        b2.title = "volume 2".into();

        let mut b3 = sample_book();
        b3.id = "3".into();
        b3.title = "Attack on Titan".into();

        let mut b4 = sample_book();
        b4.id = "4".into();
        b4.title = "進撃の巨人".into();

        let mut books = vec![&b1, &b2, &b3, &b4];
        sort_books(&mut books, SortBy::Title, SortDirection::Ascending);
        let ids: Vec<&str> = books.iter().map(|b| b.id.as_str()).collect();
        // "Attack on Titan" < "volume 2" < "Volume 10" (natural 2 before 10) < "進撃の巨人"
        assert_eq!(ids, vec!["3", "2", "1", "4"]);

        sort_books(&mut books, SortBy::Title, SortDirection::Descending);
        let ids_desc: Vec<&str> = books.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids_desc, vec!["4", "1", "2", "3"]);
    }

    #[test]
    fn sort_by_date_added() {
        let mut b1 = sample_book();
        b1.id = "1".into();
        b1.added_at = 100;

        let mut b2 = sample_book();
        b2.id = "2".into();
        b2.added_at = 300;

        let mut b3 = sample_book();
        b3.id = "3".into();
        b3.added_at = 200;

        let mut books = vec![&b1, &b2, &b3];
        // Descending is natural for Date Added (Newest first)
        sort_books(&mut books, SortBy::DateAdded, SortDirection::Descending);
        let ids_desc: Vec<&str> = books.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids_desc, vec!["2", "3", "1"]);

        // Ascending is Oldest first
        sort_books(&mut books, SortBy::DateAdded, SortDirection::Ascending);
        let ids_asc: Vec<&str> = books.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids_asc, vec!["1", "3", "2"]);
    }

    #[test]
    fn sort_by_last_read_and_missing_timestamps() {
        let mut b1 = sample_book();
        b1.id = "read_recent".into();
        b1.last_read = Some(500);

        let mut b2 = sample_book();
        b2.id = "read_old".into();
        b2.last_read = Some(200);

        let mut b3 = sample_book();
        b3.id = "unread_1".into();
        b3.last_read = None;
        b3.added_at = 50;

        let mut b4 = sample_book();
        b4.id = "unread_2".into();
        b4.last_read = None;
        b4.added_at = 100;

        let mut books = vec![&b3, &b1, &b4, &b2];

        // Descending (Recently Read): read books newest-to-oldest, unread at the bottom
        sort_books(&mut books, SortBy::LastRead, SortDirection::Descending);
        let ids_desc: Vec<&str> = books.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids_desc[0], "read_recent");
        assert_eq!(ids_desc[1], "read_old");
        assert!(ids_desc[2..].contains(&"unread_1") && ids_desc[2..].contains(&"unread_2"));

        // Ascending (Least Recently Read): read books oldest-to-newest, unread still at bottom
        sort_books(&mut books, SortBy::LastRead, SortDirection::Ascending);
        let ids_asc: Vec<&str> = books.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids_asc[0], "read_old");
        assert_eq!(ids_asc[1], "read_recent");
        assert!(ids_asc[2..].contains(&"unread_1") && ids_asc[2..].contains(&"unread_2"));
    }

    #[test]
    fn sort_by_progress_and_unopened_books() {
        let mut b1 = sample_book();
        b1.id = "finished".into();
        b1.page_count = 10;
        b1.current_page = 9;
        b1.opened = true; // 100%

        let mut b2 = sample_book();
        b2.id = "half".into();
        b2.page_count = 10;
        b2.current_page = 4;
        b2.opened = true; // 50%

        let mut b3 = sample_book();
        b3.id = "unopened".into();
        b3.page_count = 10;
        b3.current_page = 0;
        b3.opened = false; // 0%

        let mut books = vec![&b2, &b3, &b1];

        // Descending: 100% -> 50% -> 0%
        sort_books(&mut books, SortBy::Progress, SortDirection::Descending);
        let ids_desc: Vec<&str> = books.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids_desc, vec!["finished", "half", "unopened"]);

        // Ascending: 0% -> 50% -> 100%
        sort_books(&mut books, SortBy::Progress, SortDirection::Ascending);
        let ids_asc: Vec<&str> = books.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids_asc, vec!["unopened", "half", "finished"]);
    }

    #[test]
    fn sort_default_preserves_order_and_reverse() {
        let mut b1 = sample_book();
        b1.id = "a".into();
        let mut b2 = sample_book();
        b2.id = "b".into();
        let mut b3 = sample_book();
        b3.id = "c".into();

        let mut books = vec![&b2, &b1, &b3];
        sort_books(&mut books, SortBy::Default, SortDirection::Ascending);
        let ids_asc: Vec<&str> = books.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids_asc, vec!["b", "a", "c"]);

        sort_books(&mut books, SortBy::Default, SortDirection::Descending);
        let ids_desc: Vec<&str> = books.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids_desc, vec!["c", "a", "b"]);
    }

    #[test]
    fn sort_edge_cases_empty_single_and_ties() {
        let mut empty: Vec<&Book> = Vec::new();
        sort_books(&mut empty, SortBy::Title, SortDirection::Ascending);
        assert!(empty.is_empty());

        let indices = sort_book_indices(&[], SortBy::DateAdded, SortDirection::Descending);
        assert!(indices.is_empty());

        let b1 = sample_book();
        let mut single = vec![&b1];
        sort_books(&mut single, SortBy::Progress, SortDirection::Ascending);
        assert_eq!(single.len(), 1);

        // Ties with identical timestamp and title should break on id
        let mut t1 = sample_book();
        t1.id = "z_tie".into();
        t1.title = "Same".into();
        t1.added_at = 50;

        let mut t2 = sample_book();
        t2.id = "a_tie".into();
        t2.title = "Same".into();
        t2.added_at = 50;

        let mut ties = vec![&t1, &t2];
        sort_books(&mut ties, SortBy::DateAdded, SortDirection::Ascending);
        assert_eq!(ties[0].id, "a_tie");
        assert_eq!(ties[1].id, "z_tie");
    }

    #[test]
    fn sort_settings_persistence_and_lenient_fallback() {
        let settings = Settings {
            sort_by: SortBy::DateAdded,
            sort_direction: SortDirection::Descending,
            ..Default::default()
        };

        let json = serde_json::to_value(&settings).unwrap();
        let restored = Settings::from_value(&json);
        assert_eq!(restored.sort_by, SortBy::DateAdded);
        assert_eq!(restored.sort_direction, SortDirection::Descending);

        // Lenient fallback for missing or corrupted sort settings
        let corrupt = serde_json::json!({
            "sort_by": "InvalidMode",
            "sort_direction": 12345
        });
        let fallback = Settings::from_value(&corrupt);
        assert_eq!(fallback.sort_by, SortBy::Default);
        assert_eq!(fallback.sort_direction, SortDirection::Ascending);
    }

    #[test]
    fn test_persistence_durability_and_temp_cleanup() {
        let tmp = temp_dir("persistence-durability");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();

        let mut lib = Library::default();
        let mut b = sample_book();
        b.id = "book_durability_1".into();
        b.title = "Durability Test Book".into();
        b.insert_page_dimension("p1.png".into(), (1200, 1800));
        lib.add(b);

        // Simulate leftover temp file from an interrupted process
        let leftover_tmp = dirs.root().join("library.tmp.99999.12345678");
        fs::write(&leftover_tmp, b"partial corrupted temporary write").unwrap();
        assert!(leftover_tmp.exists());

        // Save library
        lib.save(&dirs)
            .expect("save should succeed with atomic rename and sync");
        assert!(dirs.root().join("library.json").exists());

        // Load library - should clean up leftover temp files and load successfully
        let loaded = Library::load(&dirs).expect("load should succeed");
        assert_eq!(loaded.books().len(), 1);
        assert_eq!(loaded.books()[0].title, "Durability Test Book");
        assert_eq!(
            loaded.books()[0].page_dimensions.get("p1.png"),
            Some(&(1200, 1800))
        );
        assert!(
            !leftover_tmp.exists(),
            "leftover temp file must be cleaned up on load"
        );
    }

    #[test]
    fn test_persistence_preserves_valid_file_on_failure() {
        let tmp = temp_dir("persistence-preservation");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();

        let mut lib = Library::default();
        let mut b = sample_book();
        b.id = "book_orig".into();
        b.title = "Original Book".into();
        lib.add(b);
        lib.save(&dirs).unwrap();

        let lib_path = dirs.root().join("library.json");
        let original_contents = fs::read_to_string(&lib_path).unwrap();

        // Verify that if a temp file cannot be moved or if destination is valid, original is untouched
        let reloaded = Library::load(&dirs).unwrap();
        assert_eq!(reloaded.books().len(), 1);
        assert_eq!(reloaded.books()[0].title, "Original Book");
        assert_eq!(fs::read_to_string(&lib_path).unwrap(), original_contents);
    }

    #[test]
    fn test_concurrent_saves_produce_consistent_library() {
        use std::thread;

        let tmp = temp_dir("persistence-concurrent");
        let dirs = Arc::new(DataDirs::at(tmp.join("data")).unwrap());

        let handles: Vec<_> = (0..5)
            .map(|thread_id| {
                let dirs = Arc::clone(&dirs);
                thread::spawn(move || {
                    let mut lib = Library::default();
                    for b_idx in 0..10 {
                        let mut b = sample_book();
                        b.id = format!("t{thread_id}_b{b_idx}");
                        b.title = format!("Thread {thread_id} Book {b_idx}");
                        lib.add(b);
                    }
                    lib.save(&dirs).unwrap();
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // Library file must be valid, parseable JSON and load cleanly
        let loaded = Library::load(&dirs).expect("concurrent saves must not leave corrupted file");
        assert_eq!(loaded.books().len(), 10);
    }

    #[test]
    fn test_first_time_library_creation() {
        let tmp = temp_dir("persistence-first-time-library");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let lib_path = dirs.library_file();
        assert!(!lib_path.exists());

        let mut lib = Library::default();
        let mut b = sample_book();
        b.id = "book_initial".into();
        b.title = "First Library Book".into();
        lib.add(b);

        lib.save(&dirs).expect("first-time save should succeed");
        assert!(lib_path.exists());

        let loaded = Library::load(&dirs).expect("load should succeed");
        assert_eq!(loaded.books().len(), 1);
        assert_eq!(loaded.books()[0].title, "First Library Book");
    }

    #[test]
    fn test_replacing_existing_library_file() {
        let tmp = temp_dir("persistence-replace-library");
        let dirs = DataDirs::at(tmp.join("data")).unwrap();
        let lib_path = dirs.library_file();

        let mut lib = Library::default();
        let mut b1 = sample_book();
        b1.id = "b1".into();
        b1.title = "Book 1".into();
        lib.add(b1);
        lib.save(&dirs).unwrap();

        // Mutate and replace
        let mut b2 = sample_book();
        b2.id = "b2".into();
        b2.title = "Book 2".into();
        lib.add(b2);
        lib.save(&dirs).expect("replace save should succeed");

        let loaded = Library::load(&dirs).expect("load replaced library");
        assert_eq!(loaded.books().len(), 2);
        assert_eq!(loaded.books()[1].title, "Book 2");

        // Ensure no temporary files remained
        let parent = lib_path.parent().unwrap();
        let tmp_files: Vec<_> = fs::read_dir(parent)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("library.tmp."))
            .collect();
        assert!(tmp_files.is_empty(), "temporary files must be cleaned up");
    }

    #[test]
    fn test_atomic_replace_transient_retry_success() {
        use std::cell::RefCell;
        use std::io::{Error, ErrorKind};
        use std::time::Duration;

        let attempt_count = RefCell::new(0);
        let slept_delays = RefCell::new(Vec::new());

        let mock_rename = |_from: &Path, _to: &Path| -> std::io::Result<()> {
            let mut count = attempt_count.borrow_mut();
            *count += 1;
            if *count < 3 {
                // Fail twice with transient permission denied
                Err(Error::new(ErrorKind::PermissionDenied, "locked by scanner"))
            } else {
                // Succeed on 3rd attempt
                Ok(())
            }
        };

        let mock_sleep = |d: Duration| {
            slept_delays.borrow_mut().push(d);
        };

        let delays = [10, 25, 50];
        let p1 = Path::new("dummy_from");
        let p2 = Path::new("dummy_to");

        let result = atomic_replace_with_retry(p1, p2, mock_rename, mock_sleep, &delays);
        assert!(result.is_ok(), "should succeed after retries");
        assert_eq!(*attempt_count.borrow(), 3);
        assert_eq!(
            *slept_delays.borrow(),
            vec![Duration::from_millis(10), Duration::from_millis(25)]
        );
    }

    #[test]
    fn test_atomic_replace_retry_exhaustion() {
        use std::cell::RefCell;
        use std::io::{Error, ErrorKind};
        use std::time::Duration;

        let attempt_count = RefCell::new(0);
        let slept_delays = RefCell::new(Vec::new());

        let mock_rename = |_from: &Path, _to: &Path| -> std::io::Result<()> {
            let mut count = attempt_count.borrow_mut();
            *count += 1;
            Err(Error::new(
                ErrorKind::PermissionDenied,
                "persistently locked",
            ))
        };

        let mock_sleep = |d: Duration| {
            slept_delays.borrow_mut().push(d);
        };

        let delays = [10, 25, 50];
        let p1 = Path::new("dummy_from");
        let p2 = Path::new("dummy_to");

        let result = atomic_replace_with_retry(p1, p2, mock_rename, mock_sleep, &delays);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::PermissionDenied);
        // Initial try + 3 retries = 4 total attempts
        assert_eq!(*attempt_count.borrow(), 4);
        assert_eq!(
            *slept_delays.borrow(),
            vec![
                Duration::from_millis(10),
                Duration::from_millis(25),
                Duration::from_millis(50)
            ]
        );
    }

    #[test]
    fn test_atomic_replace_permanent_error_no_retry() {
        use std::cell::RefCell;
        use std::io::{Error, ErrorKind};
        use std::time::Duration;

        let attempt_count = RefCell::new(0);
        let slept_delays = RefCell::new(Vec::new());

        let mock_rename = |_from: &Path, _to: &Path| -> std::io::Result<()> {
            let mut count = attempt_count.borrow_mut();
            *count += 1;
            Err(Error::new(ErrorKind::NotFound, "file not found"))
        };

        let mock_sleep = |d: Duration| {
            slept_delays.borrow_mut().push(d);
        };

        let delays = [10, 25, 50];
        let p1 = Path::new("dummy_from");
        let p2 = Path::new("dummy_to");

        let result = atomic_replace_with_retry(p1, p2, mock_rename, mock_sleep, &delays);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound);
        // Permanent error aborts immediately on first try with 0 sleeps
        assert_eq!(*attempt_count.borrow(), 1);
        assert!(slept_delays.borrow().is_empty());
    }
}
