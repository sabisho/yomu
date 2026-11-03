//! Reader state, visual presentation, continuous vertical layout, page transitions, and inertial scroll management.

use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Instant,
};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::label::Label;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::{ScrollableElement, ScrollbarHandle};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Icon, Side as MenuSide, Sizable, Size as ComponentSize};
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, AnyElement, App, Bounds, ClickEvent, Div, FontWeight, InteractiveElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, PinchEvent, Pixels, Point,
    RenderImage, ScrollEvent, SharedString, Size, Stateful, Styled, TouchPhase, Window, div, img,
    point, px, relative, size,
};
use yomu::{
    archive::SourceKind,
    layout::{
        self, Arrive, FitMode, PageClassification, PageLayout, ReadingDirection, SpreadPages,
        SpreadPlan, Viewport, VisualSpread,
    },
    library::{Book, PageSpreadOverride, Settings},
    motion::{self, KineticScroll, PageTransition},
    vim::Vim,
};

use super::components::{
    CONTROL_HEIGHT_TOOLBAR, SPACE_MD, SPACE_SM, TOOLBAR_HEIGHT, button_radius, pill, with_alpha,
};
use super::theme::UiColors;

type ReaderClickCallback = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

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

/// Secondary page for dual-page spreads that is sliding out during a page turn.
#[derive(Clone)]
pub struct OutgoingSecond {
    pub image: Arc<RenderImage>,
    pub display: (f32, f32),
    pub origin: (f32, f32),
}

/// The page (or spread) that is sliding out during a page turn, frozen as it was when the turn began.
#[derive(Clone)]
pub struct Outgoing {
    pub image: Arc<RenderImage>,
    pub display: (f32, f32),
    pub origin: (f32, f32),
    pub second: Option<OutgoingSecond>,
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

fn reader_loading(colors: UiColors) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            Spinner::new()
                .with_size(ComponentSize::Medium)
                .color(colors.accent),
        )
        .child(Label::new("Opening…").text_sm().text_color(colors.muted))
        .into_any_element()
}

fn reader_error(message: impl Into<SharedString>, colors: UiColors) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .p_4()
        .child(
            Icon::new(IconName::TriangleAlert)
                .size(px(28.))
                .text_color(colors.danger),
        )
        .child(Label::new(message).text_sm().text_color(colors.muted))
        .into_any_element()
}

/// Autohiding top navigation and header overlay for the reader canvas.
#[allow(clippy::too_many_arguments)]
pub fn reader_header(
    book_title: SharedString,
    current_page_str: SharedString,
    fit_mode: FitMode,
    page_layout: PageLayout,
    reading_direction: ReadingDirection,
    first_page_is_cover: bool,
    current_spread_override: PageSpreadOverride,
    colors: UiColors,
    radius: Pixels,
    on_back: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_set_fit: impl Fn(FitMode, &mut Window, &mut App) + 'static,
    on_set_layout: impl Fn(PageLayout, &mut Window, &mut App) + 'static,
    on_set_direction: impl Fn(ReadingDirection, &mut Window, &mut App) + 'static,
    on_toggle_cover: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_set_spread_override: impl Fn(PageSpreadOverride, &mut Window, &mut App) + 'static,
    on_turn_prev: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_turn_next: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let btn_radius = button_radius(radius, CONTROL_HEIGHT_TOOLBAR);
    let on_set_fit = Rc::new(on_set_fit);
    let on_set_layout = Rc::new(on_set_layout);
    let on_set_direction = Rc::new(on_set_direction);
    let on_toggle_cover = Rc::new(on_toggle_cover);
    let on_set_spread_override = Rc::new(on_set_spread_override);
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
                .h(TOOLBAR_HEIGHT)
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
                        .gap(SPACE_MD)
                        .child(
                            Button::new("reader-back-btn")
                                .ghost()
                                .icon(IconName::ArrowLeft)
                                .with_size(ComponentSize::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(btn_radius))
                                .label("Library")
                                .tooltip("Back to library (Esc)")
                                .on_click(move |event, window, cx| {
                                    on_back(event, window, cx);
                                }),
                        )
                        .child(
                            Label::new(book_title)
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(colors.foreground)
                                .truncate()
                                .max_w(px(280.)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .child(
                            Button::new("reader-header-prev-btn")
                                .ghost()
                                .icon(IconName::ChevronLeft)
                                .with_size(ComponentSize::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(btn_radius))
                                .tooltip(if reading_direction.is_rtl() {
                                    "Next page (Left or J)"
                                } else {
                                    "Previous page (Left or K)"
                                })
                                .on_click(move |event, window, cx| {
                                    on_turn_prev(event, window, cx);
                                }),
                        )
                        .child(Tag::secondary().rounded(btn_radius).child(current_page_str))
                        .child(
                            Button::new("reader-header-next-btn")
                                .ghost()
                                .icon(IconName::ChevronRight)
                                .with_size(ComponentSize::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(btn_radius))
                                .tooltip(if reading_direction.is_rtl() {
                                    "Previous page (Right or K)"
                                } else {
                                    "Next page (Right or J)"
                                })
                                .on_click(move |event, window, cx| {
                                    on_turn_next(event, window, cx);
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(SPACE_SM)
                        .child(
                            Button::new("reader-layout-toggle")
                                .ghost()
                                .with_size(ComponentSize::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(btn_radius))
                                .dropdown_caret(true)
                                .tooltip("Page layout & reading direction")
                                .label(page_layout.label())
                                .dropdown_menu_with_anchor(Anchor::TopRight, {
                                    let on_set_layout = on_set_layout.clone();
                                    let on_set_direction = on_set_direction.clone();
                                    let on_toggle_cover = on_toggle_cover.clone();
                                    move |mut menu, _, _| {
                                        menu = menu.check_side(MenuSide::Left).min_w(px(180.));
                                        for &layout in &PageLayout::ALL {
                                            let on_set_layout = on_set_layout.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(layout.label())
                                                    .checked(layout == page_layout)
                                                    .on_click(move |_, window, cx| {
                                                        on_set_layout(layout, window, cx);
                                                    }),
                                            );
                                        }
                                        menu = menu.item(PopupMenuItem::separator());
                                        for &dir in &ReadingDirection::ALL {
                                            let on_set_direction = on_set_direction.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(dir.label())
                                                    .checked(dir == reading_direction)
                                                    .on_click(move |_, window, cx| {
                                                        on_set_direction(dir, window, cx);
                                                    }),
                                            );
                                        }
                                        menu = menu.item(PopupMenuItem::separator());
                                        let on_toggle_cover = on_toggle_cover.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new("First Page as Cover")
                                                .checked(first_page_is_cover)
                                                .on_click(move |_, window, cx| {
                                                    on_toggle_cover(
                                                        !first_page_is_cover,
                                                        window,
                                                        cx,
                                                    );
                                                }),
                                        );
                                        menu = menu.item(PopupMenuItem::separator());
                                        for &ov in &PageSpreadOverride::ALL {
                                            let on_set_spread_override =
                                                on_set_spread_override.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(match ov {
                                                    PageSpreadOverride::Auto => {
                                                        "Page: Auto Detection"
                                                    }
                                                    PageSpreadOverride::ForceSingle => {
                                                        "Page: Force Single"
                                                    }
                                                    PageSpreadOverride::ForceSpread => {
                                                        "Page: Force Spread"
                                                    }
                                                })
                                                .checked(ov == current_spread_override)
                                                .on_click(move |_, window, cx| {
                                                    on_set_spread_override(ov, window, cx);
                                                }),
                                            );
                                        }
                                        menu
                                    }
                                }),
                        )
                        .child(
                            Button::new("reader-fit-toggle")
                                .ghost()
                                .with_size(ComponentSize::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(btn_radius))
                                .dropdown_caret(true)
                                .tooltip("Display fit mode (Tab)")
                                .label(fit_mode.label())
                                .dropdown_menu_with_anchor(Anchor::TopRight, {
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
                                .icon(IconName::Maximize)
                                .with_size(ComponentSize::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(btn_radius))
                                .tooltip("Toggle fullscreen (F11)")
                                .on_click(move |_, window, _| window.toggle_fullscreen()),
                        ),
                ),
        )
}

/// Floating lateral chevrons for navigating previous and next pages.
pub fn lateral_turners(
    _colors: UiColors,
    radius: Pixels,
    is_rtl: bool,
    on_left: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_right: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> (Stateful<Div>, Stateful<Div>) {
    let left_tooltip = if is_rtl {
        "Next page (Left or J)"
    } else {
        "Previous page (Left or K)"
    };
    let right_tooltip = if is_rtl {
        "Previous page (Right or K)"
    } else {
        "Next page (Right or J)"
    };

    let left_btn = Button::new("reader-prev-chevron")
        .icon(IconName::ChevronLeft)
        .with_size(ComponentSize::Size(px(40.)))
        .rounded(ButtonRounded::Size(radius))
        .secondary()
        .outline()
        .tooltip(left_tooltip)
        .on_click(move |event, window, cx| {
            on_left(event, window, cx);
        });

    let right_btn = Button::new("reader-next-chevron")
        .icon(IconName::ChevronRight)
        .with_size(ComponentSize::Size(px(40.)))
        .rounded(ButtonRounded::Size(radius))
        .secondary()
        .outline()
        .tooltip(right_tooltip)
        .on_click(move |event, window, cx| {
            on_right(event, window, cx);
        });

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
                .group_hover("reader-prev", |s| s.opacity(1.0))
                .opacity(0.0)
                .child(left_btn),
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
                .group_hover("reader-next", |s| s.opacity(1.0))
                .opacity(0.0)
                .child(right_btn),
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
    pub book_title: SharedString,
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
    on_set_layout: impl Fn(PageLayout, &mut Window, &mut App) + 'static,
    on_set_direction: impl Fn(ReadingDirection, &mut Window, &mut App) + 'static,
    on_toggle_cover: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_set_spread_override: impl Fn(PageSpreadOverride, &mut Window, &mut App) + 'static,
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
        content.push(reader_error(
            format!("Couldn't open this book: {error}"),
            props.colors,
        ));
    } else if reader.continuous {
        let width = vp.width * reader.zoom;
        let left = (vp.width - width) / 2.0 - reader.off.0;
        let visible_top = reader.off.1;
        let visible_bottom = visible_top + vp.height;
        let (first, last) = reader.continuous_visible_range(visible_top, visible_bottom, &vp);
        let cache = reader.ensure_continuous_cache(&vp);
        if reader.total > 0 && !cache.tops.is_empty() {
            for ix in first..=last {
                let top = cache.tops[ix];
                let height = cache.heights[ix];
                let overlap = if reader.webcomics && ix + 1 < reader.total {
                    1.0
                } else {
                    0.0
                };
                if let Some(page) = reader.images.get(&ix) {
                    content.push(page_layer(
                        &page.image,
                        left,
                        top - reader.off.1,
                        (width, height + overlap),
                    ));
                } else if let Some(error) = reader.failed.get(&ix) {
                    content.push(reader_error(
                        format!("Couldn't decode page {}: {error}", ix + 1),
                        props.colors,
                    ));
                }
            }
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
            if let Some(second) = &outgoing.second {
                content.push(page_layer(
                    &second.image,
                    second.origin.0 + shift_out,
                    second.origin.1,
                    second.display,
                ));
            }
        }
        if reader.is_double_page_active() {
            let vis = reader.active_visual_spread();
            if let (Some(left_idx), Some(right_idx)) = (vis.left_page, vis.right_page) {
                let left_page = reader.images.get(&left_idx);
                let right_page = reader.images.get(&right_idx);
                if let (Some(left), Some(right)) = (left_page, right_page) {
                    let (left_display, right_display) = layout::spread_display_sizes(
                        reader.fit,
                        reader.zoom,
                        Reader::orig_of(left),
                        Reader::orig_of(right),
                        layout::SPREAD_GAP,
                        &vp,
                    );
                    let (left_pos, right_pos) = layout::spread_origins(
                        reader.off,
                        left_display,
                        right_display,
                        layout::SPREAD_GAP,
                        &vp,
                    );
                    content.push(page_layer(
                        &left.image,
                        left_pos.0 + shift_in,
                        left_pos.1,
                        left_display,
                    ));
                    content.push(page_layer(
                        &right.image,
                        right_pos.0 + shift_in,
                        right_pos.1,
                        right_display,
                    ));
                } else if reader.outgoing.is_none() {
                    let failed_left = reader.failed.get(&left_idx);
                    let failed_right = reader.failed.get(&right_idx);
                    if let Some(err) = failed_left {
                        content.push(reader_error(
                            format!("Couldn't decode page {}: {err}", left_idx + 1),
                            props.colors,
                        ));
                    } else if let Some(err) = failed_right {
                        content.push(reader_error(
                            format!("Couldn't decode page {}: {err}", right_idx + 1),
                            props.colors,
                        ));
                    } else if reader.pages.is_none()
                        || (!reader.images.contains_key(&left_idx)
                            || !reader.images.contains_key(&right_idx))
                    {
                        content.push(reader_loading(props.colors));
                    }
                }
            } else {
                let single_idx = vis.left_page.or(vis.right_page).unwrap_or(reader.page);
                if let Some(page) = reader.images.get(&single_idx) {
                    let display =
                        layout::display_size(reader.fit, reader.zoom, Reader::orig_of(page), &vp);
                    let (left, top) = layout::origin(reader.off, display, &vp);
                    content.push(page_layer(&page.image, left + shift_in, top, display));
                } else if reader.outgoing.is_none() {
                    if let Some(error) = reader.failed.get(&single_idx) {
                        content.push(reader_error(
                            format!("Couldn't decode page {}: {error}", single_idx + 1),
                            props.colors,
                        ));
                    } else if reader.pages.is_none() || !reader.images.contains_key(&single_idx) {
                        content.push(reader_loading(props.colors));
                    }
                }
            }
        } else if let Some(page) = reader.current() {
            let display = layout::display_size(reader.fit, reader.zoom, Reader::orig_of(page), &vp);
            let (left, top) = layout::origin(reader.off, display, &vp);
            content.push(page_layer(&page.image, left + shift_in, top, display));
        } else if reader.total == 0 && reader.pages.is_some() {
            content.push(reader_error("No pages found in archive", props.colors));
        } else if reader.outgoing.is_none() {
            if let Some(error) = reader.failed.get(&reader.page) {
                content.push(reader_error(
                    format!("Couldn't decode page {}: {error}", reader.page + 1),
                    props.colors,
                ));
            } else if reader.pages.is_none() || !reader.images.contains_key(&reader.page) {
                content.push(reader_loading(props.colors));
            }
        }
    }

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
                } else if reader.is_double_page_active() {
                    "double page"
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

    let vim_status = reader.vim.status().map(|text| {
        pill(
            div().absolute().bottom_4().left_0().right_0(),
            text,
            props.colors,
            props.radius,
        )
        .into_any_element()
    });

    let is_rtl = reader.reading_direction.is_rtl();
    let on_turn_prev: ReaderClickCallback = Rc::new(on_turn_prev);
    let on_turn_next: ReaderClickCallback = Rc::new(on_turn_next);
    let (left_cb, right_cb) = if is_rtl {
        (on_turn_next.clone(), on_turn_prev.clone())
    } else {
        (on_turn_prev.clone(), on_turn_next.clone())
    };

    let current_page_str = reader.current_page_str();
    let top_bar = reader_header(
        props.book_title,
        current_page_str,
        reader.fit,
        reader.page_layout,
        reader.reading_direction,
        reader.first_page_is_cover,
        reader.current_spread_override(),
        props.colors,
        props.radius,
        on_back,
        on_set_fit,
        on_set_layout,
        on_set_direction,
        on_toggle_cover,
        on_set_spread_override,
        {
            let l = left_cb.clone();
            move |e, w, cx| l(e, w, cx)
        },
        {
            let r = right_cb.clone();
            move |e, w, cx| r(e, w, cx)
        },
    );

    let (left_chevron, right_chevron) = {
        let l = left_cb.clone();
        let r = right_cb.clone();
        lateral_turners(
            props.colors,
            props.radius,
            is_rtl,
            move |e, w, cx| l(e, w, cx),
            move |e, w, cx| r(e, w, cx),
        )
    };

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
        .children(info)
        .children(hud)
        .children(vim_status)
        .child(top_bar)
        .when(!reader.continuous, |this| {
            this.child(left_chevron).child(right_chevron)
        })
        .children(reader.show_help.then(|| {
            super::overlays::help_overlay(props.colors, props.border_px, props.radius_xl_px)
        }))
        .when_some(props.scroll_handle, |this, handle| {
            this.vertical_scrollbar(handle)
        })
        .into_any_element()
}

/// State of the book that is currently open.
#[derive(Debug, Clone, Default)]
pub struct ContinuousLayoutCache {
    pub width: f32,
    pub zoom: f32,
    pub webcomics: bool,
    pub layout_version: u64,
    pub total: usize,
    pub tops: Vec<f32>,
    pub heights: Vec<f32>,
    pub bottoms: Vec<f32>,
    pub total_height: f32,
}

pub struct Reader {
    pub book_id: String,
    pub book_title: SharedString,
    pub path: PathBuf,
    pub kind: SourceKind,
    /// `None` while the page list is still being read from disk.
    pub pages: Option<Arc<Vec<String>>>,
    /// Original dimensions are retained separately from decoded textures so continuous layout
    /// can keep stable geometry while pages are evicted from the image cache.
    pub orig_sizes: Vec<Option<(u32, u32)>>,
    pub page: usize,
    pub total: usize,
    pub layout_version: u64,
    pub continuous_cache: std::cell::RefCell<Option<ContinuousLayoutCache>>,

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

    /// Page layout mode: single page or double page spread.
    pub page_layout: PageLayout,
    /// Reading progression direction: right-to-left (manga) or left-to-right (western comics).
    pub reading_direction: ReadingDirection,
    /// In dual-page mode, display the first page as an isolated single cover spread.
    pub first_page_is_cover: bool,
    /// Cached visual spread plan mapping logical pages to visual spreads.
    pub spread_plan: SpreadPlan,
    /// Manual overrides for double-page spread classification: archive entry name -> override.
    pub spread_overrides: BTreeMap<String, PageSpreadOverride>,
    /// Configurable aspect ratio threshold to automatically classify an image as a double spread.
    pub spread_ratio_threshold: f32,

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
    pub entries_scratch: Vec<yomu::pagecache::Entry>,

    pub images: HashMap<usize, Page>,
    pub pending: HashSet<usize>,
    pub cancellations: HashMap<usize, Arc<std::sync::atomic::AtomicBool>>,
    pub probe_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
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
        let spread_plan = SpreadPlan::new(
            book.page_count,
            settings.page_layout,
            settings.first_page_is_cover,
            |_| PageClassification::Single,
        );
        let mut reader = Self {
            book_id: book.id.clone(),
            book_title: SharedString::from(book.title.clone()),
            path: book.path.clone(),
            kind: book.kind,
            pages: None,
            orig_sizes: Vec::new(),
            page: book.current_page.min(book.page_count.saturating_sub(1)),
            total: book.page_count,
            layout_version: 0,
            continuous_cache: std::cell::RefCell::new(None),
            fit: settings.fit,
            zoom,
            off: (0.0, 0.0),
            scroll_target: (0.0, 0.0),
            arrive: Some(Arrive::Start),
            viewport: Viewport::EMPTY,
            page_layout: settings.page_layout,
            reading_direction: settings.reading_direction,
            first_page_is_cover: settings.first_page_is_cover,
            spread_plan,
            spread_overrides: book.spread_overrides.clone(),
            spread_ratio_threshold: settings.spread_ratio_threshold,
            transition: None,
            outgoing: None,
            transition_viewport: Viewport::EMPTY,
            animate: settings.animate,
            continuous: settings.continuous || settings.webcomics,
            webcomics: settings.webcomics,
            released: Vec::new(),
            entries_scratch: Vec::new(),
            images: HashMap::new(),
            pending: HashSet::new(),
            cancellations: HashMap::new(),
            probe_cancel: None,
            failed: HashMap::new(),
            error: None,
            vim: Vim::new(),
            drag: None,
            kinetic: KineticScroll::new(),
            back_page: None,
            show_info: false,
            show_help: false,
        };
        reader.rebuild_spread_plan();
        reader
    }

    pub fn current(&self) -> Option<&Page> {
        self.images.get(&self.page)
    }

    /// Whether dual-page spread mode is actively presenting on the current viewport.
    pub fn is_double_page_active(&self) -> bool {
        !self.continuous
            && self.page_layout.is_double()
            && layout::is_dual_spread_effective(&self.viewport)
    }

    /// Formatted page counter string (e.g. "2 / 10" in single-page/fallback or "2–3 / 10" in double-page spread).
    pub fn current_page_str(&self) -> SharedString {
        if self.total == 0 {
            return SharedString::from("0 / 0");
        }
        if self.is_double_page_active() {
            let spread = self.active_spread();
            match spread.secondary {
                Some(sec) => {
                    let min_p = spread.primary.min(sec) + 1;
                    let max_p = spread.primary.max(sec) + 1;
                    SharedString::from(format!("{}–{} / {}", min_p, max_p, self.total))
                }
                None => SharedString::from(format!("{} / {}", spread.primary + 1, self.total)),
            }
        } else {
            SharedString::from(format!("{} / {}", self.page + 1, self.total))
        }
    }

    /// Total number of visual spreads under current layout and page classifications.
    pub fn total_spreads(&self) -> usize {
        self.spread_plan.total_spreads()
    }

    /// Index of the current spread containing the reader's current page.
    pub fn current_spread_index(&self) -> usize {
        self.spread_plan.spread_index_for_page(self.page)
    }

    /// Computes the spread index for any given page.
    pub fn spread_index_for_page(&self, page: usize) -> usize {
        self.spread_plan.spread_index_for_page(page)
    }

    /// Logical pages for a given spread index.
    pub fn pages_for_spread(&self, spread_index: usize) -> Option<SpreadPages> {
        self.spread_plan.pages_for_spread(spread_index)
    }

    /// Logical pages in the currently active spread.
    pub fn active_spread(&self) -> SpreadPages {
        self.spread_plan
            .pages_for_spread(self.current_spread_index())
            .unwrap_or(SpreadPages::new(self.page, None))
    }

    /// Rebuilds the cached spread plan based on current layout, cover isolation, manual overrides,
    /// and detected image aspect ratios.
    pub fn rebuild_spread_plan(&mut self) {
        let threshold = self.spread_ratio_threshold;
        let orig_sizes = &self.orig_sizes;
        let overrides = &self.spread_overrides;
        let pages = self.pages.as_ref();

        let mut portrait = 0;
        let mut landscape = 0;
        for orig in orig_sizes.iter().flatten() {
            if orig.1 > orig.0 {
                portrait += 1;
            } else if orig.0 > orig.1 {
                landscape += 1;
            }
        }
        let typical_portrait = landscape < 3 || portrait >= landscape;

        self.spread_plan = SpreadPlan::new(
            self.total,
            self.page_layout,
            self.first_page_is_cover,
            |ix| {
                if let Some(pages) = pages
                    && let Some(name) = pages.get(ix)
                    && let Some(manual) = overrides.get(name)
                {
                    match manual {
                        PageSpreadOverride::ForceSingle => return PageClassification::Single,
                        PageSpreadOverride::ForceSpread => return PageClassification::Spread,
                        PageSpreadOverride::Auto => {}
                    }
                }
                if let Some(Some(orig)) = orig_sizes.get(ix) {
                    layout::classify_image_aspect_ratio(orig.0, orig.1, threshold, typical_portrait)
                } else {
                    PageClassification::Single
                }
            },
        );
    }

    /// Classifies an individual page based on manual overrides or detected aspect ratio.
    pub fn classify_page(&self, ix: usize) -> PageClassification {
        if let Some(pages) = self.pages.as_ref()
            && let Some(name) = pages.get(ix)
            && let Some(manual) = self.spread_overrides.get(name)
        {
            match manual {
                PageSpreadOverride::ForceSingle => return PageClassification::Single,
                PageSpreadOverride::ForceSpread => return PageClassification::Spread,
                PageSpreadOverride::Auto => {}
            }
        }
        if let Some(Some(orig)) = self.orig_sizes.get(ix) {
            let mut portrait = 0;
            let mut landscape = 0;
            for s in self.orig_sizes.iter().flatten() {
                if s.1 > s.0 {
                    portrait += 1;
                } else if s.0 > s.1 {
                    landscape += 1;
                }
            }
            let typical_portrait = landscape < 3 || portrait >= landscape;
            layout::classify_image_aspect_ratio(
                orig.0,
                orig.1,
                self.spread_ratio_threshold,
                typical_portrait,
            )
        } else {
            PageClassification::Single
        }
    }

    /// Active manual override for the reader's current page.
    pub fn current_spread_override(&self) -> PageSpreadOverride {
        if let Some(pages) = self.pages.as_ref()
            && let Some(name) = pages.get(self.page)
            && let Some(manual) = self.spread_overrides.get(name)
        {
            *manual
        } else {
            PageSpreadOverride::Auto
        }
    }

    /// Sets or clears a manual spread override for a page and updates the spread plan.
    pub fn set_spread_override(&mut self, page_index: usize, override_mode: PageSpreadOverride) {
        if let Some(pages) = self.pages.as_ref()
            && let Some(name) = pages.get(page_index)
        {
            if override_mode == PageSpreadOverride::Auto {
                self.spread_overrides.remove(name);
            } else {
                self.spread_overrides.insert(name.clone(), override_mode);
            }
            self.rebuild_spread_plan();
        }
    }

    /// Visual placement (left and right) for the active spread according to reading direction.
    pub fn active_visual_spread(&self) -> VisualSpread {
        layout::visual_spread(self.active_spread(), self.reading_direction)
    }

    pub fn ensure_continuous_cache(
        &self,
        vp: &Viewport,
    ) -> std::cell::Ref<'_, ContinuousLayoutCache> {
        let width = (vp.width * self.zoom).max(1.0);
        let is_valid = {
            if let Some(cache) = self.continuous_cache.borrow().as_ref() {
                (cache.width - width).abs() < 0.001
                    && (cache.zoom - self.zoom).abs() < 0.001
                    && cache.webcomics == self.webcomics
                    && cache.layout_version == self.layout_version
                    && cache.total == self.total
            } else {
                false
            }
        };

        if !is_valid {
            let gap = if self.webcomics {
                0.0
            } else {
                12.0 * self.zoom
            };
            let mut tops = Vec::with_capacity(self.total);
            let mut heights = Vec::with_capacity(self.total);
            let mut bottoms = Vec::with_capacity(self.total);
            let mut top = 0.0;
            for ix in 0..self.total {
                tops.push(top);
                let (w, h) = self.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
                let height = width * h as f32 / w.max(1) as f32;
                heights.push(height);
                let bottom = top + height;
                bottoms.push(bottom);
                top = bottom + gap;
            }
            let total_height = bottoms.last().copied().unwrap_or(0.0);
            *self.continuous_cache.borrow_mut() = Some(ContinuousLayoutCache {
                width,
                zoom: self.zoom,
                webcomics: self.webcomics,
                layout_version: self.layout_version,
                total: self.total,
                tops,
                heights,
                bottoms,
                total_height,
            });
        }

        std::cell::Ref::map(self.continuous_cache.borrow(), |c| c.as_ref().unwrap())
    }

    #[cfg(test)]
    pub fn continuous_layout(&self, vp: &Viewport) -> (Vec<f32>, Vec<f32>) {
        let cache = self.ensure_continuous_cache(vp);
        (cache.tops.clone(), cache.heights.clone())
    }

    pub fn continuous_total_height(&self, vp: &Viewport) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        self.ensure_continuous_cache(vp).total_height
    }

    pub fn continuous_page_at(&self, y: f32, vp: &Viewport) -> usize {
        if self.total == 0 {
            return 0;
        }
        let cache = self.ensure_continuous_cache(vp);
        cache
            .bottoms
            .partition_point(|&b| b <= y)
            .min(self.total.saturating_sub(1))
    }

    pub fn continuous_visible_range(&self, low: f32, high: f32, vp: &Viewport) -> (usize, usize) {
        if self.total == 0 {
            return (0, 0);
        }
        let cache = self.ensure_continuous_cache(vp);
        let first = cache.bottoms.partition_point(|&b| b <= low);
        let last_idx = cache.tops.partition_point(|&t| t < high);
        if first >= self.total || last_idx == 0 || first >= last_idx {
            let clamped = first.min(self.total.saturating_sub(1));
            return (clamped, clamped);
        }
        let last = (last_idx - 1).min(self.total - 1);
        (first, last.max(first))
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
        if self.is_double_page_active() {
            let vis = self.active_visual_spread();
            if let (Some(left_idx), Some(right_idx)) = (vis.left_page, vis.right_page) {
                let left_page = self.images.get(&left_idx)?;
                let right_page = self.images.get(&right_idx)?;
                let (left_display, right_display) = layout::spread_display_sizes(
                    self.fit,
                    self.zoom,
                    Self::orig_of(left_page),
                    Self::orig_of(right_page),
                    layout::SPREAD_GAP,
                    vp,
                );
                let spread_size = (
                    left_display.0 + layout::SPREAD_GAP + right_display.0,
                    left_display.1.max(right_display.1),
                );
                let max =
                    layout::spread_max_offset(left_display, right_display, layout::SPREAD_GAP, vp);
                if let Some(arrive) = self.arrive.take() {
                    self.set_scroll(layout::arrival_offset(arrive, max));
                }
                self.off = layout::clamp_offset(self.off, spread_size, vp);
                self.scroll_target = layout::clamp_offset(self.scroll_target, spread_size, vp);
                return Some(spread_size);
            } else {
                let single_idx = vis.left_page.or(vis.right_page).unwrap_or(self.page);
                let page = self.images.get(&single_idx)?;
                let orig = Self::orig_of(page);
                let display = layout::display_size(self.fit, self.zoom, orig, vp);
                let max = layout::max_offset(display, vp);
                if let Some(arrive) = self.arrive.take() {
                    self.set_scroll(layout::arrival_offset(arrive, max));
                }
                self.off = layout::clamp_offset(self.off, display, vp);
                self.scroll_target = layout::clamp_offset(self.scroll_target, display, vp);
                return Some(display);
            }
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
            if let Some(sec) = outgoing.second {
                self.release(sec.image);
            }
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
        if !enabled || vp.width < 1.0 || vp.height < 1.0 {
            self.cancel_transition();
            return;
        }

        let travel = if self.reading_direction.is_rtl() {
            -motion::travel(forward)
        } else {
            motion::travel(forward)
        };

        if self.is_double_page_active() {
            let vis = self.active_visual_spread();
            if let (Some(left_idx), Some(right_idx)) = (vis.left_page, vis.right_page) {
                let left_data = self
                    .images
                    .get(&left_idx)
                    .map(|p| (p.image.clone(), Self::orig_of(p)));
                let right_data = self
                    .images
                    .get(&right_idx)
                    .map(|p| (p.image.clone(), Self::orig_of(p)));
                let Some(((left_img, left_orig), (right_img, right_orig))) =
                    left_data.zip(right_data)
                else {
                    self.cancel_transition();
                    return;
                };
                let (left_display, right_display) = layout::spread_display_sizes(
                    self.fit,
                    self.zoom,
                    left_orig,
                    right_orig,
                    layout::SPREAD_GAP,
                    &vp,
                );
                let (left_pos, right_pos) = layout::spread_origins(
                    self.off,
                    left_display,
                    right_display,
                    layout::SPREAD_GAP,
                    &vp,
                );
                let next = match self.transition {
                    Some(running) => running.interrupted_by(travel),
                    None => PageTransition::starting(travel),
                };
                if let Some(previous) = self.outgoing.take() {
                    self.release(previous.image);
                    if let Some(sec) = previous.second {
                        self.release(sec.image);
                    }
                }
                self.outgoing = Some(Outgoing {
                    image: left_img,
                    display: left_display,
                    origin: left_pos,
                    second: Some(OutgoingSecond {
                        image: right_img,
                        display: right_display,
                        origin: right_pos,
                    }),
                });
                self.transition = Some(next);
                self.transition_viewport = vp;
                return;
            }
        }

        let on_screen = self
            .current()
            .map(|page| (page.image.clone(), Self::orig_of(page)));
        let Some((image, orig)) = on_screen else {
            self.cancel_transition();
            return;
        };

        let display = layout::display_size(self.fit, self.zoom, orig, &vp);
        let origin = layout::origin(self.off, display, &vp);
        let next = match self.transition {
            Some(running) => running.interrupted_by(travel),
            None => PageTransition::starting(travel),
        };
        if let Some(previous) = self.outgoing.take() {
            self.release(previous.image);
            if let Some(sec) = previous.second {
                self.release(sec.image);
            }
        }
        self.outgoing = Some(Outgoing {
            image,
            display,
            origin,
            second: None,
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
            let ready = if self.is_double_page_active() {
                let vis = self.active_visual_spread();
                if let (Some(l), Some(r)) = (vis.left_page, vis.right_page) {
                    self.images.contains_key(&l) && self.images.contains_key(&r)
                } else {
                    let single = vis.left_page.or(vis.right_page).unwrap_or(self.page);
                    self.images.contains_key(&single)
                }
            } else {
                self.current().is_some()
            };
            if ready {
                transition.advance(dt);
                if transition.is_done() {
                    self.transition = None;
                    if let Some(outgoing) = self.outgoing.take() {
                        self.release(outgoing.image);
                        if let Some(sec) = outgoing.second {
                            self.release(sec.image);
                        }
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
        if let Some(outgoing) = self.outgoing {
            images.push(outgoing.image);
            if let Some(sec) = outgoing.second {
                images.push(sec.image);
            }
        }
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
            spread_overrides: BTreeMap::new(),
            page_dimensions: Default::default(),
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
    fn test_continuous_cache_fuzzy_matches_naive_reference() {
        let mut reader = Reader::new(
            &Book {
                id: "test".into(),
                title: "Test".into(),
                series: None,
                path: PathBuf::from("/tmp/test.cbz"),
                kind: SourceKind::Archive,
                page_count: 50,
                current_page: 0,
                opened: false,
                added_at: 1,
                last_read: None,
                marks: BTreeMap::new(),
                spread_overrides: BTreeMap::new(),
                page_dimensions: Arc::default(),
            },
            &Settings {
                continuous: true,
                ..Default::default()
            },
        );

        let dims = [
            (1000, 1500),
            (1920, 1080),
            (2400, 1600),
            (1200, 1200),
            (800, 2400),
        ];
        reader.orig_sizes = (0..50).map(|i| Some(dims[i % dims.len()])).collect();

        let vp = Viewport {
            width: 1200.0,
            height: 900.0,
            scale: 1.0,
        };

        let naive_layout = |r: &Reader, vp: &Viewport| -> (Vec<f32>, Vec<f32>, f32) {
            let width = (vp.width * r.zoom).max(1.0);
            let gap = if r.webcomics { 0.0 } else { 12.0 * r.zoom };
            let mut tops = Vec::with_capacity(r.total);
            let mut heights = Vec::with_capacity(r.total);
            let mut top = 0.0;
            for ix in 0..r.total {
                tops.push(top);
                let (w, h) = r.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
                let height = width * h as f32 / w.max(1) as f32;
                heights.push(height);
                top += height + gap;
            }
            let total = if tops.is_empty() {
                0.0
            } else {
                tops.last().unwrap() + heights.last().unwrap()
            };
            (tops, heights, total)
        };

        let naive_page_at = |r: &Reader, y: f32, vp: &Viewport| -> usize {
            let (tops, heights, _) = naive_layout(r, vp);
            for (ix, (&top, &height)) in tops.iter().zip(&heights).enumerate() {
                if y < top + height {
                    return ix;
                }
            }
            r.total.saturating_sub(1)
        };

        for zoom in [0.5, 1.0, 1.5, 2.5] {
            reader.zoom = zoom;
            for webcomics in [false, true] {
                reader.webcomics = webcomics;
                reader.layout_version += 1;

                let (ref_tops, ref_heights, ref_total) = naive_layout(&reader, &vp);
                let cached_total = reader.continuous_total_height(&vp);
                assert!(
                    (cached_total - ref_total).abs() < 0.01,
                    "cached total {cached_total} must match naive reference {ref_total} at zoom {zoom}"
                );

                let cache = reader.ensure_continuous_cache(&vp);
                for i in 0..reader.total {
                    assert!((cache.tops[i] - ref_tops[i]).abs() < 0.01);
                    assert!((cache.heights[i] - ref_heights[i]).abs() < 0.01);
                }

                for step in 0..=100 {
                    let y = ref_total * (step as f32 / 100.0);
                    let ref_page = naive_page_at(&reader, y, &vp);
                    let cached_page = reader.continuous_page_at(y, &vp);
                    assert_eq!(
                        cached_page, ref_page,
                        "page at y={y} (zoom={zoom}, webcomics={webcomics}) mismatch: cached={cached_page}, naive={ref_page}"
                    );
                }
            }
        }
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
    fn test_empty_archive_page_counter() {
        let book = Book {
            id: "empty".into(),
            title: "Empty Book".into(),
            series: None,
            path: PathBuf::from("/nonexistent/empty.cbz"),
            kind: SourceKind::Archive,
            page_count: 0,
            current_page: 0,
            opened: true,
            added_at: 1,
            last_read: None,
            marks: BTreeMap::new(),
            spread_overrides: BTreeMap::new(),
            page_dimensions: Default::default(),
        };
        let settings = Settings::default();
        let reader = Reader::new(&book, &settings);
        assert_eq!(reader.total, 0);
        assert_eq!(reader.current_page_str().as_ref(), "0 / 0");
    }
}
