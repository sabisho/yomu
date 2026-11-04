//! Reader orchestration, page decoding queue, prefetch planning, and gesture navigation.

#![forbid(unsafe_code)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use anyhow::Result;
use gpui_kit::{Context, RenderImage, TouchPhase, Window};
use image::Frame;
use yomu::{
    archive, imaging,
    layout::{self, Arrive, FitMode, PageLayout, ReadingDirection, Viewport},
    pagecache,
    util::now_secs,
    vim::{Amount, Dir},
};

use crate::app::state::{DRAG_THRESHOLD, EDGE_EPS, LINE_STEP, Side, WHEEL_LINE, Yomu};
use crate::ui::reader::{Drag, Page, Reader, ReaderSyncOutcome, WheelParams, viewport_of};

impl Yomu {
    // ------------------------------------------------------------------------------------
    // Opening and closing books
    // ------------------------------------------------------------------------------------

    pub(crate) fn open_book(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        let Some(book) = self.library.get(id).cloned() else {
            return;
        };
        if !book.path.exists() {
            self.show_toast(format!("“{}” is missing from disk", book.title), cx);
            return;
        }

        self.selection.clear();
        self.search_active = false;
        self.search_query.clear();
        if let Some(input) = &self.search_input {
            input.update(cx, |state, cx| {
                state.set_value("", window, cx);
            });
        }
        window.set_window_title(&book.title);
        window.focus(&self.focus_handle, cx);

        self.cleanup_current_reader();
        self.drain_garbage(window);

        let settings = self.library.settings().clone();
        self.reader = Some(Reader::new(&book, &settings));

        self.library
            .update_progress(id, book.current_page, true, Some(now_secs()));
        self.progress
            .set(id, book.current_page, true, Some(now_secs()));
        self.save_progress_debounced(cx);

        // Listing a big archive's pages touches the disk, so keep it off the UI thread.
        let (path, kind, book_id) = (book.path, book.kind, book.id);
        let task = cx
            .background_executor()
            .spawn(async move { archive::list_pages(&path, kind) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.pages_listed(book_id, result, cx))
                .ok();
        })
        .detach();

        cx.notify();
    }

    pub(crate) fn pages_listed(
        &mut self,
        book_id: String,
        result: Result<Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
        let Some(reader) = self.reader.as_mut() else {
            return;
        };

        if reader.book_id != book_id {
            return;
        }
        match result {
            Ok(pages) if pages.is_empty() => reader.error = Some("no pages found".into()),
            Ok(pages) => {
                reader.total = pages.len();
                reader.page = reader.page.min(pages.len() - 1);
                let (page, total) = (reader.page, pages.len());
                let mut orig_sizes = vec![None; total];
                if let Some(book) = self.library.get(&book_id) {
                    for (ix, name) in pages.iter().enumerate() {
                        if let Some(&dims) = book.page_dimensions.get(name) {
                            orig_sizes[ix] = Some(dims);
                        }
                    }
                    reader.spread_overrides = book.spread_overrides.clone();
                }
                reader.orig_sizes = orig_sizes;
                reader.layout_version += 1;
                reader.pages = Some(Arc::new(pages.clone()));
                reader.rebuild_spread_plan();

                let unprobed: Vec<(usize, String)> = reader
                    .orig_sizes
                    .iter()
                    .enumerate()
                    .filter(|(_, size)| size.is_none())
                    .map(|(ix, _)| (ix, pages[ix].clone()))
                    .collect();
                let path = reader.path.clone();
                let kind = reader.kind;

                let cancel = if !unprobed.is_empty() {
                    let cancel = Arc::new(AtomicBool::new(false));
                    reader.probe_cancel = Some(cancel.clone());
                    Some(cancel)
                } else {
                    None
                };

                let mut page_count_changed = false;
                if let Some(book) = self.library.get_mut(&book_id) {
                    if book.page_count != total {
                        book.page_count = total;
                        page_count_changed = true;
                    }
                    book.current_page = page;
                }
                self.progress.set(&book_id, page, true, Some(now_secs()));
                self.save_progress_debounced(cx);
                if page_count_changed {
                    self.save_library(cx);
                }

                if let Some(cancel) = cancel {
                    let probe_book_id = book_id.clone();
                    let task = cx.background_executor().spawn(async move {
                        archive::probe_pages_dimensions(&path, kind, &unprobed, Some(&cancel))
                    });
                    cx.spawn(async move |this, cx| {
                        let probed = task.await;
                        this.update(cx, |this, cx| {
                            this.pages_probed(probe_book_id, probed, cx);
                        })
                        .ok();
                    })
                    .detach();
                }
            }
            Err(err) => reader.error = Some(format!("{err:#}")),
        }
        cx.notify();
    }

    pub(crate) fn pages_probed(
        &mut self,
        book_id: String,
        probed: Vec<(usize, String, (u32, u32))>,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        if reader.book_id != book_id {
            return;
        }
        reader.probe_cancel = None;
        if probed.is_empty() {
            return;
        }
        let mut changed = false;
        for (ix, name, dims) in probed {
            if let Some(slot) = reader.orig_sizes.get_mut(ix)
                && slot.is_none()
            {
                *slot = Some(dims);
                changed = true;
            }
            if let Some(book) = self.library.get_mut(&book_id) {
                book.insert_page_dimension(name, dims);
            }
        }
        if changed {
            reader.layout_version += 1;
            reader.rebuild_spread_plan();
            self.save_library_debounced(cx);
            cx.notify();
        }
    }

    pub(crate) fn cleanup_current_reader(&mut self) {
        if let Some(reader) = self.reader.take() {
            if let Some(ref cancel) = reader.probe_cancel {
                cancel.store(true, Ordering::Relaxed);
            }
            for token in reader.cancellations.values() {
                token.store(true, Ordering::Relaxed);
            }
            self.garbage.extend(reader.into_images());
        }
    }

    pub(crate) fn drain_garbage(&mut self, window: &mut Window) {
        for image in self.garbage.drain(..) {
            let _ = window.drop_image(image);
        }
    }

    pub(crate) fn close_book(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        self.cleanup_current_reader();
        self.drain_garbage(window);
        jemalloc_purge();
        self.last_frame = None;
        self.hud = None;
        self.cursor_pos = None;
        window.set_window_title("yomu");
        window.focus(&self.focus_handle, cx);
        self.save_progress(cx);
        cx.notify();
    }

    // ------------------------------------------------------------------------------------
    // Page turning & navigation
    // ------------------------------------------------------------------------------------

    pub(crate) fn goto_page(&mut self, target: usize, arrive: Arrive, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        if reader.pages.is_none() || reader.total == 0 {
            return;
        }
        let target = target.min(reader.total - 1);
        if target == reader.page {
            return;
        }
        // The reading position changes immediately; the slide is only a visual that catches up.
        let same_spread = reader.is_double_page_active()
            && reader.current_spread_index() == reader.spread_index_for_page(target);
        if !same_spread {
            reader.begin_transition(target > reader.page, !reader.continuous);
        }
        reader.page = target;
        reader.arrive = Some(arrive);
        reader.drag = None;
        let book_id = reader.book_id.clone();
        self.library
            .update_progress(&book_id, target, true, Some(now_secs()));
        self.progress.set(&book_id, target, true, Some(now_secs()));
        self.save_progress_debounced(cx);
        self.wake();
        cx.notify();
    }

    /// A jump that `''` can undo (`gg`, `G`, `12G`, `:12`, marks).
    pub(crate) fn jump_to(&mut self, target: usize, cx: &mut Context<Self>) {
        if let Some(reader) = self.reader.as_mut() {
            let clamped = target.min(reader.total.saturating_sub(1));
            if clamped != reader.page {
                reader.back_page = Some(reader.page);
            }
        }
        self.goto_page(target, Arrive::Start, cx);
    }

    pub(crate) fn turn_by(
        &mut self,
        forward: bool,
        count: usize,
        arrive: Arrive,
        cx: &mut Context<Self>,
    ) {
        let Some(reader) = self.reader.as_ref() else {
            return;
        };
        let page = reader.page;
        let target = if reader.is_double_page_active() {
            let current_s = reader.current_spread_index();
            let total_s = reader.total_spreads();
            let target_s = if forward {
                current_s
                    .saturating_add(count)
                    .min(total_s.saturating_sub(1))
            } else {
                current_s.saturating_sub(count)
            };
            if let Some(pages) = reader.pages_for_spread(target_s) {
                pages.primary
            } else {
                page
            }
        } else if forward {
            page.saturating_add(count)
        } else {
            page.saturating_sub(count)
        };
        self.goto_page(target, arrive, cx);
    }

    /// Screen-edge clicks: left goes back in LTR (forward in RTL), right advances in LTR (back in RTL).
    pub(crate) fn step(&mut self, side: Side, cx: &mut Context<Self>) {
        let Some(reader) = self.reader.as_ref() else {
            return;
        };
        let is_rtl = reader.reading_direction.is_rtl();
        let forward = match side {
            Side::Right => !is_rtl,
            Side::Left => is_rtl,
        };
        let arrive = if forward { Arrive::Start } else { Arrive::End };
        self.turn_by(forward, 1, arrive, cx);
    }

    // ------------------------------------------------------------------------------------
    // Scrolling, zooming, fit modes
    // ------------------------------------------------------------------------------------

    /// `j`/`k`/`h`/`l`/space/etc. Scrolls the page; when the page cannot scroll any further in
    /// that direction (or does not overflow at all) it turns the page instead.
    pub(crate) fn scroll(
        &mut self,
        dir: Dir,
        amount: Amount,
        count: usize,
        vp: Viewport,
        cx: &mut Context<Self>,
    ) {
        let count = count.max(1);
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let Some(display) = reader.settle(&vp) else {
            return;
        };
        let max = if reader.continuous {
            let doc_height = reader.continuous_total_height(&vp);
            (
                (display.0 - vp.width).max(0.0),
                (doc_height - vp.height).max(0.0),
            )
        } else {
            layout::max_offset(display, &vp)
        };
        let vertical = matches!(dir, Dir::Up | Dir::Down);
        let positive = matches!(dir, Dir::Down | Dir::Right);
        // Edge checks and repeated presses work on where the scroll is *heading*, so pressing
        // `j` several times quickly accumulates instead of restarting from the glide's position.
        let (axis_max, view, current) = if vertical {
            (max.1, vp.height, reader.scroll_target.1)
        } else {
            (max.0, vp.width, reader.scroll_target.0)
        };

        let is_rtl = reader.reading_direction.is_rtl();
        let turn_forward = if vertical {
            positive
        } else if is_rtl {
            matches!(dir, Dir::Left)
        } else {
            matches!(dir, Dir::Right)
        };
        let arrive = if turn_forward {
            Arrive::Start
        } else {
            Arrive::End
        };

        if axis_max <= EDGE_EPS && !(reader.continuous && vertical) {
            self.turn_by(turn_forward, count, arrive, cx);
            return;
        }
        let at_end = if vertical || !is_rtl {
            if positive {
                current >= axis_max - EDGE_EPS
            } else {
                current <= EDGE_EPS
            }
        } else if matches!(dir, Dir::Left) {
            current <= EDGE_EPS
        } else {
            current >= axis_max - EDGE_EPS
        };
        if at_end && !(reader.continuous && vertical) {
            self.turn_by(turn_forward, 1, arrive, cx);
            return;
        }

        let step = match amount {
            Amount::Line => LINE_STEP,
            Amount::HalfPage => view * 0.5,
            Amount::Page => view * 0.9,
        };
        let delta = step * count as f32 * if positive { 1.0 } else { -1.0 };
        let target = (current + delta).clamp(0.0, axis_max);
        if vertical {
            reader.scroll_target.1 = target;
        } else {
            reader.scroll_target.0 = target;
        }
        if !reader.animate {
            reader.off = reader.scroll_target;
        }
        self.wake();
        cx.notify();
    }

    pub(crate) fn scroll_vertical_to(
        &mut self,
        fraction: f32,
        vp: Viewport,
        cx: &mut Context<Self>,
    ) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let Some(display) = reader.settle(&vp) else {
            return;
        };
        let max = if reader.continuous {
            (reader.continuous_total_height(&vp) - vp.height).max(0.0)
        } else {
            layout::max_offset(display, &vp).1
        };
        reader.scroll_target.1 = max * fraction;
        self.wake();
        cx.notify();
    }

    pub(crate) fn zoom_to(
        &mut self,
        zoom: f32,
        anchor: (f32, f32),
        vp: Viewport,
        cx: &mut Context<Self>,
    ) {
        let message = {
            let Some(reader) = self.reader.as_mut() else {
                return;
            };
            let Some(old_display) = reader.settle(&vp) else {
                return;
            };
            if reader.continuous {
                let old_zoom = reader.zoom;
                let old_doc = reader.continuous_total_height(&vp);
                let zoom = layout::clamp_zoom(zoom);
                reader.zoom = zoom;
                let new_doc = reader.continuous_total_height(&vp);

                if old_zoom > 1e-4 {
                    let zoom_ratio = zoom / old_zoom;
                    let target_y = (reader.off.1 + anchor.1) * zoom_ratio - anchor.1;
                    let max_y = (new_doc - vp.height).max(0.0);
                    reader.scroll_target.1 = target_y.clamp(0.0, max_y);
                } else if old_doc > vp.height {
                    reader.scroll_target.1 = reader.scroll_target.1 / (old_doc - vp.height)
                        * (new_doc - vp.height).max(0.0);
                } else {
                    reader.scroll_target.1 = 0.0;
                }

                let new_width = vp.width * zoom;
                let max_x = (new_width - vp.width).max(0.0);
                if max_x > 0.0 && old_display.0 > 0.0 {
                    let old_left = (vp.width - old_display.0) / 2.0 - reader.off.0;
                    let fraction_x = (anchor.0 - old_left) / old_display.0;
                    let new_left = anchor.0 - fraction_x * new_width;
                    let target_x = (vp.width - new_width) / 2.0 - new_left;
                    reader.scroll_target.0 = target_x.clamp(0.0, max_x);
                } else {
                    reader.scroll_target.0 = 0.0;
                }

                reader.off = reader.scroll_target;
                reader.cancel_transition();
                format!("continuous · {}%", (zoom * 100.0).round() as i32)
            } else if reader.is_double_page_active() {
                let vis = reader.active_visual_spread();
                let zoom = layout::clamp_zoom(zoom);
                let (new_display, new_spread_size) =
                    if let (Some(left_idx), Some(right_idx)) = (vis.left_page, vis.right_page) {
                        if let (Some(left_page), Some(right_page)) =
                            (reader.images.get(&left_idx), reader.images.get(&right_idx))
                        {
                            let (ld, rd) = layout::spread_display_sizes(
                                reader.fit,
                                zoom,
                                Reader::orig_of(left_page),
                                Reader::orig_of(right_page),
                                layout::SPREAD_GAP,
                                &vp,
                            );
                            let spread_sz = (ld.0 + layout::SPREAD_GAP + rd.0, ld.1.max(rd.1));
                            (spread_sz, spread_sz)
                        } else {
                            return;
                        }
                    } else {
                        let single_idx = vis.left_page.or(vis.right_page).unwrap_or(reader.page);
                        if let Some(page) = reader.images.get(&single_idx) {
                            let orig = Reader::orig_of(page);
                            let disp = layout::display_size(reader.fit, zoom, orig, &vp);
                            (disp, disp)
                        } else {
                            return;
                        }
                    };
                let old_origin = layout::origin(reader.off, old_display, &vp);
                let off = layout::zoom_anchor_offset(old_origin, old_display, new_display, anchor);
                reader.cancel_transition();
                reader.zoom = zoom;
                reader.set_scroll(layout::clamp_offset(off, new_spread_size, &vp));
                format!(
                    "{} · {}%",
                    reader.fit.label(),
                    (zoom * 100.0).round() as i32
                )
            } else {
                let Some(orig) = reader.current().map(Reader::orig_of) else {
                    return;
                };
                let zoom = layout::clamp_zoom(zoom);
                let old_origin = layout::origin(reader.off, old_display, &vp);
                let new_display = layout::display_size(reader.fit, zoom, orig, &vp);
                let off = layout::zoom_anchor_offset(old_origin, old_display, new_display, anchor);
                reader.cancel_transition();
                reader.zoom = zoom;
                reader.set_scroll(layout::clamp_offset(off, new_display, &vp));
                format!(
                    "{} · {}%",
                    reader.fit.label(),
                    (zoom * 100.0).round() as i32
                )
            }
        };
        if self.library.settings().remember_zoom
            && let Some(reader) = self.reader.as_ref()
        {
            self.library.settings_mut().zoom = reader.zoom;
            self.save_library_debounced(cx);
        }
        self.show_hud(message, cx);
    }

    pub(crate) fn zoom_by(
        &mut self,
        factor: f32,
        anchor: (f32, f32),
        vp: Viewport,
        cx: &mut Context<Self>,
    ) {
        let Some(zoom) = self.reader.as_ref().map(|r| r.zoom) else {
            return;
        };
        self.zoom_to(zoom * factor, anchor, vp, cx);
    }

    pub(crate) fn set_fit(&mut self, mode: FitMode, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        if let Some(reader) = self.reader.as_mut() {
            reader.cancel_transition();
            reader.fit = mode;
            reader.zoom = 1.0;
            reader.arrive = Some(Arrive::Start);
            self.show_hud(mode.label(), cx);
        }
        self.library.settings_mut().fit = mode;
        if self.library.settings().remember_zoom {
            self.library.settings_mut().zoom = 1.0;
        }
        self.save_library(cx);
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn cycle_fit(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.reader.as_ref().map(|r| r.fit.next()) {
            self.set_fit(next, cx);
        } else {
            let next = self.library.settings().fit.next();
            self.set_fit(next, cx);
        }
    }

    pub(crate) fn set_remember_zoom(&mut self, value: Option<bool>, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        let next = value.unwrap_or(!self.library.settings().remember_zoom);
        self.library.settings_mut().remember_zoom = next;
        if next && let Some(reader) = self.reader.as_ref() {
            self.library.settings_mut().zoom = reader.zoom;
        }
        self.save_library(cx);
        self.show_hud(
            if next {
                "remember zoom on"
            } else {
                "remember zoom off"
            },
            cx,
        );
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn reset_zoom(&mut self, cx: &mut Context<Self>) {
        self.library.settings_mut().zoom = 1.0;
        if let Some(reader) = self.reader.as_mut() {
            reader.zoom = 1.0;
            reader.cancel_transition();
            reader.arrive = Some(Arrive::Start);
        }
        self.save_library(cx);
        self.show_hud("zoom reset (100%)", cx);
        cx.notify();
    }

    pub(crate) fn set_continuous(&mut self, value: Option<bool>, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        let next = value.unwrap_or(!self.library.settings().continuous);
        self.library.settings_mut().continuous = next;
        if let Some(reader) = self.reader.as_mut() {
            reader.continuous = next || reader.webcomics;
            reader.zoom = 1.0;
            reader.off = (0.0, 0.0);
            reader.scroll_target = (0.0, 0.0);
            reader.arrive = Some(Arrive::Start);
            reader.cancel_transition();
        }
        if self.library.settings().remember_zoom {
            self.library.settings_mut().zoom = 1.0;
        }
        self.save_library(cx);
        let webcomics = self.library.settings().webcomics;
        self.show_hud(
            if webcomics {
                "webcomics mode"
            } else if next {
                "continuous scrolling"
            } else {
                "single page"
            },
            cx,
        );
    }

    pub(crate) fn set_webcomics(&mut self, value: Option<bool>, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        let next = value.unwrap_or(!self.library.settings().webcomics);
        self.library.settings_mut().webcomics = next;
        if let Some(reader) = self.reader.as_mut() {
            reader.webcomics = next;
            reader.continuous = next || self.library.settings().continuous;
            reader.zoom = 1.0;
            reader.off = (0.0, 0.0);
            reader.scroll_target = (0.0, 0.0);
            reader.arrive = Some(Arrive::Start);
            reader.cancel_transition();
        }
        if self.library.settings().remember_zoom {
            self.library.settings_mut().zoom = 1.0;
        }
        self.save_library(cx);
        self.show_hud(
            if next {
                "webcomics mode"
            } else if self.library.settings().continuous {
                "continuous scrolling"
            } else {
                "single page"
            },
            cx,
        );
    }

    // ------------------------------------------------------------------------------------
    // Gestures and interactions
    // ------------------------------------------------------------------------------------

    pub(crate) fn on_wheel(&mut self, params: WheelParams, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        let WheelParams {
            delta,
            zoom,
            at,
            discrete,
            touch_phase,
            vp,
        } = params;
        let (dx, dy) = delta;
        let Some(reader) = self.reader.as_mut() else {
            return;
        };

        let now = Instant::now();
        if touch_phase == TouchPhase::Started
            || touch_phase == TouchPhase::Cancelled
            || discrete
            || reader.kinetic.has_active_inertia()
        {
            reader.kinetic.reset();
        }

        let Some(display) = reader.settle(&vp) else {
            return;
        };

        if zoom {
            reader.kinetic.cancel_inertia();
            // Wheel up (positive) zooms in, anchored at the pointer.
            self.zoom_by(1.2f32.powf(dy / WHEEL_LINE), at, vp, cx);
            return;
        }

        let max = if reader.continuous {
            let doc_height = reader.continuous_total_height(&vp);
            (
                (display.0 - vp.width).max(0.0),
                (doc_height - vp.height).max(0.0),
            )
        } else {
            layout::max_offset(display, &vp)
        };

        if !discrete
            && (dx.abs() > 0.001 || dy.abs() > 0.001)
            && touch_phase != TouchPhase::Cancelled
        {
            // Delta displacement in offset space is (-dx, -dy)
            reader.kinetic.push_motion(now, -dx, -dy);
        }

        if touch_phase == TouchPhase::Ended && reader.kinetic.end_touch(true) {
            self.wake();
            cx.notify();
            return;
        }

        if reader.continuous {
            let target = (reader.scroll_target.1 - dy).clamp(0.0, max.1);
            reader.scroll_target.1 = target;
            if !discrete || !reader.animate {
                reader.off.1 = target;
            }
        } else if max.1 > EDGE_EPS {
            // Wheel down gives a negative dy and should move further down the page.
            let at_bottom = reader.scroll_target.1 >= max.1 - EDGE_EPS;
            let at_top = reader.scroll_target.1 <= EDGE_EPS;
            if discrete && ((dy < 0.0 && at_bottom) || (dy > 0.0 && at_top)) {
                let forward = dy < 0.0;
                let arrive = if forward { Arrive::Start } else { Arrive::End };
                self.turn_by(forward, 1, arrive, cx);
                return;
            }
            let target = (reader.scroll_target.1 - dy).clamp(0.0, max.1);
            reader.scroll_target.1 = target;
            if !discrete {
                // Touchpads already send a smooth stream of small deltas: follow them 1:1.
                reader.off.1 = target;
            }
        } else if discrete && dy != 0.0 {
            let forward = dy < 0.0;
            let arrive = if forward { Arrive::Start } else { Arrive::End };
            self.turn_by(forward, 1, arrive, cx);
            return;
        }
        if max.0 > EDGE_EPS {
            let target = (reader.scroll_target.0 - dx).clamp(0.0, max.0);
            reader.scroll_target.0 = target;
            if !discrete {
                reader.off.0 = target;
            }
        }
        self.wake();
        cx.notify();
    }

    pub(crate) fn on_pinch(
        &mut self,
        at: (f32, f32),
        delta: f32,
        phase: TouchPhase,
        vp: Viewport,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
        let Some(reader) = self.reader.as_mut() else {
            return;
        };

        if phase == TouchPhase::Started || phase == TouchPhase::Cancelled {
            reader.kinetic.reset();
            return;
        }

        if phase == TouchPhase::Ended {
            return;
        }

        if delta.abs() > 1e-5 {
            reader.kinetic.cancel_inertia();
            let factor = (1.0 + delta).clamp(0.2, 5.0);
            self.zoom_by(factor, at, vp, cx);
        }
    }

    pub(crate) fn drag_start(&mut self, at: (f32, f32)) {
        if self.is_quitting {
            return;
        }
        // Prevent click/drag interaction when mouse is over the top bar
        if at.1 <= 56.0 {
            return;
        }
        if let Some(reader) = self.reader.as_mut() {
            reader.kinetic.reset();
            let off0 = reader.off;
            reader.drag = Some(Drag {
                start: at,
                off0,
                moved: false,
                last_at: at,
            });
        }
    }

    pub(crate) fn drag_move(
        &mut self,
        at: (f32, f32),
        pressed: bool,
        vp: Viewport,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let Some(drag) = reader.drag else {
            return;
        };
        if !pressed {
            reader.drag = None;
            return;
        }
        let (dx, dy) = (at.0 - drag.start.0, at.1 - drag.start.1);
        if !drag.moved && dx.abs().max(dy.abs()) < DRAG_THRESHOLD {
            return;
        }
        let Some(display) = reader.settle(&vp) else {
            return;
        };
        let now = Instant::now();
        let (step_x, step_y) = (at.0 - drag.last_at.0, at.1 - drag.last_at.1);
        reader.kinetic.push_motion(now, -step_x, -step_y);

        reader.drag = Some(Drag {
            moved: true,
            last_at: at,
            ..drag
        });
        let target = (drag.off0.0 - dx, drag.off0.1 - dy);
        if reader.continuous {
            let doc_height = reader.continuous_total_height(&vp);
            let max = (
                (display.0 - vp.width).max(0.0),
                (doc_height - vp.height).max(0.0),
            );
            reader.set_scroll((target.0.clamp(0.0, max.0), target.1.clamp(0.0, max.1)));
        } else {
            reader.set_scroll(layout::clamp_offset(target, display, &vp));
        }
        cx.notify();
    }

    /// Mouse released: if it was a drag flick, launch inertial glide; otherwise click.
    pub(crate) fn drag_end(&mut self, x: f32, vp: Viewport, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let Some(drag) = reader.drag.take() else {
            return;
        };
        if drag.moved {
            if reader.kinetic.end_touch(true) {
                self.wake();
                cx.notify();
            }
            return;
        }
        if x < vp.width / 3.0 {
            self.step(Side::Left, cx);
        } else if x > vp.width * 2.0 / 3.0 {
            self.step(Side::Right, cx);
        }
    }

    // ------------------------------------------------------------------------------------
    // Page decoding & prefetch scheduling
    // ------------------------------------------------------------------------------------

    /// Called every render: settles the scroll position, frees far-away pages and decodes the
    /// current and next pages at the resolution the current zoom needs.
    pub(crate) fn sync_reader(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        dt: f32,
    ) -> bool {
        let vp = viewport_of(window);

        let ReaderSyncOutcome {
            animating,
            progress_changed,
        } = {
            let Some(reader) = self.reader.as_mut() else {
                return false;
            };
            reader.viewport = vp;
            reader.animate = true;
            let pages_len = match reader.pages.as_deref() {
                Some(pages) => pages.len(),
                None => return false,
            };
            let previous_page = reader.page;
            reader.settle(&vp);
            let animating = reader.advance_animations(dt, &vp);
            if reader.continuous {
                reader.settle(&vp);
            }
            let progress_changed = if reader.continuous && reader.page != previous_page {
                let id = reader.book_id.clone();
                let page = reader.page;
                self.library
                    .update_progress(&id, page, true, Some(now_secs()));
                self.progress.set(&id, page, true, Some(now_secs()));
                Some((id, page))
            } else {
                None
            };

            // Keep a bounded window of decoded pages around the current one, and stay under the
            // memory budget even with very large pages.
            reader.entries_scratch.clear();
            reader
                .entries_scratch
                .extend(reader.images.iter().map(|(&index, page)| pagecache::Entry {
                    index,
                    bytes: page.bytes,
                }));
            let keep = if reader.continuous {
                let low = reader.off.1;
                let high = low + vp.height;
                let (first, last) = reader.continuous_visible_range(low, high, &vp);
                first.saturating_sub(1)..=(last + 2).min(pages_len.saturating_sub(1))
            } else if reader.is_double_page_active() {
                let spread = reader.active_spread();
                let min_p = spread
                    .primary
                    .min(spread.secondary.unwrap_or(spread.primary));
                let max_p = spread
                    .primary
                    .max(spread.secondary.unwrap_or(spread.primary));
                min_p.saturating_sub(pagecache::KEEP_BEHIND)
                    ..=(max_p + pagecache::KEEP_AHEAD).min(pages_len.saturating_sub(1))
            } else {
                reader.page.saturating_sub(pagecache::KEEP_BEHIND)
                    ..=reader.page + pagecache::KEEP_AHEAD
            };
            for ix in pagecache::plan_eviction_with_keep(
                &reader.entries_scratch,
                reader.page,
                pagecache::system_memory_budget(),
                &keep,
            ) {
                if reader.is_double_page_active() && reader.active_spread().contains(ix) {
                    continue;
                }
                if let Some(page) = reader.images.remove(&ix) {
                    // A page that is still sliding out is freed when its slide ends instead.
                    let leaving = reader.outgoing.as_ref().is_some_and(|outgoing| {
                        Arc::ptr_eq(&outgoing.image, &page.image)
                            || outgoing
                                .second
                                .as_ref()
                                .is_some_and(|sec| Arc::ptr_eq(&sec.image, &page.image))
                    });
                    if !leaving {
                        let _ = window.drop_image(page.image);
                    }
                }
            }

            // When zoom is retained, neighbours are pre-fetched at the reader's active zoom
            // so turning the page stays sharp without a secondary re-decode.
            let long_side = |display: (f32, f32)| display.0.max(display.1) * vp.scale;
            let (current_tier, neighbour_tier) = match reader.current().map(Reader::orig_of) {
                Some(orig) => {
                    let tier = layout::tier_for(long_side(layout::display_size(
                        reader.fit,
                        reader.zoom,
                        orig,
                        &vp,
                    )));
                    (tier, tier)
                }
                None => {
                    let tier = layout::tier_for(vp.width.max(vp.height) * vp.scale * reader.zoom);
                    (tier, tier)
                }
            };

            reader.wanted_scratch.clear();
            let is_double_active = reader.is_double_page_active();
            let active_spread = if is_double_active {
                Some(reader.active_spread())
            } else {
                None
            };
            let to_tier = |ix: usize| -> u32 {
                if reader.continuous {
                    let width = vp.width * reader.zoom * vp.scale;
                    let (w, h) = reader.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
                    layout::tier_for(width.max(width * h as f32 / w.max(1) as f32))
                } else if (is_double_active
                    && active_spread.as_ref().is_some_and(|s| s.contains(ix)))
                    || (!is_double_active && ix == reader.page)
                {
                    current_tier
                } else {
                    neighbour_tier
                }
            };

            if reader.continuous {
                for ix in keep {
                    if ix < pages_len {
                        let tier = to_tier(ix);
                        reader.wanted_scratch.push((ix, tier));
                    }
                }
            } else if is_double_active {
                let current_s = reader.current_spread_index();
                let total_s = reader.total_spreads();

                // 1. Current spread pages
                if let Some(pages) = reader.pages_for_spread(current_s) {
                    reader
                        .wanted_scratch
                        .push((pages.primary, to_tier(pages.primary)));
                    if let Some(sec) = pages.secondary {
                        reader.wanted_scratch.push((sec, to_tier(sec)));
                    }
                }
                // 2. Next spread pages (ahead)
                if current_s + 1 < total_s
                    && let Some(pages) = reader.pages_for_spread(current_s + 1)
                {
                    reader
                        .wanted_scratch
                        .push((pages.primary, to_tier(pages.primary)));
                    if let Some(sec) = pages.secondary {
                        reader.wanted_scratch.push((sec, to_tier(sec)));
                    }
                }
                // 3. Previous spread pages (behind)
                if current_s > 0
                    && let Some(pages) = reader.pages_for_spread(current_s - 1)
                {
                    reader
                        .wanted_scratch
                        .push((pages.primary, to_tier(pages.primary)));
                    if let Some(sec) = pages.secondary {
                        reader.wanted_scratch.push((sec, to_tier(sec)));
                    }
                }
                // 4. Spread + 2
                if current_s + 2 < total_s
                    && let Some(pages) = reader.pages_for_spread(current_s + 2)
                {
                    reader
                        .wanted_scratch
                        .push((pages.primary, to_tier(pages.primary)));
                    if let Some(sec) = pages.secondary {
                        reader.wanted_scratch.push((sec, to_tier(sec)));
                    }
                }
            } else {
                for ix in pagecache::prefetch_order(reader.page, pages_len) {
                    reader.wanted_scratch.push((ix, to_tier(ix)));
                }
            }

            ReaderSyncOutcome {
                animating,
                progress_changed,
            }
        };

        if progress_changed.is_some() {
            self.save_progress_debounced(cx);
        }

        // Textures released by finished slides are freed on the next frame.
        if let Some(reader) = self.reader.as_mut() {
            self.garbage.append(&mut reader.released);
            // Cancel background prefetch tasks for pages that are no longer wanted:
            reader.cancellations.retain(|ix, token| {
                if !reader
                    .wanted_scratch
                    .iter()
                    .any(|(wanted_ix, _)| wanted_ix == ix)
                {
                    token.store(true, Ordering::Relaxed);
                    reader.pending.remove(ix);
                    false
                } else {
                    true
                }
            });
        }
        let mut stack_wanted = [(0usize, 0u32); 32];
        let count = self
            .reader
            .as_ref()
            .map_or(0, |r| r.wanted_scratch.len().min(stack_wanted.len()));
        if let Some(reader) = self.reader.as_ref() {
            stack_wanted[..count].copy_from_slice(&reader.wanted_scratch[..count]);
        }
        for &(ix, tier) in &stack_wanted[..count] {
            self.request_page(ix, tier, cx);
        }
        animating
    }

    pub(crate) fn request_page(&mut self, ix: usize, tier: u32, cx: &mut Context<Self>) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let Some(pages) = reader.pages.as_ref() else {
            return;
        };
        if ix >= pages.len() || reader.pending.contains(&ix) || reader.failed.contains_key(&ix) {
            return;
        }
        if matches!(reader.images.get(&ix), Some(page) if page.tier >= tier) {
            return;
        }
        let name = match pages.get(ix) {
            Some(name) => name.clone(),
            None => return,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        reader.pending.insert(ix);
        reader.cancellations.insert(ix, cancel.clone());

        let book_id = reader.book_id.clone();
        let path = reader.path.clone();
        let kind = reader.kind;
        let cancel_token = cancel.clone();
        let rar_session = reader.rar_session.clone();

        let task = cx.background_executor().spawn(async move {
            if cancel_token.load(Ordering::Relaxed) {
                anyhow::bail!("page decode cancelled");
            }
            let bytes = archive::read_page_with_session(
                &path,
                kind,
                &name,
                rar_session.as_deref(),
                Some(cancel_token.clone()),
            )?;
            if cancel_token.load(Ordering::Relaxed) {
                anyhow::bail!("page decode cancelled");
            }
            let decoded = imaging::decode_fit_bgra(&bytes, tier, tier)?;
            if cancel_token.load(Ordering::Relaxed) {
                anyhow::bail!("page decode cancelled");
            }
            let (width, height) = decoded.image.dimensions();
            anyhow::Ok((
                Arc::new(RenderImage::new(vec![Frame::new(decoded.image)])),
                decoded.orig,
                u64::from(width) * u64::from(height) * 4,
            ))
        });

        let cancel_token_cb = cancel.clone();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this
                .update(cx, |this, cx| {
                    this.page_decoded(book_id, ix, tier, cancel_token_cb, result, cx)
                })
                .ok();
        })
        .detach();
    }

    pub(crate) fn page_decoded(
        &mut self,
        book_id: String,
        ix: usize,
        tier: u32,
        cancel_token: Arc<AtomicBool>,
        result: Result<(Arc<RenderImage>, (u32, u32), u64)>,
        cx: &mut Context<Self>,
    ) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        if reader.book_id != book_id {
            return;
        }

        let is_current = reader
            .cancellations
            .get(&ix)
            .is_some_and(|active| Arc::ptr_eq(active, &cancel_token));

        if is_current {
            reader.pending.remove(&ix);
            reader.cancellations.remove(&ix);
        }

        let was_cancelled = cancel_token.load(Ordering::Relaxed);
        if !is_current || was_cancelled {
            return;
        }

        match result {
            Ok((image, orig, bytes)) => {
                if reader.continuous && ix < reader.page {
                    let width = (reader.viewport.width * reader.zoom).max(1.0);
                    let (old_w, old_h) =
                        reader.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
                    let old_height = width * old_h as f32 / old_w.max(1) as f32;
                    let new_height = width * orig.1 as f32 / orig.0.max(1) as f32;
                    let shift = new_height - old_height;
                    reader.off.1 = (reader.off.1 + shift).max(0.0);
                    reader.scroll_target.1 = (reader.scroll_target.1 + shift).max(0.0);
                }
                let old_class = reader.classify_page(ix);
                if let Some(size) = reader.orig_sizes.get_mut(ix) {
                    *size = Some(orig);
                    reader.layout_version += 1;
                }
                if let Some(pages) = reader.pages.as_ref()
                    && let Some(name) = pages.get(ix)
                    && let Some(book) = self.library.get_mut(&book_id)
                {
                    book.insert_page_dimension(name.clone(), orig);
                }
                if reader.classify_page(ix) != old_class {
                    reader.rebuild_spread_plan();
                }
                if let Some(old) = reader.images.insert(
                    ix,
                    Page {
                        image,
                        orig,
                        tier,
                        bytes,
                    },
                ) {
                    self.garbage.push(old.image);
                }
            }
            Err(err) => {
                let msg = format!("{err:#}");
                if !msg.contains("cancelled") {
                    reader.failed.insert(ix, msg);
                }
            }
        }
        cx.notify();
    }

    pub fn set_page_layout(&mut self, layout: PageLayout, cx: &mut Context<Self>) {
        if self.library.settings().page_layout == layout {
            return;
        }
        self.library.settings_mut().page_layout = layout;
        if let Some(reader) = self.reader.as_mut() {
            reader.page_layout = layout;
            reader.rebuild_spread_plan();
            reader.cancel_transition();
            reader.arrive = Some(Arrive::Start);
            self.show_hud(layout.label(), cx);
        }
        self.save_library_debounced(cx);
        self.wake();
        cx.notify();
    }

    pub fn set_reading_direction(&mut self, dir: ReadingDirection, cx: &mut Context<Self>) {
        if self.library.settings().reading_direction == dir {
            return;
        }
        self.library.settings_mut().reading_direction = dir;
        if let Some(reader) = self.reader.as_mut() {
            reader.reading_direction = dir;
            reader.cancel_transition();
            reader.arrive = Some(Arrive::Start);
            self.show_hud(dir.label(), cx);
        }
        self.save_library_debounced(cx);
        self.wake();
        cx.notify();
    }

    pub fn set_first_page_is_cover(&mut self, first_page_is_cover: bool, cx: &mut Context<Self>) {
        if self.library.settings().first_page_is_cover == first_page_is_cover {
            return;
        }
        self.library.settings_mut().first_page_is_cover = first_page_is_cover;
        if let Some(reader) = self.reader.as_mut() {
            reader.first_page_is_cover = first_page_is_cover;
            reader.rebuild_spread_plan();
            reader.cancel_transition();
            reader.arrive = Some(Arrive::Start);
            self.show_hud(
                if first_page_is_cover {
                    "First Page as Cover: On"
                } else {
                    "First Page as Cover: Off"
                },
                cx,
            );
        }
        self.save_library_debounced(cx);
        self.wake();
        cx.notify();
    }

    pub fn set_spread_ratio_threshold(&mut self, threshold: f32, cx: &mut Context<Self>) {
        let threshold = threshold.clamp(1.0, 3.0);
        if (self.library.settings().spread_ratio_threshold - threshold).abs() < 1e-4 {
            return;
        }
        self.library.settings_mut().spread_ratio_threshold = threshold;
        if let Some(reader) = self.reader.as_mut() {
            reader.spread_ratio_threshold = threshold;
            reader.rebuild_spread_plan();
        }
        self.save_library_debounced(cx);
        self.wake();
        cx.notify();
    }

    pub fn set_page_spread_override(
        &mut self,
        override_mode: yomu::library::PageSpreadOverride,
        cx: &mut Context<Self>,
    ) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let page = reader.page;
        reader.set_spread_override(page, override_mode);
        reader.cancel_transition();
        let book_id = reader.book_id.clone();
        if let Some(pages) = reader.pages.as_ref()
            && let Some(name) = pages.get(page)
            && let Some(book) = self.library.get_mut(&book_id)
        {
            if override_mode == yomu::library::PageSpreadOverride::Auto {
                book.spread_overrides.remove(name);
            } else {
                book.spread_overrides.insert(name.clone(), override_mode);
            }
        }
        let msg = match override_mode {
            yomu::library::PageSpreadOverride::Auto => "Spread: Auto Detection",
            yomu::library::PageSpreadOverride::ForceSpread => "Spread: Force Spread",
            yomu::library::PageSpreadOverride::ForceSingle => "Spread: Force Single",
        };
        self.show_hud(msg, cx);
        self.save_library_debounced(cx);
        self.wake();
        cx.notify();
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn jemalloc_purge() {
    use tikv_jemalloc_ctl::{background_thread, epoch};
    let _ = epoch::mib().and_then(|e| e.advance());
    let _ = background_thread::write(true);
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn jemalloc_purge() {}
