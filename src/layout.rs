//! Reader geometry: fit modes, zoom and panning. Pure math with no gpui types, so it is unit tested.
//!
//! Terminology:
//! * *viewport*: the window area the page is shown in, in logical pixels.
//! * *display size*: how big the page is drawn, in logical pixels (may exceed the viewport).
//! * *offset*: how far the page has been scrolled from its top-left corner (always `>= 0`).
//! * *origin*: where the page's top-left corner lands relative to the viewport. When the page is
//!   smaller than the viewport on an axis it is centred; otherwise it is `-offset`.

use serde::{Deserialize, Serialize};

pub const ZOOM_MIN: f32 = 0.25;
pub const ZOOM_MAX: f32 = 8.0;
/// Multiplier for one zoom-in / zoom-out step.
pub const ZOOM_STEP: f32 = 1.25;

/// Gap between left and right pages in dual-page spread mode.
pub const SPREAD_GAP: f32 = 2.0;

/// Default aspect ratio threshold (width / height) to classify an image as a double-page spread.
pub const DEFAULT_SPREAD_RATIO_THRESHOLD: f32 = 1.25;

/// Minimum pixel dimension on both axes to qualify for double-page spread detection.
pub const MIN_SPREAD_DIMENSION: u32 = 200;

/// Decode sizes (longest side, in device pixels). Decoding at a few fixed tiers instead of the
/// exact size means zooming does not trigger a re-decode on every step.
pub const TIERS: [u32; 6] = [1024, 1536, 2048, 3072, 4096, 6144];

/// Reader page display layout: single page or dual-page spread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PageLayout {
    /// Show a single page at a time.
    #[default]
    Single,
    /// Show two pages side by side as one spread.
    Double,
}

impl PageLayout {
    pub const ALL: [PageLayout; 2] = [PageLayout::Single, PageLayout::Double];

    pub fn label(self) -> &'static str {
        match self {
            PageLayout::Single => "Single Page",
            PageLayout::Double => "Double Page",
        }
    }

    pub fn is_double(self) -> bool {
        matches!(self, PageLayout::Double)
    }

    pub fn toggle(self) -> Self {
        match self {
            PageLayout::Single => PageLayout::Double,
            PageLayout::Double => PageLayout::Single,
        }
    }
}

/// Reading progression direction: right-to-left (standard manga) or left-to-right (western comics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ReadingDirection {
    /// Left to right: page 1 on left, page 2 on right (western comic order).
    #[default]
    LeftToRight,
    /// Right to left: page 1 on right, page 2 on left (manga order).
    RightToLeft,
}

impl ReadingDirection {
    pub const ALL: [ReadingDirection; 2] =
        [ReadingDirection::RightToLeft, ReadingDirection::LeftToRight];

    pub fn label(self) -> &'static str {
        match self {
            ReadingDirection::RightToLeft => "Right to Left (Manga)",
            ReadingDirection::LeftToRight => "Left to Right",
        }
    }

    pub fn is_rtl(self) -> bool {
        matches!(self, ReadingDirection::RightToLeft)
    }

    pub fn toggle(self) -> Self {
        match self {
            ReadingDirection::RightToLeft => ReadingDirection::LeftToRight,
            ReadingDirection::LeftToRight => ReadingDirection::RightToLeft,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FitMode {
    /// The whole page is visible (best fit).
    #[default]
    Page,
    /// The page is as wide as the window; taller pages scroll vertically.
    Width,
    /// The page is as tall as the window; wider pages scroll horizontally.
    Height,
    /// One image pixel per screen pixel.
    Original,
}

impl FitMode {
    pub fn label(self) -> &'static str {
        match self {
            FitMode::Page => "fit page",
            FitMode::Width => "fit width",
            FitMode::Height => "fit height",
            FitMode::Original => "original size",
        }
    }

    pub fn next(self) -> Self {
        match self {
            FitMode::Page => FitMode::Width,
            FitMode::Width => FitMode::Height,
            FitMode::Height => FitMode::Original,
            FitMode::Original => FitMode::Page,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    /// Device pixels per logical pixel (the window's scale factor).
    pub scale: f32,
}

impl Viewport {
    pub const EMPTY: Viewport = Viewport {
        width: 0.0,
        height: 0.0,
        scale: 1.0,
    };
}

/// Where to put the scroll position after a page turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrive {
    /// Where reading starts: the top, on the side the reading direction starts from.
    Start,
    /// Where reading ends: the bottom, on the side the reading direction ends at.
    End,
}

/// Scale (display pixels per image pixel) for a fit mode at zoom 1.0.
pub fn base_scale(fit: FitMode, orig: (f32, f32), vp: &Viewport) -> f32 {
    let (ow, oh) = (orig.0.max(1.0), orig.1.max(1.0));
    match fit {
        FitMode::Page => (vp.width / ow).min(vp.height / oh),
        FitMode::Width => vp.width / ow,
        FitMode::Height => vp.height / oh,
        FitMode::Original => 1.0 / vp.scale.max(0.01),
    }
}

pub fn display_size(fit: FitMode, zoom: f32, orig: (f32, f32), vp: &Viewport) -> (f32, f32) {
    let scale = base_scale(fit, orig, vp) * zoom;
    (orig.0 * scale, orig.1 * scale)
}

/// Furthest the page can be scrolled on each axis.
pub fn max_offset(display: (f32, f32), vp: &Viewport) -> (f32, f32) {
    (
        (display.0 - vp.width).max(0.0),
        (display.1 - vp.height).max(0.0),
    )
}

pub fn clamp_offset(off: (f32, f32), display: (f32, f32), vp: &Viewport) -> (f32, f32) {
    let max = max_offset(display, vp);
    (off.0.clamp(0.0, max.0), off.1.clamp(0.0, max.1))
}

/// Top-left corner of the page relative to the viewport.
pub fn origin(off: (f32, f32), display: (f32, f32), vp: &Viewport) -> (f32, f32) {
    fn axis(off: f32, display: f32, view: f32) -> f32 {
        if display <= view {
            (view - display) / 2.0
        } else {
            -off.clamp(0.0, display - view)
        }
    }
    (
        axis(off.0, display.0, vp.width),
        axis(off.1, display.1, vp.height),
    )
}

/// The offset that keeps the image point under `anchor` (viewport coordinates) fixed while the
/// display size changes from `old_display` to `new_display`. Not clamped.
pub fn zoom_anchor_offset(
    old_origin: (f32, f32),
    old_display: (f32, f32),
    new_display: (f32, f32),
    anchor: (f32, f32),
) -> (f32, f32) {
    fn axis(old_origin: f32, old_display: f32, new_display: f32, anchor: f32) -> f32 {
        let fraction = if old_display > 0.0 {
            (anchor - old_origin) / old_display
        } else {
            0.5
        };
        -(anchor - fraction * new_display)
    }
    (
        axis(old_origin.0, old_display.0, new_display.0, anchor.0),
        axis(old_origin.1, old_display.1, new_display.1, anchor.1),
    )
}

pub fn arrival_offset(arrive: Arrive, max: (f32, f32)) -> (f32, f32) {
    match arrive {
        Arrive::Start => (0.0, 0.0),
        Arrive::End => (max.0, max.1),
    }
}

pub fn clamp_zoom(zoom: f32) -> f32 {
    zoom.clamp(ZOOM_MIN, ZOOM_MAX)
}

/// Smallest decode tier that covers `long_side_px` device pixels.
pub fn tier_for(long_side_px: f32) -> u32 {
    TIERS
        .iter()
        .copied()
        .find(|tier| *tier as f32 >= long_side_px)
        .unwrap_or(TIERS[TIERS.len() - 1])
}

/// Page classification for dual-page spread layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PageClassification {
    /// Normal single page (portrait or non-spread). Can be paired with another normal page into a two-image spread.
    #[default]
    Single,
    /// Extra-wide single-image double-page spread. Occupies an entire visual spread on its own.
    Spread,
}

impl PageClassification {
    pub const fn is_spread(self) -> bool {
        matches!(self, PageClassification::Spread)
    }
}

/// Classifies an image as a normal single page or a double-page spread based on dimensions,
/// ratio threshold, and context.
pub fn classify_image_aspect_ratio(
    width: u32,
    height: u32,
    threshold: f32,
    typical_portrait: bool,
) -> PageClassification {
    if width < MIN_SPREAD_DIMENSION || height < MIN_SPREAD_DIMENSION {
        return PageClassification::Single;
    }
    if width <= height {
        // Portrait or square is always a normal single page.
        return PageClassification::Single;
    }
    let ratio = width as f32 / height as f32;
    if ratio < threshold {
        return PageClassification::Single;
    }
    // If the comic is known to be primarily landscape (e.g. comic strip book),
    // wide pages are normal pages, not spreads.
    if !typical_portrait {
        return PageClassification::Single;
    }
    PageClassification::Spread
}

/// The logical pages contained in a spread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpreadPages {
    /// Logical page index read first in this spread.
    pub primary: usize,
    /// Logical page index read second in this spread (if double spread and page exists).
    pub secondary: Option<usize>,
    /// Whether this spread is a single-image spread.
    pub is_single_image_spread: bool,
}

impl SpreadPages {
    pub const fn new(primary: usize, secondary: Option<usize>) -> Self {
        Self {
            primary,
            secondary,
            is_single_image_spread: false,
        }
    }

    pub const fn single_image_spread(primary: usize) -> Self {
        Self {
            primary,
            secondary: None,
            is_single_image_spread: true,
        }
    }

    pub fn contains(&self, page: usize) -> bool {
        self.primary == page || self.secondary == Some(page)
    }

    pub fn to_vec(&self) -> Vec<usize> {
        match self.secondary {
            Some(sec) => vec![self.primary, sec],
            None => vec![self.primary],
        }
    }
}

/// Precomputed mapping of logical pages to visual spreads for a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpreadPlan {
    spreads: Vec<SpreadPages>,
    page_to_spread: Vec<usize>,
}

impl SpreadPlan {
    /// Builds a spread plan for the archive based on layout, cover isolation, and page classifications.
    pub fn new(
        total_pages: usize,
        layout: PageLayout,
        first_page_is_cover: bool,
        mut classify: impl FnMut(usize) -> PageClassification,
    ) -> Self {
        if total_pages == 0 {
            return Self {
                spreads: Vec::new(),
                page_to_spread: Vec::new(),
            };
        }

        let mut spreads = Vec::new();
        let mut page_to_spread = vec![0; total_pages];

        match layout {
            PageLayout::Single => {
                spreads.reserve(total_pages);
                for (p, slot) in page_to_spread.iter_mut().enumerate() {
                    let is_spread = classify(p).is_spread();
                    let s_idx = spreads.len();
                    spreads.push(if is_spread {
                        SpreadPages::single_image_spread(p)
                    } else {
                        SpreadPages::new(p, None)
                    });
                    *slot = s_idx;
                }
            }
            PageLayout::Double => {
                let mut p = 0;
                if first_page_is_cover && total_pages > 0 {
                    let is_spread = classify(0).is_spread();
                    let s_idx = spreads.len();
                    spreads.push(if is_spread {
                        SpreadPages::single_image_spread(0)
                    } else {
                        SpreadPages::new(0, None)
                    });
                    page_to_spread[0] = s_idx;
                    p = 1;
                }

                while p < total_pages {
                    let class_p = classify(p);
                    if class_p.is_spread() {
                        let s_idx = spreads.len();
                        spreads.push(SpreadPages::single_image_spread(p));
                        page_to_spread[p] = s_idx;
                        p += 1;
                    } else {
                        // Single page. Can it pair with p + 1?
                        if p + 1 < total_pages {
                            let class_next = classify(p + 1);
                            if class_next.is_spread() {
                                // Next page is a spread; cannot pair. p remains alone.
                                let s_idx = spreads.len();
                                spreads.push(SpreadPages::new(p, None));
                                page_to_spread[p] = s_idx;
                                p += 1;
                            } else {
                                // Both are single pages. Pair them!
                                let s_idx = spreads.len();
                                spreads.push(SpreadPages::new(p, Some(p + 1)));
                                page_to_spread[p] = s_idx;
                                page_to_spread[p + 1] = s_idx;
                                p += 2;
                            }
                        } else {
                            // Final odd single page.
                            let s_idx = spreads.len();
                            spreads.push(SpreadPages::new(p, None));
                            page_to_spread[p] = s_idx;
                            p += 1;
                        }
                    }
                }
            }
        }

        Self {
            spreads,
            page_to_spread,
        }
    }

    #[inline]
    pub fn total_spreads(&self) -> usize {
        self.spreads.len()
    }

    #[inline]
    pub fn pages_for_spread(&self, spread_index: usize) -> Option<SpreadPages> {
        self.spreads.get(spread_index).copied()
    }

    #[inline]
    pub fn spread_index_for_page(&self, page: usize) -> usize {
        self.page_to_spread.get(page).copied().unwrap_or(0)
    }

    pub fn spreads(&self) -> &[SpreadPages] {
        &self.spreads
    }
}

/// Physical positioning of pages on screen (left vs right).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualSpread {
    pub left_page: Option<usize>,
    pub right_page: Option<usize>,
}

/// Computes the visual placement of pages for a spread based on reading direction.
pub fn visual_spread(spread: SpreadPages, direction: ReadingDirection) -> VisualSpread {
    match spread.secondary {
        None => VisualSpread {
            left_page: Some(spread.primary),
            right_page: None,
        },
        Some(second) => {
            if direction.is_rtl() {
                // In RTL: primary is on the right, secondary on the left
                VisualSpread {
                    left_page: Some(second),
                    right_page: Some(spread.primary),
                }
            } else {
                // In LTR: primary is on the left, secondary on the right
                VisualSpread {
                    left_page: Some(spread.primary),
                    right_page: Some(second),
                }
            }
        }
    }
}

/// Total number of spreads for a book with `total_pages`.
pub fn total_spreads(total_pages: usize, layout: PageLayout, first_page_is_cover: bool) -> usize {
    if total_pages == 0 {
        return 0;
    }
    match layout {
        PageLayout::Single => total_pages,
        PageLayout::Double => {
            if first_page_is_cover {
                if total_pages == 1 {
                    1
                } else {
                    1 + (total_pages - 1).div_ceil(2)
                }
            } else {
                total_pages.div_ceil(2)
            }
        }
    }
}

/// Computes the spread index for a given `page_index`.
pub fn spread_index_for_page(
    page_index: usize,
    total_pages: usize,
    layout: PageLayout,
    first_page_is_cover: bool,
) -> usize {
    if total_pages == 0 || page_index >= total_pages {
        return 0;
    }
    match layout {
        PageLayout::Single => page_index,
        PageLayout::Double => {
            if first_page_is_cover {
                if page_index == 0 {
                    0
                } else {
                    1 + (page_index - 1) / 2
                }
            } else {
                page_index / 2
            }
        }
    }
}

/// Retrieves the logical `SpreadPages` for a given `spread_index`.
pub fn pages_for_spread(
    spread_index: usize,
    total_pages: usize,
    layout: PageLayout,
    first_page_is_cover: bool,
) -> Option<SpreadPages> {
    if total_pages == 0 {
        return None;
    }
    match layout {
        PageLayout::Single => {
            if spread_index < total_pages {
                Some(SpreadPages::new(spread_index, None))
            } else {
                None
            }
        }
        PageLayout::Double => {
            let total_s = total_spreads(total_pages, layout, first_page_is_cover);
            if spread_index >= total_s {
                return None;
            }
            if first_page_is_cover {
                if spread_index == 0 {
                    Some(SpreadPages::new(0, None))
                } else {
                    let p1 = 1 + (spread_index - 1) * 2;
                    let p2 = if p1 + 1 < total_pages {
                        Some(p1 + 1)
                    } else {
                        None
                    };
                    Some(SpreadPages::new(p1, p2))
                }
            } else {
                let p1 = spread_index * 2;
                let p2 = if p1 + 1 < total_pages {
                    Some(p1 + 1)
                } else {
                    None
                };
                Some(SpreadPages::new(p1, p2))
            }
        }
    }
}

/// Whether the viewport is suitable for dual-page presentation.
/// Narrow portrait windows fall back to single page so text remains legible.
pub fn is_dual_spread_effective(vp: &Viewport) -> bool {
    vp.width >= 550.0 && vp.width >= vp.height * 0.75
}

/// Computes display sizes for left and right pages in a dual-page spread.
/// Both pages are scaled to share a common height while preserving their respective aspect ratios.
pub fn spread_display_sizes(
    fit: FitMode,
    zoom: f32,
    left_orig: (f32, f32),
    right_orig: (f32, f32),
    gap: f32,
    vp: &Viewport,
) -> ((f32, f32), (f32, f32)) {
    let (lw, lh) = (left_orig.0.max(1.0), left_orig.1.max(1.0));
    let (rw, rh) = (right_orig.0.max(1.0), right_orig.1.max(1.0));

    let r_left = lw / lh;
    let r_right = rw / rh;
    let r_total = r_left + r_right;

    let avail_w = (vp.width - gap).max(1.0);
    let avail_h = vp.height.max(1.0);

    let base_height = match fit {
        FitMode::Page => (avail_w / r_total).min(avail_h),
        FitMode::Width => avail_w / r_total,
        FitMode::Height => avail_h,
        FitMode::Original => {
            let avg_h = (lh + rh) * 0.5;
            avg_h / vp.scale.max(0.01)
        }
    };

    let height = base_height * zoom;
    let left_w = height * r_left;
    let right_w = height * r_right;

    ((left_w, height), (right_w, height))
}

/// Computes viewport origins for left and right pages in a dual-page spread.
pub fn spread_origins(
    off: (f32, f32),
    left_display: (f32, f32),
    right_display: (f32, f32),
    gap: f32,
    vp: &Viewport,
) -> ((f32, f32), (f32, f32)) {
    let total_w = left_display.0 + gap + right_display.0;
    let total_h = left_display.1.max(right_display.1);

    let spread_x = if total_w <= vp.width {
        (vp.width - total_w) / 2.0
    } else {
        -off.0.clamp(0.0, total_w - vp.width)
    };

    let spread_y = if total_h <= vp.height {
        (vp.height - total_h) / 2.0
    } else {
        -off.1.clamp(0.0, total_h - vp.height)
    };

    let left_pos = (spread_x, spread_y);
    let right_pos = (spread_x + left_display.0 + gap, spread_y);

    (left_pos, right_pos)
}

/// Furthest a dual-page spread can be scrolled on each axis.
pub fn spread_max_offset(
    left_display: (f32, f32),
    right_display: (f32, f32),
    gap: f32,
    vp: &Viewport,
) -> (f32, f32) {
    let total_w = left_display.0 + gap + right_display.0;
    let total_h = left_display.1.max(right_display.1);
    (
        (total_w - vp.width).max(0.0),
        (total_h - vp.height).max(0.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const VP: Viewport = Viewport {
        width: 800.0,
        height: 600.0,
        scale: 1.0,
    };

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn fit_modes_pick_the_right_scale() {
        let orig = (1000.0, 2000.0);
        assert!(close(
            display_size(FitMode::Page, 1.0, orig, &VP),
            (300.0, 600.0)
        ));
        assert!(close(
            display_size(FitMode::Width, 1.0, orig, &VP),
            (800.0, 1600.0)
        ));
        assert!(close(
            display_size(FitMode::Height, 1.0, orig, &VP),
            (300.0, 600.0)
        ));
        let hidpi = Viewport { scale: 2.0, ..VP };
        assert!(close(
            display_size(FitMode::Original, 1.0, orig, &hidpi),
            (500.0, 1000.0)
        ));
    }

    #[test]
    fn wide_page_fits_differently() {
        let orig = (3000.0, 1000.0);
        assert!(close(
            display_size(FitMode::Page, 1.0, orig, &VP),
            (800.0, 266.666)
        ));
        assert!(close(
            display_size(FitMode::Height, 1.0, orig, &VP),
            (1800.0, 600.0)
        ));
    }

    #[test]
    fn zoom_scales_display() {
        let d = display_size(FitMode::Page, 2.0, (1000.0, 2000.0), &VP);
        assert!(close(d, (600.0, 1200.0)));
    }

    #[test]
    fn small_pages_are_centred_large_pages_scroll() {
        assert!(close(origin((0.0, 0.0), (300.0, 600.0), &VP), (250.0, 0.0)));
        assert!(close(
            origin((0.0, 100.0), (800.0, 1600.0), &VP),
            (0.0, -100.0)
        ));
        // an offset beyond the end is clamped
        assert!(close(
            origin((0.0, 5000.0), (800.0, 1600.0), &VP),
            (0.0, -1000.0)
        ));
        assert!(close(max_offset((800.0, 1600.0), &VP), (0.0, 1000.0)));
        assert!(close(
            clamp_offset((50.0, -9.0), (800.0, 1600.0), &VP),
            (0.0, 0.0)
        ));
    }

    #[test]
    fn zoom_keeps_the_anchor_point_fixed() {
        let old_display = (800.0, 1600.0);
        let off = (0.0, 400.0);
        let old_origin = origin(off, old_display, &VP);
        let anchor = (200.0, 150.0);
        let image_point = (
            (anchor.0 - old_origin.0) / old_display.0,
            (anchor.1 - old_origin.1) / old_display.1,
        );

        let new_display = (1600.0, 3200.0);
        let new_off = zoom_anchor_offset(old_origin, old_display, new_display, anchor);
        let new_origin = origin(new_off, new_display, &VP);
        let after = (
            (anchor.0 - new_origin.0) / new_display.0,
            (anchor.1 - new_origin.1) / new_display.1,
        );
        assert!(
            (image_point.1 - after.1).abs() < 1e-4,
            "{image_point:?} vs {after:?}"
        );
    }

    #[test]
    fn arrival_offsets_match_bounds() {
        let max = (500.0, 900.0);
        assert_eq!(arrival_offset(Arrive::Start, max), (0.0, 0.0));
        assert_eq!(arrival_offset(Arrive::End, max), (500.0, 900.0));
    }

    #[test]
    fn tiers_round_up_and_cap() {
        assert_eq!(tier_for(900.0), 1024);
        assert_eq!(tier_for(1025.0), 1536);
        assert_eq!(tier_for(2048.0), 2048);
        assert_eq!(tier_for(99999.0), 6144);
    }

    #[test]
    fn fit_cycle_visits_every_mode() {
        let mut mode = FitMode::Page;
        let mut seen = vec![mode];
        for _ in 0..3 {
            mode = mode.next();
            seen.push(mode);
        }
        assert_eq!(mode.next(), FitMode::Page);
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn test_spread_calculations_with_cover() {
        // 5 pages:
        // Spread 0: [0]
        // Spread 1: [1, 2]
        // Spread 2: [3, 4]
        assert_eq!(total_spreads(5, PageLayout::Double, true), 3);
        assert_eq!(spread_index_for_page(0, 5, PageLayout::Double, true), 0);
        assert_eq!(spread_index_for_page(1, 5, PageLayout::Double, true), 1);
        assert_eq!(spread_index_for_page(2, 5, PageLayout::Double, true), 1);
        assert_eq!(spread_index_for_page(3, 5, PageLayout::Double, true), 2);
        assert_eq!(spread_index_for_page(4, 5, PageLayout::Double, true), 2);

        assert_eq!(
            pages_for_spread(0, 5, PageLayout::Double, true),
            Some(SpreadPages::new(0, None))
        );
        assert_eq!(
            pages_for_spread(1, 5, PageLayout::Double, true),
            Some(SpreadPages::new(1, Some(2)))
        );
        assert_eq!(
            pages_for_spread(2, 5, PageLayout::Double, true),
            Some(SpreadPages::new(3, Some(4)))
        );
        assert_eq!(pages_for_spread(3, 5, PageLayout::Double, true), None);

        // 6 pages:
        // Spread 0: [0]
        // Spread 1: [1, 2]
        // Spread 2: [3, 4]
        // Spread 3: [5]
        assert_eq!(total_spreads(6, PageLayout::Double, true), 4);
        assert_eq!(spread_index_for_page(5, 6, PageLayout::Double, true), 3);
        assert_eq!(
            pages_for_spread(3, 6, PageLayout::Double, true),
            Some(SpreadPages::new(5, None))
        );
    }

    #[test]
    fn test_spread_calculations_without_cover() {
        // 5 pages:
        // Spread 0: [0, 1]
        // Spread 1: [2, 3]
        // Spread 2: [4]
        assert_eq!(total_spreads(5, PageLayout::Double, false), 3);
        assert_eq!(spread_index_for_page(0, 5, PageLayout::Double, false), 0);
        assert_eq!(spread_index_for_page(1, 5, PageLayout::Double, false), 0);
        assert_eq!(spread_index_for_page(2, 5, PageLayout::Double, false), 1);
        assert_eq!(spread_index_for_page(3, 5, PageLayout::Double, false), 1);
        assert_eq!(spread_index_for_page(4, 5, PageLayout::Double, false), 2);

        assert_eq!(
            pages_for_spread(0, 5, PageLayout::Double, false),
            Some(SpreadPages::new(0, Some(1)))
        );
        assert_eq!(
            pages_for_spread(1, 5, PageLayout::Double, false),
            Some(SpreadPages::new(2, Some(3)))
        );
        assert_eq!(
            pages_for_spread(2, 5, PageLayout::Double, false),
            Some(SpreadPages::new(4, None))
        );
    }

    #[test]
    fn test_visual_spread_directions() {
        let spread_two = SpreadPages::new(1, Some(2));
        let rtl = visual_spread(spread_two, ReadingDirection::RightToLeft);
        assert_eq!(rtl.left_page, Some(2));
        assert_eq!(rtl.right_page, Some(1));

        let ltr = visual_spread(spread_two, ReadingDirection::LeftToRight);
        assert_eq!(ltr.left_page, Some(1));
        assert_eq!(ltr.right_page, Some(2));

        let spread_one = SpreadPages::new(0, None);
        let rtl_single = visual_spread(spread_one, ReadingDirection::RightToLeft);
        assert_eq!(rtl_single.left_page, Some(0));
        assert_eq!(rtl_single.right_page, None);

        let ltr_single = visual_spread(spread_one, ReadingDirection::LeftToRight);
        assert_eq!(ltr_single.left_page, Some(0));
        assert_eq!(ltr_single.right_page, None);
    }

    #[test]
    fn test_is_dual_spread_effective() {
        let wide_vp = Viewport {
            width: 1200.0,
            height: 800.0,
            scale: 1.0,
        };
        assert!(is_dual_spread_effective(&wide_vp));

        let narrow_vp = Viewport {
            width: 480.0,
            height: 900.0,
            scale: 1.0,
        };
        assert!(!is_dual_spread_effective(&narrow_vp));

        let square_vp = Viewport {
            width: 600.0,
            height: 600.0,
            scale: 1.0,
        };
        assert!(is_dual_spread_effective(&square_vp));
    }

    #[test]
    fn test_spread_display_sizes_and_origins() {
        let vp = Viewport {
            width: 1600.0,
            height: 1000.0,
            scale: 1.0,
        };
        let (left_size, right_size) = spread_display_sizes(
            FitMode::Height,
            1.0,
            (600.0, 1000.0),
            (600.0, 1000.0),
            SPREAD_GAP,
            &vp,
        );
        assert_eq!(left_size.1, 1000.0);
        assert_eq!(right_size.1, 1000.0);
        assert_eq!(left_size.0, 600.0);
        assert_eq!(right_size.0, 600.0);

        let (left_orig, right_orig) =
            spread_origins((0.0, 0.0), left_size, right_size, SPREAD_GAP, &vp);
        // Total width = 600 + 2 + 600 = 1202. Centered in 1600: (1600 - 1202)/2 = 199.0
        assert_eq!(left_orig.0, 199.0);
        assert_eq!(right_orig.0, 199.0 + 600.0 + SPREAD_GAP);
        assert_eq!(left_orig.1, 0.0);
        assert_eq!(right_orig.1, 0.0);

        let max_off = spread_max_offset(left_size, right_size, SPREAD_GAP, &vp);
        assert_eq!(max_off, (0.0, 0.0));
    }

    #[test]
    fn test_classify_image_aspect_ratio() {
        // Normal portrait image
        assert_eq!(
            classify_image_aspect_ratio(1000, 1500, 1.25, true),
            PageClassification::Single
        );
        // Square image
        assert_eq!(
            classify_image_aspect_ratio(1000, 1000, 1.25, true),
            PageClassification::Single
        );
        // Ordinary landscape artwork below threshold (e.g. 1100x1000 = 1.10)
        assert_eq!(
            classify_image_aspect_ratio(1100, 1000, 1.25, true),
            PageClassification::Single
        );
        // Borderline ratios around 1.25
        assert_eq!(
            classify_image_aspect_ratio(1240, 1000, 1.25, true),
            PageClassification::Single
        );
        assert_eq!(
            classify_image_aspect_ratio(1260, 1000, 1.25, true),
            PageClassification::Spread
        );
        // Typical extra-wide double spread (2000x1500 = 1.33)
        assert_eq!(
            classify_image_aspect_ratio(2000, 1500, 1.25, true),
            PageClassification::Spread
        );
        // Missing or tiny dimensions (< MIN_SPREAD_DIMENSION)
        assert_eq!(
            classify_image_aspect_ratio(0, 0, 1.25, true),
            PageClassification::Single
        );
        assert_eq!(
            classify_image_aspect_ratio(150, 100, 1.25, true),
            PageClassification::Single
        );
        // Comic strip / landscape comic context (typical_portrait is false)
        assert_eq!(
            classify_image_aspect_ratio(2000, 1500, 1.25, false),
            PageClassification::Single
        );
    }

    #[test]
    fn test_mixed_source_spread_plan() {
        // 0: Normal, 1: Normal, 2: Spread, 3: Normal, 4: Normal
        let pages = [
            PageClassification::Single,
            PageClassification::Single,
            PageClassification::Spread,
            PageClassification::Single,
            PageClassification::Single,
        ];
        let plan = SpreadPlan::new(pages.len(), PageLayout::Double, true, |i| pages[i]);
        // Cover: [0], Normal: [1] (cannot pair with 2!), Spread: [2], Paired: [3, 4]
        assert_eq!(plan.total_spreads(), 4);
        assert_eq!(plan.pages_for_spread(0), Some(SpreadPages::new(0, None)));
        assert_eq!(plan.pages_for_spread(1), Some(SpreadPages::new(1, None)));
        assert_eq!(
            plan.pages_for_spread(2),
            Some(SpreadPages::single_image_spread(2))
        );
        assert_eq!(plan.pages_for_spread(3), Some(SpreadPages::new(3, Some(4))));

        assert_eq!(plan.spread_index_for_page(0), 0);
        assert_eq!(plan.spread_index_for_page(1), 1);
        assert_eq!(plan.spread_index_for_page(2), 2);
        assert_eq!(plan.spread_index_for_page(3), 3);
        assert_eq!(plan.spread_index_for_page(4), 3);
    }

    #[test]
    fn test_consecutive_single_image_spreads() {
        let pages = [
            PageClassification::Single,
            PageClassification::Spread,
            PageClassification::Spread,
            PageClassification::Single,
        ];
        let plan = SpreadPlan::new(pages.len(), PageLayout::Double, false, |i| pages[i]);
        assert_eq!(plan.total_spreads(), 4);
        assert_eq!(plan.pages_for_spread(0), Some(SpreadPages::new(0, None)));
        assert_eq!(
            plan.pages_for_spread(1),
            Some(SpreadPages::single_image_spread(1))
        );
        assert_eq!(
            plan.pages_for_spread(2),
            Some(SpreadPages::single_image_spread(2))
        );
        assert_eq!(plan.pages_for_spread(3), Some(SpreadPages::new(3, None)));
    }

    #[test]
    fn test_alternating_spreads_and_final_incomplete() {
        let pages = [
            PageClassification::Single,
            PageClassification::Spread,
            PageClassification::Single,
        ];
        let plan = SpreadPlan::new(pages.len(), PageLayout::Double, false, |i| pages[i]);
        // 0 cannot pair with 1, 1 is spread, 2 is odd final
        assert_eq!(plan.total_spreads(), 3);
        assert_eq!(plan.pages_for_spread(0), Some(SpreadPages::new(0, None)));
        assert_eq!(
            plan.pages_for_spread(1),
            Some(SpreadPages::single_image_spread(1))
        );
        assert_eq!(plan.pages_for_spread(2), Some(SpreadPages::new(2, None)));
    }
}
