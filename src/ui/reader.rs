//! Reader state, visual presentation, continuous vertical layout, page transitions, and inertial scroll management.

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Instant,
};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::{ScrollableElement, ScrollbarHandle};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Side as MenuSide, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, ClickEvent, Div, FontWeight, Hsla, InteractiveElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, PinchEvent, Pixels, Point,
    RenderImage, ScrollEvent, SharedString, Size, Stateful, Styled, TouchPhase, Window, div, img,
    point, px, relative, size,
};
use yomu::{
    archive::SourceKind,
    layout::{self, Arrive, FitMode, Viewport},
    library::{Book, Settings},
    motion::{self, KineticScroll, PageTransition},
    vim::Vim,
};

use super::components::{centered, pill, with_alpha};
use super::theme::UiColors;

pub fn viewport_of(window: &Window) -> Viewport {
    let size = window.viewport_size();
    Viewport {
        width: f32::from(size.width),
        height: f32::from(size.height),
        scale: window.scale_factor(),
    }
}

/// A decoded page, drawn at a size we compute ourselves (see `layout`).
pub struct Page {
    pub image: Arc<RenderImage>,
    /// Dimensions of the original image, which is what fit modes are computed from.
    pub orig: (u32, u32),
    /// Decode tier (longest side in pixels) this image was made at.
    pub tier: u32,
    /// Memory the decoded pixels take (BGRA), for the cache budget.
    pub bytes: u64,
}

/// The page that is sliding out during a page turn, frozen as it was when the turn began.
pub struct Outgoing {
    pub image: Arc<RenderImage>,
    pub display: (f32, f32),
    pub origin: (f32, f32),
}

#[derive(Clone, Copy)]
pub struct Drag {
    pub start: (f32, f32),
    pub off0: (f32, f32),
    pub moved: bool,
    pub last_at: (f32, f32),
}

#[derive(Clone, Copy, Debug)]
pub struct WheelParams {
    pub delta: (f32, f32),
    pub zoom: bool,
    pub at: (f32, f32),
    pub discrete: bool,
    pub touch_phase: TouchPhase,
    pub vp: Viewport,
}

impl WheelParams {
    pub const fn new(
        delta: (f32, f32),
        zoom: bool,
        at: (f32, f32),
        discrete: bool,
        touch_phase: TouchPhase,
        vp: Viewport,
    ) -> Self {
        Self {
            delta,
            zoom,
            at,
            discrete,
            touch_phase,
            vp,
        }
    }
}

pub struct ReaderSyncOutcome {
    pub wanted: Vec<(usize, u32)>,
    pub animating: bool,
    pub progress_changed: Option<(String, usize)>,
}

/// Scrollbar handle synchronizing GPUI vertical scrollbar dragging with continuous reader scroll offset.
#[derive(Clone)]
pub struct ReaderScrollHandle {
    pub bounds: Bounds<Pixels>,
    pub offset_y: f32,
    pub content_height: f32,
    pub sync: Rc<Cell<Option<f32>>>,
    pub max_offset: f32,
}

impl ScrollbarHandle for ReaderScrollHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    fn offset(&self) -> Point<Pixels> {
        point(px(0.), px(-self.offset_y))
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let target_y = (-f32::from(offset.y)).clamp(0.0, self.max_offset);
        self.sync.set(Some(target_y));
    }

    fn content_size(&self) -> Size<Pixels> {
        size(
            self.bounds.size.width,
            px(self.content_height.max(f32::from(self.bounds.size.height))),
        )
    }
}

/// One page image at an exact position and size.
pub fn page_layer(
    image: &Arc<RenderImage>,
    left: f32,
    top: f32,
    display: (f32, f32),
) -> AnyElement {
    img(image.clone())
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(display.0))
        .h(px(display.1))
        .object_fit(ObjectFit::Fill)
        .into_any_element()
}

/// Horizontal reading progress bar along the bottom of the viewport.
pub fn reader_progress_bar(fraction: f32, accent: Hsla) -> Div {
    div()
        .absolute()
        .bottom_0()
        .left_0()
        .h(px(2.))
        .w(relative(fraction))
        .bg(accent)
}

/// Autohiding top navigation and header overlay for the reader canvas.
pub fn reader_header(
    book_title: &str,
    current_page_str: &str,
    fit_mode: FitMode,
    colors: UiColors,
    on_back: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_set_fit: impl Fn(FitMode, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let on_set_fit = Rc::new(on_set_fit);
    div()
        .id("reader-top-hover-area")
        .group("reader-top")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(56.))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .group_hover("reader-top", |s| s.opacity(1.0))
                .opacity(0.0)
                .w_full()
                .h(px(52.))
                .px_4()
                .bg(with_alpha(colors.background, 0.92))
                .border_b_1()
                .border_color(with_alpha(colors.border, 0.5))
                .shadow_lg()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            Button::new("reader-back-btn")
                                .ghost()
                                .small()
                                .label("← Library")
                                .tooltip("Back to library (Esc)")
                                .rounded(ButtonRounded::Medium)
                                .on_click(move |event, window, cx| {
                                    on_back(event, window, cx);
                                }),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(colors.foreground)
                                .child(book_title.to_string()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.muted)
                        .child(current_page_str.to_string()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("reader-fit-toggle")
                                .ghost()
                                .small()
                                .dropdown_caret(true)
                                .rounded(ButtonRounded::Medium)
                                .tooltip("Display fit mode")
                                .label(fit_mode.label())
                                .dropdown_menu({
                                    let on_set_fit = on_set_fit.clone();
                                    move |mut menu, _, _| {
                                        menu = menu.check_side(MenuSide::Left).min_w(px(140.));
                                        let options = [
                                            FitMode::Page,
                                            FitMode::Width,
                                            FitMode::Height,
                                            FitMode::Original,
                                        ];
                                        for &mode in &options {
                                            let on_set_fit = on_set_fit.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(mode.label())
                                                    .checked(mode == fit_mode)
                                                    .on_click(move |_, window, cx| {
                                                        on_set_fit(mode, window, cx);
                                                    }),
                                            );
                                        }
                                        menu
                                    }
                                }),
                        )
                        .child(
                            Button::new("reader-fullscreen-toggle")
                                .ghost()
                                .small()
                                .rounded(ButtonRounded::Medium)
                                .tooltip("Toggle fullscreen (F11)")
                                .icon(IconName::Maximize)
                                .on_click(move |_, window, _| window.toggle_fullscreen()),
                        ),
                ),
        )
}

/// Floating lateral chevrons for navigating previous and next pages.
pub fn lateral_turners(
    colors: UiColors,
    radius: Pixels,
    on_prev: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_next: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> (Stateful<Div>, Stateful<Div>) {
    let left_chevron = div()
        .id("reader-prev-hover-area")
        .group("reader-prev")
        .absolute()
        .left_0()
        .top(relative(0.35))
        .bottom(relative(0.35))
        .w(px(72.))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .flex()
        .items_center()
        .pl_4()
        .child(
            div()
                .id("reader-prev-chevron")
                .group_hover("reader-prev", |s| s.opacity(1.0))
                .opacity(0.0)
                .size(px(40.))
                .rounded(radius)
                .bg(with_alpha(colors.background, 0.85))
                .border_1()
                .border_color(with_alpha(colors.border, 0.5))
                .shadow_md()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(with_alpha(colors.accent, 0.2)))
                .tooltip(|window, cx| Tooltip::new("Previous page (Left or K)").build(window, cx))
                .on_click(move |event, window, cx| {
                    on_prev(event, window, cx);
                })
                .child(
                    Icon::new(IconName::ChevronLeft)
                        .size(px(20.))
                        .text_color(colors.foreground),
                ),
        );

    let right_chevron = div()
        .id("reader-next-hover-area")
        .group("reader-next")
        .absolute()
        .right_0()
        .top(relative(0.35))
        .bottom(relative(0.35))
        .w(px(72.))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .flex()
        .items_center()
        .justify_end()
        .pr_4()
        .child(
            div()
                .id("reader-next-chevron")
                .group_hover("reader-next", |s| s.opacity(1.0))
                .opacity(0.0)
                .size(px(40.))
                .rounded(radius)
                .bg(with_alpha(colors.background, 0.85))
                .border_1()
                .border_color(with_alpha(colors.border, 0.5))
                .shadow_md()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(with_alpha(colors.accent, 0.2)))
                .tooltip(|window, cx| Tooltip::new("Next page (Right or J)").build(window, cx))
                .on_click(move |event, window, cx| {
                    on_next(event, window, cx);
                })
                .child(
                    Icon::new(IconName::ChevronRight)
                        .size(px(20.))
                        .text_color(colors.foreground),
                ),
        );

    (left_chevron, right_chevron)
}

/// Configuration properties for reader presentation.
pub struct ReaderProps<'a> {
    pub reader: &'a Reader,
    pub colors: UiColors,
    pub radius: Pixels,
    pub border_px: f32,
    pub radius_xl_px: f32,
    pub book_title: &'a str,
    pub hud: Option<SharedString>,
    pub scroll_handle: Option<&'a ReaderScrollHandle>,
}

/// Renders the complete reader view: pages canvas, progress indicator, HUD pills,
/// lateral turn chevrons, autohiding header, shortcuts help overlay, and continuous scrollbar.
#[allow(clippy::too_many_arguments)]
pub fn render_reader(
    props: ReaderProps<'_>,
    on_back: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_set_fit: impl Fn(FitMode, &mut Window, &mut App) + 'static,
    on_turn_prev: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_turn_next: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_mouse_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    on_mouse_move: impl Fn(&MouseMoveEvent, &mut Window, &mut App) + 'static,
    on_mouse_up: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    on_scroll: impl Fn(&ScrollEvent, &mut Window, &mut App) + 'static,
    on_pinch: impl Fn(&PinchEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let reader = props.reader;
    let vp = reader.viewport;

    // Pages are drawn at a size and position we compute ourselves, so what you see is exactly
    // what the fit mode and zoom say, independent of the texture's pixel size. During a page
    // turn the outgoing and incoming pages are both drawn, shifted along the same strip.
    let mut content: Vec<AnyElement> = Vec::new();
    if let Some(error) = &reader.error {
        content.push(centered(
            format!("Couldn't open this book: {error}"),
            props.colors,
        ));
    } else if reader.continuous {
        let width = vp.width * reader.zoom;
        let page_w = width.max(1.0);
        let gap = if reader.webcomics {
            0.0
        } else {
            12.0 * reader.zoom
        };
        let left = (vp.width - width) / 2.0 - reader.off.0;
        let visible_top = reader.off.1;
        let visible_bottom = visible_top + vp.height;
        let mut top = 0.0;
        for ix in 0..reader.total {
            let (w, h) = reader.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
            let height = page_w * h as f32 / w.max(1) as f32;
            let overlap = if reader.webcomics && ix + 1 < reader.total {
                1.0
            } else {
                0.0
            };
            if top > visible_bottom {
                break;
            }
            if top + height + overlap >= visible_top {
                if let Some(page) = reader.images.get(&ix) {
                    content.push(page_layer(
                        &page.image,
                        left,
                        top - reader.off.1,
                        (width, height + overlap),
                    ));
                } else if let Some(error) = reader.failed.get(&ix) {
                    content.push(centered(
                        format!("Couldn't decode page {}: {error}", ix + 1),
                        props.colors,
                    ));
                }
            }
            top += height + gap;
        }
    } else {
        let (shift_out, shift_in) = match &reader.transition {
            Some(transition) => (
                transition.outgoing_shift() * vp.width,
                transition.incoming_shift() * vp.width,
            ),
            None => (0.0, 0.0),
        };
        if let Some(outgoing) = &reader.outgoing {
            content.push(page_layer(
                &outgoing.image,
                outgoing.origin.0 + shift_out,
                outgoing.origin.1,
                outgoing.display,
            ));
        }
        if let Some(page) = reader.current() {
            let display = layout::display_size(reader.fit, reader.zoom, Reader::orig_of(page), &vp);
            let (left, top) = layout::origin(reader.off, display, &vp);
            content.push(page_layer(&page.image, left + shift_in, top, display));
        } else if reader.outgoing.is_none() {
            if let Some(error) = reader.failed.get(&reader.page) {
                content.push(centered(
                    format!("Couldn't decode page {}: {error}", reader.page + 1),
                    props.colors,
                ));
            } else if reader.pages.is_none() {
                content.push(centered("Opening…", props.colors));
            }
            // otherwise: blank for the moment it takes the first page to decode
        }
    }

    let fraction = (reader.page + 1) as f32 / reader.total.max(1) as f32;
    let progress_line = reader_progress_bar(fraction, props.colors.accent);

    let info = reader.show_info.then(|| {
        pill(
            div().absolute().bottom_4().left_0().right_0(),
            format!(
                "{} / {}  ·  {}  ·  {}%  ·  {}",
                reader.page + 1,
                reader.total,
                reader.fit.label(),
                (reader.zoom * 100.0).round() as i32,
                if reader.webcomics {
                    "webcomics"
                } else if reader.continuous {
                    "continuous"
                } else {
                    "single page"
                }
            ),
            props.colors,
            props.radius,
        )
    });
    let hud = props.hud.map(|text| {
        pill(
            div().absolute().top_4().left_0().right_0(),
            text,
            props.colors,
            props.radius,
        )
    });

    let vim_status = if reader.vim.in_cmdline() {
        None
    } else {
        reader.vim.status().map(|text| {
            pill(
                div().absolute().bottom_4().left_0().right_0(),
                text,
                props.colors,
                props.radius,
            )
            .into_any_element()
        })
    };

    let current_page_str = format!("{} / {}", reader.page + 1, reader.total);
    let top_bar = reader_header(
        props.book_title,
        &current_page_str,
        reader.fit,
        props.colors,
        on_back,
        on_set_fit,
    );

    let (left_chevron, right_chevron) =
        lateral_turners(props.colors, props.radius, on_turn_prev, on_turn_next);

    div()
        .size_full()
        .relative()
        .overflow_hidden()
        .bg(props.colors.background)
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            on_mouse_down(event, window, cx);
        })
        .on_mouse_move(move |event, window, cx| {
            on_mouse_move(event, window, cx);
        })
        .on_mouse_up(MouseButton::Left, move |event, window, cx| {
            on_mouse_up(event, window, cx);
        })
        .on_scroll(move |event, window, cx| {
            on_scroll(event, window, cx);
        })
        .on_pinch(move |event, window, cx| {
            on_pinch(event, window, cx);
        })
        .children(content)
        .child(progress_line)
        .children(info)
        .children(hud)
        .children(vim_status)
        .child(top_bar)
        .child(left_chevron)
        .child(right_chevron)
        .children(reader.show_help.then(|| {
            super::overlays::help_overlay(props.colors, props.border_px, props.radius_xl_px)
        }))
        .when_some(props.scroll_handle, |this, handle| {
            this.vertical_scrollbar(handle)
        })
        .into_any_element()
}

/// State of the book that is currently open.
pub struct Reader {
    pub book_id: String,
    pub path: PathBuf,
    pub kind: SourceKind,
    /// `None` while the page list is still being read from disk.
    pub pages: Option<Arc<Vec<String>>>,
    /// Original dimensions are retained separately from decoded textures so continuous layout
    /// can keep stable geometry while pages are evicted from the image cache.
    pub orig_sizes: Vec<Option<(u32, u32)>>,
    pub page: usize,
    pub total: usize,

    pub fit: FitMode,
    /// Multiplier on top of the fit size; `1.0` is exactly the fit.
    pub zoom: f32,
    /// How far the page is scrolled from its top-left corner (what is drawn this frame).
    pub off: (f32, f32),
    /// Where `off` is gliding to. Equal to `off` when nothing is moving. Key and wheel-notch
    /// scrolling move this; edge checks and repeated presses accumulate on it, not on `off`.
    pub scroll_target: (f32, f32),
    /// Scroll position to apply once the new page's size is known.
    pub arrive: Option<Arrive>,
    pub viewport: Viewport,

    /// The slide between pages, if one is running. At most one exists (no queue); a page turn
    /// that arrives mid-slide replaces it. Purely visual: `page` and the saved progress already
    /// point at the new page the moment you turn.
    pub transition: Option<PageTransition>,
    pub outgoing: Option<Outgoing>,
    /// The viewport the slide started in; resizing the window cancels it.
    pub transition_viewport: Viewport,
    /// Whether page turns and scrolling animate (the setting, unless the desktop asks for
    /// reduced motion).
    pub animate: bool,
    pub continuous: bool,
    pub webcomics: bool,
    /// Textures no longer shown, to be freed by the window on its next frame.
    pub released: Vec<Arc<RenderImage>>,

    pub images: HashMap<usize, Page>,
    pub pending: HashSet<usize>,
    pub cancellations: HashMap<usize, Arc<std::sync::atomic::AtomicBool>>,
    pub failed: HashMap<usize, String>,
    pub error: Option<String>,

    pub vim: Vim,
    pub drag: Option<Drag>,
    pub kinetic: KineticScroll,
    /// Where `''` jumps back to.
    pub back_page: Option<usize>,
    pub show_info: bool,
    pub show_help: bool,
}

impl Reader {
    pub fn new(book: &Book, settings: &Settings) -> Self {
        let zoom = if settings.remember_zoom {
            layout::clamp_zoom(settings.zoom)
        } else {
            1.0
        };
        Self {
            book_id: book.id.clone(),
            path: book.path.clone(),
            kind: book.kind,
            pages: None,
            orig_sizes: Vec::new(),
            page: book.current_page.min(book.page_count.saturating_sub(1)),
            total: book.page_count,
            fit: settings.fit,
            zoom,
            off: (0.0, 0.0),
            scroll_target: (0.0, 0.0),
            arrive: Some(Arrive::Start),
            viewport: Viewport::EMPTY,
            transition: None,
            outgoing: None,
            transition_viewport: Viewport::EMPTY,
            animate: settings.animate,
            continuous: settings.continuous || settings.webcomics,
            webcomics: settings.webcomics,
            released: Vec::new(),
            images: HashMap::new(),
            pending: HashSet::new(),
            cancellations: HashMap::new(),
            failed: HashMap::new(),
            error: None,
            vim: Vim::new(),
            drag: None,
            kinetic: KineticScroll::new(),
            back_page: None,
            show_info: false,
            show_help: false,
        }
    }

    pub fn current(&self) -> Option<&Page> {
        self.images.get(&self.page)
    }

    #[cfg(test)]
    pub fn continuous_layout(&self, vp: &Viewport) -> (Vec<f32>, Vec<f32>) {
        let width = (vp.width * self.zoom).max(1.0);
        let gap = if self.webcomics {
            0.0
        } else {
            12.0 * self.zoom
        };
        let mut tops = Vec::with_capacity(self.total);
        let mut heights = Vec::with_capacity(self.total);
        let mut top = 0.0;
        for ix in 0..self.total {
            tops.push(top);
            // Unknown pages use a typical portrait ratio until their metadata is read.
            let (w, h) = self.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
            let height = width * h as f32 / w.max(1) as f32;
            heights.push(height);
            top += height + gap;
        }
        (tops, heights)
    }

    pub fn continuous_total_height(&self, vp: &Viewport) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        let width = (vp.width * self.zoom).max(1.0);
        let gap = if self.webcomics {
            0.0
        } else {
            12.0 * self.zoom
        };
        let mut total = 0.0;
        for ix in 0..self.total {
            let (w, h) = self.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
            let height = width * h as f32 / w.max(1) as f32;
            total += height;
            if ix + 1 < self.total {
                total += gap;
            }
        }
        total
    }

    pub fn continuous_page_at(&self, y: f32, vp: &Viewport) -> usize {
        if self.total == 0 {
            return 0;
        }
        let width = (vp.width * self.zoom).max(1.0);
        let gap = if self.webcomics {
            0.0
        } else {
            12.0 * self.zoom
        };
        let mut top = 0.0;
        for ix in 0..self.total {
            let (w, h) = self.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
            let height = width * h as f32 / w.max(1) as f32;
            if y < top + height {
                return ix;
            }
            top += height + gap;
        }
        self.total.saturating_sub(1)
    }

    pub fn continuous_visible_range(&self, low: f32, high: f32, vp: &Viewport) -> (usize, usize) {
        if self.total == 0 {
            return (0, 0);
        }
        let width = (vp.width * self.zoom).max(1.0);
        let gap = if self.webcomics {
            0.0
        } else {
            12.0 * self.zoom
        };
        let mut top = 0.0;
        let mut first = None;
        let mut last = None;
        for ix in 0..self.total {
            let (w, h) = self.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
            let height = width * h as f32 / w.max(1) as f32;
            if top < high && top + height > low {
                if first.is_none() {
                    first = Some(ix);
                }
                last = Some(ix);
            } else if top >= high && first.is_some() {
                break;
            }
            top += height + gap;
        }
        (first.unwrap_or(self.page), last.unwrap_or(self.page))
    }

    pub fn orig_of(page: &Page) -> (f32, f32) {
        (page.orig.0 as f32, page.orig.1 as f32)
    }

    /// Apply a pending page-turn arrival and clamp the scroll offset. Returns the display size
    /// of the current page, or `None` while it is still being decoded.
    pub fn settle(&mut self, vp: &Viewport) -> Option<(f32, f32)> {
        if self.continuous {
            if self.total == 0 {
                return None;
            }
            let width = vp.width * self.zoom;
            let page_w = width.max(1.0);
            let gap = if self.webcomics {
                0.0
            } else {
                12.0 * self.zoom
            };
            let page = self.page.min(self.total - 1);
            if let Some(arrive) = self.arrive.take() {
                let mut top = 0.0;
                let mut page_h = 0.0;
                for ix in 0..=page {
                    let (w, h) = self.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
                    let h = page_w * h as f32 / w.max(1) as f32;
                    if ix == page {
                        page_h = h;
                        break;
                    }
                    top += h + gap;
                }
                self.scroll_target.1 = match arrive {
                    Arrive::Start => top,
                    Arrive::End => (top + page_h - vp.height).max(0.0),
                };
                self.off = self.scroll_target;
            }
            let doc_height = self.continuous_total_height(vp);
            let max_x = (width - vp.width).max(0.0);
            let max_y = (doc_height - vp.height).max(0.0);
            self.off.0 = self.off.0.clamp(0.0, max_x);
            self.off.1 = self.off.1.clamp(0.0, max_y);
            self.scroll_target.0 = self.scroll_target.0.clamp(0.0, max_x);
            self.scroll_target.1 = self.scroll_target.1.clamp(0.0, max_y);
            let mid_y = self.off.1 + vp.height * 0.5;
            self.page = self.continuous_page_at(mid_y, vp);
            let (w, h) = self
                .orig_sizes
                .get(self.page)
                .and_then(|v| *v)
                .unwrap_or((2, 3));
            let current_h = page_w * h as f32 / w.max(1) as f32;
            return Some((width, current_h));
        }
        let orig = Self::orig_of(self.current()?);
        let display = layout::display_size(self.fit, self.zoom, orig, vp);
        let max = layout::max_offset(display, vp);
        if let Some(arrive) = self.arrive.take() {
            self.set_scroll(layout::arrival_offset(arrive, max));
        }
        self.off = layout::clamp_offset(self.off, display, vp);
        self.scroll_target = layout::clamp_offset(self.scroll_target, display, vp);
        Some(display)
    }

    /// Jump the scroll position (no glide).
    pub fn set_scroll(&mut self, off: (f32, f32)) {
        self.off = off;
        self.scroll_target = off;
    }

    /// Hand a texture that is no longer drawn to the window to free, unless the page cache
    /// still holds it.
    pub fn release(&mut self, image: Arc<RenderImage>) {
        if !self
            .images
            .values()
            .any(|page| Arc::ptr_eq(&page.image, &image))
        {
            self.released.push(image);
        }
    }

    pub fn cancel_transition(&mut self) {
        self.transition = None;
        if let Some(outgoing) = self.outgoing.take() {
            self.release(outgoing.image);
        }
    }

    /// Start the slide for a page turn. Call *before* `page` changes: the page on screen right
    /// now is what slides out.
    ///
    /// No slide is started (and any running one is dropped) when animations are off, when the
    /// page on screen is not decoded yet, or when the window has no size. If a slide is already
    /// running it is replaced, never queued: the page that was sliding in takes over as the
    /// outgoing page from exactly where it is, so rapid turns stay smooth and cannot fall behind.
    pub fn begin_transition(&mut self, forward: bool, enabled: bool) {
        let vp = self.viewport;
        let on_screen = self
            .current()
            .map(|page| (page.image.clone(), Self::orig_of(page)));
        let Some((image, orig)) =
            on_screen.filter(|_| enabled && vp.width >= 1.0 && vp.height >= 1.0)
        else {
            self.cancel_transition();
            return;
        };

        let travel = motion::travel(forward);
        let display = layout::display_size(self.fit, self.zoom, orig, &vp);
        let origin = layout::origin(self.off, display, &vp);
        let next = match self.transition {
            Some(running) => running.interrupted_by(travel),
            None => PageTransition::starting(travel),
        };
        if let Some(previous) = self.outgoing.take() {
            self.release(previous.image);
        }
        self.outgoing = Some(Outgoing {
            image,
            display,
            origin,
        });
        self.transition = Some(next);
        self.transition_viewport = vp;
    }

    /// Advance the slide and the scroll glide by `dt` seconds of real time. Returns whether
    /// another frame is needed; when it returns `false` nothing is moving and the window can
    /// stay idle.
    pub fn advance_animations(&mut self, dt: f32, vp: &Viewport) -> bool {
        let dt = motion::clamp_frame_dt(dt);

        let resized = vp.width != self.transition_viewport.width
            || vp.height != self.transition_viewport.height;
        if self.transition.is_some() && (resized || self.failed.contains_key(&self.page)) {
            self.cancel_transition();
        }
        if !self.animate {
            self.cancel_transition();
            self.kinetic.cancel_inertia();
            self.off = self.scroll_target;
            return false;
        }

        let mut animating = false;
        if let Some(mut transition) = self.transition {
            // The clock only runs once the new page can be shown, so a slow decode keeps the old
            // page on screen instead of showing a blank.
            if self.current().is_some() {
                transition.advance(dt);
                if transition.is_done() {
                    self.transition = None;
                    if let Some(outgoing) = self.outgoing.take() {
                        self.release(outgoing.image);
                    }
                } else {
                    self.transition = Some(transition);
                    animating = true;
                }
            }
        }

        let mut motion_active = false;
        // On macOS, native momentum events are dispatched directly by the OS window server.
        // Software stillness timeout and release tracking are reserved for Linux and Windows.
        if !cfg!(target_os = "macos")
            && self
                .kinetic
                .check_stillness_timeout(Instant::now(), self.animate)
        {
            // Started inertia from stillness
        } else if !cfg!(target_os = "macos") && self.kinetic.is_awaiting_release() {
            motion_active = true;
        }

        let mut moving = false;
        if let Some((dx, dy)) = self.kinetic.step(dt) {
            let (max_w, max_h) = if self.continuous {
                let doc_height = self.continuous_total_height(vp);
                let w = (self.viewport.width * self.zoom - vp.width).max(0.0);
                let h = (doc_height - vp.height).max(0.0);
                (w, h)
            } else if let Some(display) = self.settle(vp) {
                layout::max_offset(display, vp)
            } else {
                (0.0, 0.0)
            };

            let next_x = (self.off.0 + dx).clamp(0.0, max_w);
            let next_y = (self.off.1 + dy).clamp(0.0, max_h);

            self.kinetic.clamp_velocity(
                next_x == 0.0 || next_x == max_w,
                next_y == 0.0 || next_y == max_h,
            );

            self.off = (next_x, next_y);
            self.scroll_target = self.off;

            if self.kinetic.has_active_inertia() {
                moving = true;
            }
        } else {
            let (off, is_moving) = motion::step_scroll(self.off, self.scroll_target, dt);
            self.off = off;
            moving = is_moving;
        }

        animating || moving || motion_active
    }

    /// Every texture this reader still owns, for freeing when the book closes.
    pub fn into_images(self) -> Vec<Arc<RenderImage>> {
        let mut images: Vec<_> = self.images.into_values().map(|page| page.image).collect();
        images.extend(self.outgoing.map(|outgoing| outgoing.image));
        images.extend(self.released);
        images
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use yomu::library::{Book, Settings};

    #[test]
    fn test_continuous_calculations_agree() {
        let book = Book {
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
        };
        let settings = Settings {
            continuous: true,
            ..Default::default()
        };
        let mut reader = Reader::new(&book, &settings);
        reader.orig_sizes = vec![Some((1000, 1500)); 10];
        let vp = Viewport {
            width: 800.0,
            height: 600.0,
            scale: 1.0,
        };

        let (tops, heights) = reader.continuous_layout(&vp);
        let expected_total: f32 = tops.last().copied().unwrap() + heights.last().copied().unwrap();
        assert_eq!(reader.continuous_total_height(&vp), expected_total);

        for (ix, &top) in tops.iter().enumerate() {
            assert_eq!(reader.continuous_page_at(top + 1.0, &vp), ix);
        }

        let (first, last) = reader.continuous_visible_range(tops[2], tops[5] + 1.0, &vp);
        assert_eq!(first, 2);
        assert_eq!(last, 5);
    }

    #[test]
    fn test_reader_scroll_handle_scrollbar_contract() {
        let sync = Rc::new(Cell::new(None));
        let handle = ReaderScrollHandle {
            bounds: Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(800.), px(600.)),
            },
            offset_y: 150.0,
            content_height: 2400.0,
            sync: sync.clone(),
            max_offset: 1800.0,
        };

        assert_eq!(handle.viewport_bounds(), handle.bounds);
        assert_eq!(handle.offset(), point(px(0.), px(-150.0)));
        assert_eq!(handle.content_size(), size(px(800.), px(2400.0)));

        handle.set_offset(point(px(0.), px(-500.0)));
        assert_eq!(sync.get(), Some(500.0));

        // Clamping to [0.0, max_offset]
        handle.set_offset(point(px(0.), px(-2500.0)));
        assert_eq!(sync.get(), Some(1800.0));

        handle.set_offset(point(px(0.), px(100.0)));
        assert_eq!(sync.get(), Some(0.0));
    }

    #[test]
    fn test_reader_progress_bar_constructs() {
        let accent = gpui_kit::hsla(0.6, 0.8, 0.5, 1.0);
        let mut bar = reader_progress_bar(0.75, accent);
        assert_eq!(
            bar.style().background,
            Some(gpui_kit::Fill::Color(accent.into()))
        );
    }
}
