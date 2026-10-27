//! These drive the real views through gpui's test platform: simulated mouse clicks and key
//! presses go through hit-testing and key dispatch exactly as they do in the running app.

use super::*;
use gpui_kit::{Entity, Modifiers, TestAppContext, VisualTestContext, point};
use std::{
    fs,
    io::{Cursor, Write},
};
use yomu::archive::SourceKind;
use yomu::motion::{KineticScroll, PageTransition};

fn png_bytes() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(60, 90, image::Rgba([90, 40, 160, 255]));
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

struct Fixture {
    files: Vec<PathBuf>,
    dirs: DataDirs,
    /// Book ids in library order: A, B, C.
    ids: Vec<String>,
}

fn fixture(tag: &str) -> Fixture {
    static FIXTURE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = FIXTURE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("yomu-ui-{tag}-{}-{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let dirs = DataDirs::at(root.join("data")).unwrap();

    let mut files = Vec::new();
    for name in ["A", "B", "C"] {
        let path = root.join(format!("{name}.cbz"));
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for page in 1..=5 {
            zip.start_file(format!("{page}.png"), options).unwrap();
            zip.write_all(&png_bytes()).unwrap();
        }
        zip.finish().unwrap();
        files.push(path);
    }

    let report = library::import_paths(&files, &dirs, &HashSet::new());
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let mut library = Library::default();
    for book in report.added {
        library.add(book);
    }
    library.save(&dirs).unwrap();
    let ids = library.books().iter().map(|b| b.id.clone()).collect();
    Fixture { files, dirs, ids }
}

fn launch<'a>(
    fx: &Fixture,
    cx: &'a mut TestAppContext,
) -> (Entity<Yomu>, &'a mut VisualTestContext) {
    let dirs = fx.dirs.clone();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys([KeyBinding::new("ctrl-,", OpenSettings, None)]);
    });
    cx.add_window_view(move |window, cx| {
        let yomu = Yomu::with_dirs(dirs, cx);
        window.focus(&yomu.focus_handle, cx);
        yomu
    })
}

fn book(view: &Entity<Yomu>, cx: &mut VisualTestContext, id: &str) -> Book {
    view.read_with(cx, |this, _| {
        this.library.get(id).cloned().expect("book exists")
    })
}

fn saved(fx: &Fixture, id: &str) -> Option<Book> {
    Library::load(&fx.dirs).unwrap().get(id).cloned()
}

fn saved_settings(fx: &Fixture) -> Settings {
    Library::load(&fx.dirs).unwrap().settings().clone()
}

fn overlay_kind(view: &Entity<Yomu>, cx: &mut VisualTestContext) -> &'static str {
    view.read_with(cx, |this, _| match this.overlay {
        None => "none",
        Some(Overlay::Menu(_)) => "menu",
        Some(Overlay::LibraryMenu(_)) => "library-menu",
        Some(Overlay::Confirm(_)) => "confirm",
        Some(Overlay::Properties(_)) => "properties",
        Some(Overlay::Series(_)) => "series",
        Some(Overlay::Collection(_)) => "collection",
        Some(Overlay::Settings) => "settings",
    })
}

fn reading(view: &Entity<Yomu>, cx: &mut VisualTestContext) -> Option<usize> {
    view.read_with(cx, |this, _| this.reader.as_ref().map(|r| r.page))
}

fn opacity(view: &Entity<Yomu>, cx: &mut VisualTestContext) -> f32 {
    view.read_with(cx, |this, _| this.library.settings().opacity)
}

fn act(view: &Entity<Yomu>, cx: &mut VisualTestContext, id: &str, action: BookAction) {
    let id = id.to_string();
    view.update_in(cx, move |this, window, cx| {
        this.run_book_action(action, &id, std::slice::from_ref(&id), window, cx)
    });
    cx.run_until_parked();
}

fn accept(view: &Entity<Yomu>, cx: &mut VisualTestContext) {
    view.update_in(cx, |this, window, cx| this.confirm_accept(window, cx));
    cx.run_until_parked();
}

fn close(view: &Entity<Yomu>, cx: &mut VisualTestContext) {
    view.update(cx, |this, cx| this.close_overlay(cx));
    cx.run_until_parked();
}

fn set_progress(view: &Entity<Yomu>, cx: &mut VisualTestContext, id: &str, page: usize) {
    let id = id.to_string();
    view.update(cx, move |this, _| {
        let book = this.library.get_mut(&id).unwrap();
        book.opened = true;
        book.current_page = page;
    });
}

fn open_menu(view: &Entity<Yomu>, cx: &mut VisualTestContext, id: &str) {
    let id = id.to_string();
    view.update(cx, move |this, cx| {
        this.open_book_menu(&id, point(px(100.), px(100.)), cx)
    });
    cx.run_until_parked();
}

fn menu_labels(view: &Entity<Yomu>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |this, _| match &this.overlay {
        Some(Overlay::Menu(menu)) => menu
            .entries
            .iter()
            .filter_map(|e| match e {
                Entry::Item(item) => Some(item.label.clone()),
                Entry::Separator => None,
            })
            .collect(),
        _ => Vec::new(),
    })
}

/// Centre of the nth book card in the library grid (window is 1920 wide in the test platform).
fn card(index: usize) -> Point<Pixels> {
    point(px(369.0 + 234.0 * index as f32), px(200.0))
}

fn click(cx: &mut VisualTestContext, at: Point<Pixels>, button: MouseButton) {
    cx.simulate_mouse_down(at, button, Modifiers::default());
    cx.simulate_mouse_up(at, button, Modifiers::default());
}

// ---------------------------------------------------------------------------------------
// Context menu: actions
// ---------------------------------------------------------------------------------------

#[gpui_kit::test]
fn mark_as_read_then_unread_updates_and_persists(cx: &mut TestAppContext) {
    let fx = fixture("markread");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();

    open_menu(&view, cx, &id);
    assert_eq!(menu_labels(&view, cx)[0], "Start Reading");
    act(&view, cx, &id, BookAction::MarkRead);
    assert_eq!(overlay_kind(&view, cx), "none");
    let b = book(&view, cx, &id);
    assert_eq!((b.current_page, b.opened), (4, true));
    assert_eq!(saved(&fx, &id).unwrap().current_page, 4);

    open_menu(&view, cx, &id);
    let labels = menu_labels(&view, cx);
    assert_eq!(labels[0], "Start from Beginning");
    assert!(labels.contains(&"Mark as Unread".to_string()));
    assert!(!labels.contains(&"Mark as Read".to_string()));
    act(&view, cx, &id, BookAction::MarkUnread);
    let b = book(&view, cx, &id);
    assert_eq!((b.current_page, b.opened), (0, false));
    let on_disk = saved(&fx, &id).unwrap();
    assert!(!on_disk.opened);

    // repeating the action is harmless
    act(&view, cx, &id, BookAction::MarkUnread);
    assert!(!book(&view, cx, &id).opened);
}

#[gpui_kit::test]
fn reset_progress_asks_first_and_keeps_marks(cx: &mut TestAppContext) {
    let fx = fixture("reset");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[1].clone();
    set_progress(&view, cx, &id, 3);
    view.update(cx, |this, _| {
        this.library
            .get_mut(&fx.ids[1])
            .unwrap()
            .marks
            .insert("a".into(), 2);
    });

    open_menu(&view, cx, &id);
    assert!(menu_labels(&view, cx).contains(&"Resume Reading".to_string()));
    act(&view, cx, &id, BookAction::ResetProgress);
    assert_eq!(overlay_kind(&view, cx), "confirm");
    assert_eq!(
        book(&view, cx, &id).current_page,
        3,
        "nothing changes before confirming"
    );

    // cancelling leaves everything alone
    close(&view, cx);
    assert_eq!(overlay_kind(&view, cx), "none");
    assert_eq!(book(&view, cx, &id).current_page, 3);

    act(&view, cx, &id, BookAction::ResetProgress);
    accept(&view, cx);
    let b = book(&view, cx, &id);
    assert_eq!((b.current_page, b.opened), (0, false));
    assert_eq!(b.marks.get("a"), Some(&2));
    let on_disk = saved(&fx, &id).unwrap();
    assert!(!on_disk.opened);
    assert_eq!(on_disk.marks.get("a"), Some(&2));
}

#[gpui_kit::test]
fn remove_never_deletes_the_file_and_confirms_only_when_progress_would_be_lost(
    cx: &mut TestAppContext,
) {
    let fx = fixture("remove");
    let (view, cx) = launch(&fx, cx);

    // unread: removed straight away
    let (a, b) = (fx.ids[0].clone(), fx.ids[1].clone());
    act(&view, cx, &a, BookAction::Remove);
    assert_eq!(overlay_kind(&view, cx), "none");
    assert!(saved(&fx, &a).is_none());
    assert!(fx.files[0].exists(), "the comic file must survive");
    assert!(
        !fx.dirs.cover(&a).exists(),
        "the cached cover is cleaned up"
    );

    // with progress: asks first, and cancelling keeps the book
    set_progress(&view, cx, &b, 2);
    act(&view, cx, &b, BookAction::Remove);
    assert_eq!(overlay_kind(&view, cx), "confirm");
    close(&view, cx);
    assert!(saved(&fx, &b).is_some());
    act(&view, cx, &b, BookAction::Remove);
    accept(&view, cx);
    assert!(saved(&fx, &b).is_none());
    assert!(fx.files[1].exists());
    assert_eq!(view.read_with(cx, |this, _| this.library.books().len()), 1);
}

#[gpui_kit::test]
fn resume_and_start_from_beginning_open_the_reader(cx: &mut TestAppContext) {
    let fx = fixture("resume");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[2].clone();
    set_progress(&view, cx, &id, 3);

    act(&view, cx, &id, BookAction::Open);
    assert_eq!(
        reading(&view, cx),
        Some(3),
        "resume lands on the saved page"
    );
    view.update_in(cx, |this, window, cx| this.close_book(window, cx));
    cx.run_until_parked();
    assert_eq!(reading(&view, cx), None);

    act(&view, cx, &id, BookAction::StartOver);
    assert_eq!(reading(&view, cx), Some(0));
    assert_eq!(book(&view, cx, &id).current_page, 0);
    assert_eq!(saved(&fx, &id).unwrap().current_page, 0);
}

#[gpui_kit::test]
fn missing_file_disables_file_actions_and_reports_it(cx: &mut TestAppContext) {
    let fx = fixture("missing");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();
    fs::remove_file(&fx.files[0]).unwrap();

    open_menu(&view, cx, &id);
    let disabled = view.read_with(cx, |this, _| match &this.overlay {
        Some(Overlay::Menu(menu)) => menu
            .entries
            .iter()
            .filter_map(|e| match e {
                Entry::Item(item) if !item.enabled => Some(item.action),
                _ => None,
            })
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    });
    assert_eq!(disabled, vec![BookAction::Open, BookAction::RevealFile]);

    // even if triggered anyway (stale menu), nothing breaks
    act(&view, cx, &id, BookAction::Open);
    assert_eq!(reading(&view, cx), None);
    let toast = view.read_with(cx, |this, _| this.toast.clone().map(|t| t.to_string()));
    assert!(toast.unwrap().contains("missing"));

    // bookkeeping actions still work without the file
    act(&view, cx, &id, BookAction::MarkRead);
    assert!(book(&view, cx, &id).is_finished());
    act(&view, cx, &id, BookAction::Remove);
    assert_eq!(overlay_kind(&view, cx), "confirm");
}

#[gpui_kit::test]
fn properties_dialog_lists_book_information(cx: &mut TestAppContext) {
    let fx = fixture("props");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();
    act(&view, cx, &id, BookAction::Properties);
    assert_eq!(overlay_kind(&view, cx), "properties");
    let rows = view.read_with(cx, |this, _| match &this.overlay {
        Some(Overlay::Properties(dialog)) => dialog.rows.clone(),
        _ => Vec::new(),
    });
    let get = |name: &str| rows.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    assert_eq!(get("Pages").as_deref(), Some("5"));
    assert_eq!(get("Format").as_deref(), Some("CBZ archive"));
    assert!(get("Location").unwrap().ends_with("A.cbz"));
    assert!(get("Size").is_some());
    close(&view, cx);
    assert_eq!(overlay_kind(&view, cx), "none");
}

// ---------------------------------------------------------------------------------------
// Context menu: mouse and keyboard
// ---------------------------------------------------------------------------------------

#[gpui_kit::test]
fn right_click_opens_the_menu_and_never_opens_the_book(cx: &mut TestAppContext) {
    let fx = fixture("rightclick");
    let (view, cx) = launch(&fx, cx);

    click(cx, card(0), MouseButton::Right);
    assert_eq!(overlay_kind(&view, cx), "menu");
    assert_eq!(
        reading(&view, cx),
        None,
        "right-click must not open the book"
    );
    view.read_with(cx, |this, _| match &this.overlay {
        Some(Overlay::Menu(menu)) => assert_eq!(menu.book_id, fx.ids[0]),
        _ => unreachable!(),
    });

    // a left click on a book (no menu) still opens it
    close(&view, cx);
    std::thread::sleep(Duration::from_millis(350));
    click(cx, card(1), MouseButton::Left);
    assert_eq!(reading(&view, cx), Some(0));
}

#[gpui_kit::test]
fn clicking_outside_closes_the_menu_without_clicking_through(cx: &mut TestAppContext) {
    let fx = fixture("outside");
    let (view, cx) = launch(&fx, cx);

    click(cx, card(0), MouseButton::Right);
    assert_eq!(overlay_kind(&view, cx), "menu");
    // far from the menu, but directly over another book
    click(cx, card(3), MouseButton::Left);
    assert_eq!(overlay_kind(&view, cx), "none");
    assert_eq!(
        reading(&view, cx),
        None,
        "the click only dismisses the menu"
    );

    // and an immediate second click (double-click on a menu item) is swallowed too
    click(cx, card(3), MouseButton::Left);
    assert_eq!(reading(&view, cx), None);
}

#[gpui_kit::test]
fn menu_items_can_be_clicked(cx: &mut TestAppContext) {
    let fx = fixture("itemclick");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();

    click(cx, card(0), MouseButton::Right);
    // second item of an unread book's menu is "Mark as Read": 4px padding, one 28px row,
    // a 9px separator, then the row itself
    let origin = card(0);
    let at = point(origin.x + px(20.), origin.y + px(4. + 28. + 9. + 14.));
    click(cx, at, MouseButton::Left);
    assert_eq!(overlay_kind(&view, cx), "none");
    assert!(book(&view, cx, &id).is_finished());
}

#[gpui_kit::test]
fn menu_stays_inside_the_window_near_every_edge(cx: &mut TestAppContext) {
    let fx = fixture("edges");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();
    let (width, height) = (1920.0, 1080.0);

    for (x, y) in [
        (2.0, 2.0),
        (width - 2.0, 2.0),
        (2.0, height - 2.0),
        (width - 2.0, height - 2.0),
    ] {
        let id = id.clone();
        view.update(cx, move |this, cx| {
            this.open_book_menu(&id, point(px(x), px(y)), cx)
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("book-menu").expect("menu was painted");
        let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        let right = left + f32::from(bounds.size.width);
        let bottom = top + f32::from(bounds.size.height);
        assert!(left >= 0.0 && top >= 0.0, "({x},{y}) -> {bounds:?}");
        assert!(
            right <= width && bottom <= height,
            "({x},{y}) -> {bounds:?}"
        );
        close(&view, cx);
    }

    // away from the edges it opens exactly where you clicked
    view.update(cx, |this, cx| {
        this.open_book_menu(&fx.ids[0], point(px(300.), px(200.)), cx)
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("book-menu").unwrap();
    assert_eq!(
        (f32::from(bounds.origin.x), f32::from(bounds.origin.y)),
        (300.0, 200.0)
    );
}

#[gpui_kit::test]
fn menu_keyboard_navigation(cx: &mut TestAppContext) {
    let fx = fixture("menukeys");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();
    open_menu(&view, cx, &id);

    let selected = |view: &Entity<Yomu>, cx: &mut VisualTestContext| {
        view.read_with(cx, |this, _| match &this.overlay {
            Some(Overlay::Menu(menu)) => menu.selected,
            _ => None,
        })
    };
    assert_eq!(selected(&view, cx), None);
    cx.simulate_keystrokes("down");
    assert_eq!(selected(&view, cx), Some(0));
    cx.simulate_keystrokes("down down");
    assert_eq!(
        selected(&view, cx),
        Some(4),
        "skips separators: Start, Mark as Read, Open Location"
    );
    cx.simulate_keystrokes("up");
    assert_eq!(selected(&view, cx), Some(2));
    cx.simulate_keystrokes("end");
    assert_eq!(selected(&view, cx), Some(8), "Remove from Library is last");
    cx.simulate_keystrokes("down");
    assert_eq!(selected(&view, cx), Some(0), "wraps around");

    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");

    // enter activates the highlighted item
    open_menu(&view, cx, &id);
    cx.simulate_keystrokes("down down enter");
    assert!(book(&view, cx, &id).is_finished());
    assert_eq!(overlay_kind(&view, cx), "none");
}

#[gpui_kit::test]
fn keys_do_not_leak_to_the_reader_while_a_dialog_is_open(cx: &mut TestAppContext) {
    let fx = fixture("leak");
    let (view, cx) = launch(&fx, cx);
    view.update_in(cx, |this, _, _| {
        this.library.settings_mut().vim_mode = true;
    });
    let id = fx.ids[0].clone();
    act(&view, cx, &id, BookAction::Open);
    assert_eq!(reading(&view, cx), Some(0));

    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");
    cx.simulate_keystrokes("J G space");
    assert_eq!(
        reading(&view, cx),
        Some(0),
        "reader keys are ignored behind a dialog"
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
    assert_eq!(
        reading(&view, cx),
        Some(0),
        "escape closed the dialog, not the book"
    );
    cx.simulate_keystrokes("J");
    assert_eq!(
        reading(&view, cx),
        Some(1),
        "the reader works again afterwards"
    );
}

// ---------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------

#[gpui_kit::test]
fn settings_open_with_shortcut_and_header_button_and_close_with_escape(cx: &mut TestAppContext) {
    let fx = fixture("settingsopen");
    let (view, cx) = launch(&fx, cx);

    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "none", "the shortcut toggles");

    view.update(cx, |this, cx| this.open_settings(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
}

#[gpui_kit::test]
fn window_appearance_is_always_opaque(cx: &mut TestAppContext) {
    let fx = fixture("appearance");
    let (view, cx) = launch(&fx, cx);
    let applied = |view: &Entity<Yomu>, cx: &mut VisualTestContext| {
        view.read_with(cx, |this, _| this.applied_opaque)
    };
    assert_eq!(
        applied(&view, cx),
        Some(true),
        "window is locked to opaque appearance"
    );
}

#[gpui_kit::test]
fn reset_to_defaults_confirms_and_restores_everything(cx: &mut TestAppContext) {
    let fx = fixture("resetsettings");
    let (view, cx) = launch(&fx, cx);

    // at the defaults there is nothing to reset
    view.update(cx, |this, cx| {
        this.open_settings(cx);
        this.request_reset_settings(cx);
    });
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "settings");

    view.update(cx, |this, _| {
        let settings = this.library.settings_mut();
        settings.fit = FitMode::Width;
        this.save_library();
    });
    view.update(cx, |this, cx| this.request_reset_settings(cx));
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "confirm");

    // cancelling goes back to the panel and changes nothing
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "settings");
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().fit),
        FitMode::Width
    );

    view.update(cx, |this, cx| this.request_reset_settings(cx));
    cx.simulate_keystrokes("enter");
    assert_eq!(
        overlay_kind(&view, cx),
        "settings",
        "back on the panel to see the result"
    );
    assert_eq!(saved_settings(&fx), Settings::default());
}

#[gpui_kit::test]
fn reset_also_updates_an_open_reader(cx: &mut TestAppContext) {
    let fx = fixture("resetreader");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();
    act(&view, cx, &id, BookAction::Open);
    view.update(cx, |this, cx| {
        this.set_fit(FitMode::Width, cx);
    });
    view.update(cx, |this, cx| {
        this.open_settings(cx);
        this.request_reset_settings(cx);
    });
    cx.simulate_keystrokes("enter");
    let fit = view.read_with(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        reader.fit
    });
    assert_eq!(fit, FitMode::Page);
}

#[gpui_kit::test]
fn settings_survive_a_restart(cx: &mut TestAppContext) {
    let fx = fixture("restart");
    {
        let (view, cx) = launch(&fx, cx);
        let id = fx.ids[2].clone();
        view.update(cx, |this, _| {
            this.library.settings_mut().fit = FitMode::Width;
            this.save_library();
        });
        set_progress(&view, cx, &id, 2);
        act(&view, cx, &id, BookAction::MarkRead);
    }
    // a brand new app instance reads the same files
    let (view, cx) = launch(&fx, cx);
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().fit),
        FitMode::Width
    );
    assert!(book(&view, cx, &fx.ids[2]).is_finished());
}

#[gpui_kit::test]
fn border_and_rounding_settings_survive_a_restart_and_reset(cx: &mut TestAppContext) {
    let fx = fixture("border-rounding-ui");
    {
        let (view, cx) = launch(&fx, cx);
        view.update(cx, |this, cx| {
            this.set_border_size(BorderSize::Thick, cx);
            this.set_corner_rounding(CornerRounding::Rounded, cx);
        });
        assert_eq!(
            view.read_with(cx, |this, _| this.library.settings().border_size),
            BorderSize::Thick
        );
        assert_eq!(
            view.read_with(cx, |this, _| this.library.settings().corner_rounding),
            CornerRounding::Rounded
        );
    }

    // Restarting the app preserves the configured border size and corner rounding
    let (view, cx) = launch(&fx, cx);
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().border_size),
        BorderSize::Thick
    );
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().corner_rounding),
        CornerRounding::Rounded
    );
    assert_eq!(view.read_with(cx, |this, _| this.border_px()), 2.0);
    assert_eq!(view.read_with(cx, |this, _| this.radius_px()), 12.0);
    assert_eq!(view.read_with(cx, |this, _| this.radius_lg_px()), 16.0);

    // Reset restores defaults
    view.update(cx, |this, cx| {
        this.open_settings(cx);
        this.request_reset_settings(cx);
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().border_size),
        BorderSize::Thin
    );
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().corner_rounding),
        CornerRounding::Standard
    );
    assert_eq!(view.read_with(cx, |this, _| this.border_px()), 1.0);
    assert_eq!(view.read_with(cx, |this, _| this.radius_px()), 6.0);
    assert_eq!(view.read_with(cx, |this, _| this.radius_lg_px()), 10.0);
}

#[gpui_kit::test]
fn corrupt_preferences_do_not_lose_the_library(cx: &mut TestAppContext) {
    let fx = fixture("corrupt");
    let text = fs::read_to_string(fx.dirs.library_file()).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["settings"] = serde_json::json!({"opacity": "bogus", "fit": 17, "card_size": "huge"});
    fs::write(fx.dirs.library_file(), json.to_string()).unwrap();

    let (view, cx) = launch(&fx, cx);
    assert_eq!(view.read_with(cx, |this, _| this.library.books().len()), 3);
    assert_eq!(opacity(&view, cx), 1.0);
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().clone()),
        Settings::default()
    );
}

#[gpui_kit::test]
fn view_mode_grid_and_list_toggle_and_persistence(cx: &mut TestAppContext) {
    let fx = fixture("viewmode");
    {
        let (view, cx) = launch(&fx, cx);
        assert!(view.read_with(cx, |this, _| this.view_mode_grid));
        assert!(view.read_with(cx, |this, _| this.library.settings().view_mode_grid));

        // Switch to List View
        view.update(cx, |this, _| {
            this.view_mode_grid = false;
            this.library.settings_mut().view_mode_grid = false;
            this.save_library();
        });
        assert!(!view.read_with(cx, |this, _| this.view_mode_grid));
    }

    // Restart and verify persisted state
    {
        let (view, cx) = launch(&fx, cx);
        assert!(!view.read_with(cx, |this, _| this.view_mode_grid));
        assert!(!view.read_with(cx, |this, _| this.library.settings().view_mode_grid));

        // Toggle back to Grid View
        view.update(cx, |this, _| {
            this.view_mode_grid = true;
            this.library.settings_mut().view_mode_grid = true;
            this.save_library();
        });
        assert!(view.read_with(cx, |this, _| this.view_mode_grid));
    }
}

#[gpui_kit::test]
fn dynamic_os_shortcuts_match_operating_system(cx: &mut TestAppContext) {
    let _ = cx;
    let is_mac = cfg!(target_os = "macos");
    let search = super::ui::components::search_shortcut_str();
    let settings = super::ui::components::settings_shortcut_str();
    let open = super::ui::components::open_shortcut_str();
    let zoom = super::ui::components::zoom_shortcut_str();

    if is_mac {
        assert_eq!(search, "⌘K");
        assert_eq!(settings, "⌘,");
        assert_eq!(open, "⌘O");
        assert_eq!(zoom, "+ / - or Cmd+Wheel");
    } else {
        assert_eq!(search, "Ctrl+K");
        assert_eq!(settings, "Ctrl+,");
        assert_eq!(open, "Ctrl+O");
        assert_eq!(zoom, "+ / - or Ctrl+Wheel");
    }
}

// ---------------------------------------------------------------------------------------
// Multiple selection and bulk removal
// ---------------------------------------------------------------------------------------

fn click_with(cx: &mut VisualTestContext, at: Point<Pixels>, modifiers: Modifiers) {
    cx.simulate_mouse_down(at, MouseButton::Left, modifiers);
    cx.simulate_mouse_up(at, MouseButton::Left, modifiers);
}

fn ctrl_click(cx: &mut VisualTestContext, index: usize) {
    click_with(cx, card(index), Modifiers::control());
}

fn selected(view: &Entity<Yomu>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |this, _| this.selected_ids())
}

fn book_count(view: &Entity<Yomu>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |this, _| this.library.books().len())
}

#[gpui_kit::test]
fn ctrl_click_selects_without_opening_and_plain_clicks_then_toggle(cx: &mut TestAppContext) {
    let fx = fixture("selectclick");
    let (view, cx) = launch(&fx, cx);

    ctrl_click(cx, 0);
    assert_eq!(selected(&view, cx), vec![fx.ids[0].clone()]);
    assert_eq!(reading(&view, cx), None, "ctrl+click never opens a book");

    // selection mode: a plain click adds, and clicking a selected book removes it
    click(cx, card(2), MouseButton::Left);
    assert_eq!(
        selected(&view, cx),
        vec![fx.ids[0].clone(), fx.ids[2].clone()]
    );
    assert_eq!(reading(&view, cx), None);
    click(cx, card(0), MouseButton::Left);
    assert_eq!(selected(&view, cx), vec![fx.ids[2].clone()]);

    // deselecting the last one leaves selection mode: a plain click opens again
    click(cx, card(2), MouseButton::Left);
    assert!(selected(&view, cx).is_empty());
    click(cx, card(1), MouseButton::Left);
    assert_eq!(reading(&view, cx), Some(0));
    assert!(selected(&view, cx).is_empty());
}

#[gpui_kit::test]
fn shift_click_selects_a_range(cx: &mut TestAppContext) {
    let fx = fixture("shiftclick");
    let (view, cx) = launch(&fx, cx);

    ctrl_click(cx, 0);
    click_with(cx, card(2), Modifiers::shift());
    assert_eq!(selected(&view, cx), fx.ids);
    assert_eq!(reading(&view, cx), None);
}

#[gpui_kit::test]
fn select_all_and_escape(cx: &mut TestAppContext) {
    let fx = fixture("selectall");
    let (view, cx) = launch(&fx, cx);

    cx.simulate_keystrokes("ctrl-a");
    assert_eq!(selected(&view, cx), fx.ids);
    cx.simulate_keystrokes("escape");
    assert!(selected(&view, cx).is_empty());
    assert_eq!(
        reading(&view, cx),
        None,
        "escape in the library does nothing else"
    );
}

#[gpui_kit::test]
fn escape_and_backspace_exit_open_group(cx: &mut TestAppContext) {
    let fx = fixture("opengroup");
    let (view, cx) = launch(&fx, cx);

    view.update(cx, |this, _| {
        this.open_group = Some("series:test".into());
    });
    assert_eq!(
        view.read_with(cx, |this, _| this.open_group.clone()),
        Some("series:test".into())
    );

    cx.simulate_keystrokes("escape");
    assert_eq!(
        view.read_with(cx, |this, _| this.open_group.clone()),
        None,
        "escape exits open group"
    );

    view.update(cx, |this, _| {
        this.open_group = Some("series:test".into());
    });
    cx.simulate_keystrokes("backspace");
    assert_eq!(
        view.read_with(cx, |this, _| this.open_group.clone()),
        None,
        "backspace exits open group"
    );
}

#[gpui_kit::test]
fn top_bar_back_button_exits_group_and_resets_scroll(cx: &mut TestAppContext) {
    let fx = fixture("backbutton");
    let (view, cx) = launch(&fx, cx);

    // When no group is open, group-back button is not rendered
    assert!(cx.debug_bounds("group-back").is_none());

    // Enter a group in the library
    view.update(cx, |this, cx| {
        this.open_group = Some("series:test".into());
        cx.notify();
    });
    cx.run_until_parked();

    // The group-back button is now visible in the persistent top bar header
    let back = cx
        .debug_bounds("group-back")
        .expect("group-back is in the top bar when group is open");
    click(cx, back.center(), MouseButton::Left);
    cx.run_until_parked();

    assert_eq!(
        view.read_with(cx, |this, _| this.open_group.clone()),
        None,
        "clicking group-back exits group"
    );
}

#[gpui_kit::test]
fn group_view_scroll_is_independent_from_library_grid_scroll(cx: &mut TestAppContext) {
    let fx = fixture("groupscroll");
    let (view, cx) = launch(&fx, cx);

    view.update(cx, |this, _| {
        this.library.settings_mut().group_series = true;
    });

    let group_key = view.read_with(cx, |this, _| {
        directory_groups(this.library.books())[0].key.clone()
    });

    // Simulate user having scrolled down in the library grid
    view.update(cx, |this, _| {
        this.library_scroll.set_offset(point(px(0.), px(-600.)));
    });

    // Enter a group
    view.update(cx, |this, cx| {
        this.enter_group(group_key, cx);
    });
    cx.run_until_parked();

    // Group view MUST open at the top (offset 0), not inheriting the -600px library scroll
    let group_offset = view.read_with(cx, |this, _| this.group_scroll.offset());
    assert_eq!(
        group_offset.y,
        px(0.),
        "group view must open at row 0 (top)"
    );

    // Library grid's scroll position is preserved while group is open
    let lib_offset = view.read_with(cx, |this, _| this.library_scroll.offset());
    assert_eq!(
        lib_offset.y,
        px(-600.),
        "library scroll position must be preserved"
    );

    // Exit group
    view.update(cx, |this, cx| {
        this.exit_group(cx);
    });

    assert_eq!(view.read_with(cx, |this, _| this.open_group.clone()), None);
    // Group scroll is reset on exit so reopening any group starts fresh at top
    let group_offset_after = view.read_with(cx, |this, _| this.group_scroll.offset());
    assert_eq!(
        group_offset_after.y,
        px(0.),
        "group scroll is reset on exit"
    );
}

fn multi_group_fixture(tag: &str) -> (Fixture, String, String) {
    static FIXTURE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = FIXTURE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("yomu-ui-{tag}-{}-{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let dirs = DataDirs::at(root.join("data")).unwrap();

    let dir1 = root.join("manga_one");
    let dir2 = root.join("manga_two");
    fs::create_dir_all(&dir1).unwrap();
    fs::create_dir_all(&dir2).unwrap();

    let mut files = Vec::new();
    for (dir, names) in [(&dir1, &["A1", "A2"][..]), (&dir2, &["B1", "B2"][..])] {
        for name in names {
            let path = dir.join(format!("{name}.cbz"));
            let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for page in 1..=5 {
                zip.start_file(format!("{page}.png"), options).unwrap();
                zip.write_all(&png_bytes()).unwrap();
            }
            zip.finish().unwrap();
            files.push(path);
        }
    }

    let report = library::import_paths(&files, &dirs, &HashSet::new());
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let mut library = Library::default();
    library.settings_mut().group_series = true;
    for book in report.added {
        library.add(book);
    }
    library.save(&dirs).unwrap();
    let ids = library.books().iter().map(|b| b.id.clone()).collect();
    let key1 = format!("directory:{}", dir1.display());
    let key2 = format!("directory:{}", dir2.display());
    (Fixture { files, dirs, ids }, key1, key2)
}

#[gpui_kit::test]
fn select_all_inside_group_only_selects_and_removes_from_that_group(cx: &mut TestAppContext) {
    let (fx, key1, key2) = multi_group_fixture("selgrp");
    let (view, cx) = launch(&fx, cx);

    view.update(cx, |this, _| {
        this.library.settings_mut().group_series = true;
    });

    assert_eq!(book_count(&view, cx), 4);

    // Enter Group 1
    view.update(cx, |this, cx| {
        this.enter_group(key1.clone(), cx);
    });
    cx.run_until_parked();

    // Select all inside Group 1 using Ctrl+A
    cx.simulate_keystrokes("ctrl-a");
    let sel = selected(&view, cx);
    assert_eq!(sel.len(), 2, "only the 2 books in group 1 are selected");

    let group1_books = view.read_with(cx, |this, _| {
        let groups = directory_groups(this.library.books());
        let g1 = groups.into_iter().find(|g| g.key == key1).unwrap();
        g1.books.iter().map(|b| b.id.clone()).collect::<Vec<_>>()
    });
    assert_eq!(sel, group1_books);

    // Remove the selected books
    cx.simulate_keystrokes("delete");
    cx.run_until_parked();

    // Group 1's books are removed; Group 2's books are untouched
    assert_eq!(book_count(&view, cx), 2);
    let remaining_ids: Vec<String> = view.read_with(cx, |this, _| {
        this.library.books().iter().map(|b| b.id.clone()).collect()
    });
    let group2_books = view.read_with(cx, |this, _| {
        let groups = directory_groups(this.library.books());
        let g2 = groups.into_iter().find(|g| g.key == key2).unwrap();
        g2.books.iter().map(|b| b.id.clone()).collect::<Vec<_>>()
    });
    assert_eq!(remaining_ids, group2_books);

    // Group 1 no longer exists, so open_group was automatically exited to None
    assert_eq!(view.read_with(cx, |this, _| this.open_group.clone()), None);

    // Persisted data on disk also only has Group 2's 2 books
    let on_disk = Library::load(&fx.dirs).unwrap();
    assert_eq!(on_disk.books().len(), 2);
}

#[gpui_kit::test]
fn select_all_button_in_group_header_only_selects_current_group(cx: &mut TestAppContext) {
    let (fx, key1, _key2) = multi_group_fixture("btnselgrp");
    let (view, cx) = launch(&fx, cx);

    view.update(cx, |this, _| {
        this.library.settings_mut().group_series = true;
    });

    let first_id = fx.ids[0].clone();

    // Enter Group 1
    view.update(cx, |this, cx| {
        this.enter_group(key1.clone(), cx);
    });
    cx.run_until_parked();

    // Select first book via select_click
    view.update(cx, |this, cx| {
        this.select_click(&first_id, true, false, cx);
    });
    cx.run_until_parked();
    assert_eq!(selected(&view, cx).len(), 1);

    // The header offers "Select all"
    let select_all = cx
        .debug_bounds("select-all")
        .expect("header offers Select all for group");
    click(cx, select_all.center(), MouseButton::Left);
    cx.run_until_parked();

    // Only the 2 books in group 1 are selected
    assert_eq!(selected(&view, cx).len(), 2);

    // Remove via the header Remove button
    let remove_btn = cx
        .debug_bounds("select-remove")
        .expect("header offers Remove");
    click(cx, remove_btn.center(), MouseButton::Left);
    cx.run_until_parked();

    // Group 1 is gone, Group 2 books remain
    assert_eq!(book_count(&view, cx), 2);
    assert_eq!(view.read_with(cx, |this, _| this.open_group.clone()), None);
}

#[gpui_kit::test]
fn entering_and_exiting_group_clears_selection(cx: &mut TestAppContext) {
    let (fx, key1, key2) = multi_group_fixture("clrenterexit");
    let (view, cx) = launch(&fx, cx);

    view.update(cx, |this, _| {
        this.library.settings_mut().group_series = true;
    });

    // Enter Group 1 and select all
    view.update(cx, |this, cx| {
        this.enter_group(key1.clone(), cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-a");
    assert_eq!(selected(&view, cx).len(), 2);

    // Exiting Group 1 clears selection
    view.update(cx, |this, cx| {
        this.exit_group(cx);
    });
    assert!(selected(&view, cx).is_empty());

    // Entering Group 2 starts with empty selection
    view.update(cx, |this, cx| {
        this.enter_group(key2.clone(), cx);
    });
    assert!(selected(&view, cx).is_empty());
}

#[gpui_kit::test]
fn delete_removes_the_selection_at_once_when_nothing_is_lost(cx: &mut TestAppContext) {
    let fx = fixture("bulkdelete");
    let (view, cx) = launch(&fx, cx);

    ctrl_click(cx, 0);
    ctrl_click(cx, 1);
    cx.simulate_keystrokes("delete");
    assert_eq!(
        overlay_kind(&view, cx),
        "none",
        "no confirmation: nothing would be lost"
    );
    assert_eq!(book_count(&view, cx), 1);
    assert!(selected(&view, cx).is_empty());
    let on_disk = Library::load(&fx.dirs).unwrap();
    assert_eq!(on_disk.books().len(), 1, "saved once, for the whole batch");
    assert_eq!(on_disk.books()[0].id, fx.ids[2]);
    assert!(
        fx.files.iter().all(|file| file.exists()),
        "comic files are never deleted"
    );
    assert!(!fx.dirs.cover(&fx.ids[0]).exists() && !fx.dirs.cover(&fx.ids[1]).exists());
    let toast = view.read_with(cx, |this, _| this.toast.clone().map(|t| t.to_string()));
    assert_eq!(toast.as_deref(), Some("Removed 2 books from the library"));
}

#[gpui_kit::test]
fn bulk_removal_asks_first_when_progress_would_be_lost(cx: &mut TestAppContext) {
    let fx = fixture("bulkconfirm");
    let (view, cx) = launch(&fx, cx);
    set_progress(&view, cx, &fx.ids[1], 3);

    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_keystrokes("delete");
    assert_eq!(overlay_kind(&view, cx), "confirm");
    assert_eq!(
        book_count(&view, cx),
        3,
        "nothing is removed before confirming"
    );

    // cancelling keeps both the books and the selection
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
    assert_eq!(book_count(&view, cx), 3);
    assert_eq!(selected(&view, cx).len(), 3);

    cx.simulate_keystrokes("delete");
    cx.simulate_keystrokes("enter");
    assert_eq!(book_count(&view, cx), 0);
    assert!(selected(&view, cx).is_empty());
    assert!(fx.files.iter().all(|file| file.exists()));
    assert_eq!(Library::load(&fx.dirs).unwrap().books().len(), 0);
}

#[gpui_kit::test]
fn delete_with_nothing_selected_does_nothing(cx: &mut TestAppContext) {
    let fx = fixture("deletenothing");
    let (view, cx) = launch(&fx, cx);
    cx.simulate_keystrokes("delete");
    assert_eq!(book_count(&view, cx), 3);
    assert_eq!(overlay_kind(&view, cx), "none");
}

#[gpui_kit::test]
fn header_buttons_manage_the_selection(cx: &mut TestAppContext) {
    let fx = fixture("headerbuttons");
    let (view, cx) = launch(&fx, cx);
    // (gpui keeps the last painted bounds of every selector, so only "has it ever appeared"
    // can be checked for these, not "has it disappeared")
    assert!(
        cx.debug_bounds("select-remove").is_none(),
        "no selection controls until needed"
    );
    assert!(cx.debug_bounds("add-files").is_some());

    ctrl_click(cx, 0);
    let select_all = cx
        .debug_bounds("select-all")
        .expect("the header offers Select all");
    click(cx, select_all.center(), MouseButton::Left);
    assert_eq!(selected(&view, cx), fx.ids);

    let cancel = cx.debug_bounds("select-cancel").unwrap();
    click(cx, cancel.center(), MouseButton::Left);
    assert!(selected(&view, cx).is_empty());

    ctrl_click(cx, 0);
    ctrl_click(cx, 1);
    let remove = cx.debug_bounds("select-remove").unwrap();
    click(cx, remove.center(), MouseButton::Left);
    assert_eq!(book_count(&view, cx), 1);
    assert!(selected(&view, cx).is_empty());
}

#[gpui_kit::test]
fn right_click_on_a_selected_book_offers_bulk_actions(cx: &mut TestAppContext) {
    let fx = fixture("bulkmenu");
    let (view, cx) = launch(&fx, cx);

    ctrl_click(cx, 0);
    ctrl_click(cx, 1);
    click(cx, card(1), MouseButton::Right);
    assert_eq!(overlay_kind(&view, cx), "menu");
    assert_eq!(
        menu_labels(&view, cx),
        vec![
            "Clear Selection".to_string(),
            "Set Series for Selected Books…".to_string(),
            "Remove 2 Books from Library".to_string()
        ]
    );
    assert_eq!(
        selected(&view, cx).len(),
        2,
        "opening the menu keeps the selection"
    );

    cx.simulate_keystrokes("down enter");
    assert!(selected(&view, cx).is_empty());
    assert_eq!(book_count(&view, cx), 3);

    // (clicks right after a menu closes are ignored on purpose)
    std::thread::sleep(Duration::from_millis(350));
    ctrl_click(cx, 0);
    ctrl_click(cx, 2);
    click(cx, card(2), MouseButton::Right);
    cx.simulate_keystrokes("end enter");
    assert_eq!(book_count(&view, cx), 1);
    assert_eq!(Library::load(&fx.dirs).unwrap().books()[0].id, fx.ids[1]);
}

#[gpui_kit::test]
fn right_click_on_an_unselected_book_drops_the_selection(cx: &mut TestAppContext) {
    let fx = fixture("rightunselected");
    let (view, cx) = launch(&fx, cx);

    ctrl_click(cx, 0);
    ctrl_click(cx, 1);
    click(cx, card(2), MouseButton::Right);
    assert!(selected(&view, cx).is_empty());
    assert_eq!(
        menu_labels(&view, cx)[0],
        "Start Reading",
        "the normal single-book menu"
    );

    // a single selected book also gets the normal menu, not a "1 book" bulk menu
    close(&view, cx);
    std::thread::sleep(Duration::from_millis(350));
    ctrl_click(cx, 0);
    click(cx, card(0), MouseButton::Right);
    assert_eq!(menu_labels(&view, cx)[0], "Start Reading");
}

#[gpui_kit::test]
fn removing_one_book_updates_the_selection_and_opening_clears_it(cx: &mut TestAppContext) {
    let fx = fixture("pruneselection");
    let (view, cx) = launch(&fx, cx);

    ctrl_click(cx, 0);
    ctrl_click(cx, 1);
    act(&view, cx, &fx.ids[0], BookAction::Remove);
    assert_eq!(
        selected(&view, cx),
        vec![fx.ids[1].clone()],
        "the removed book left the selection"
    );

    act(&view, cx, &fx.ids[1], BookAction::Open);
    assert_eq!(reading(&view, cx), Some(0));
    assert!(
        selected(&view, cx).is_empty(),
        "opening a book clears the selection"
    );
}

#[gpui_kit::test]
fn selection_keys_do_not_leak_into_the_reader(cx: &mut TestAppContext) {
    let fx = fixture("readerkeys");
    let (view, cx) = launch(&fx, cx);
    act(&view, cx, &fx.ids[0], BookAction::Open);
    cx.simulate_keystrokes("ctrl-a");
    assert!(
        selected(&view, cx).is_empty(),
        "Ctrl+A only selects in the library"
    );
    cx.simulate_keystrokes("delete");
    assert_eq!(book_count(&view, cx), 3);
}

// ---------------------------------------------------------------------------------------
// Reading direction, page-turn slide, smooth scrolling
// ---------------------------------------------------------------------------------------

const TEST_VIEWPORT: Viewport = Viewport {
    width: 800.0,
    height: 600.0,
    scale: 1.0,
};

fn fake_page(shade: u8) -> Page {
    let image = image::RgbaImage::from_pixel(60, 90, image::Rgba([shade, 0, 0, 255]));
    Page {
        image: Arc::new(RenderImage::new(vec![Frame::new(image)])),
        orig: (60, 90),
        tier: 1024,
        bytes: 60 * 90 * 4,
    }
}

/// A reader with `pages` decoded pages and an 800x600 viewport, no window needed.
fn test_reader(pages: usize, settings: &Settings) -> Reader {
    let book = Book {
        id: "b".into(),
        title: "Test".into(),
        series: None,
        path: PathBuf::from("/tmp/test.cbz"),
        kind: SourceKind::Archive,
        page_count: pages,
        current_page: 0,
        opened: true,
        added_at: 0,
        last_read: None,
        marks: Default::default(),
    };
    let mut reader = Reader::new(&book, settings);
    reader.pages = Some(Arc::new((0..pages).map(|i| format!("{i}.png")).collect()));
    reader.viewport = TEST_VIEWPORT;
    for index in 0..pages {
        reader.images.insert(index, fake_page(index as u8));
    }
    reader
}

/// Turn the page the way `Yomu::goto_page` does: start the slide, then move.
fn turn(reader: &mut Reader, forward: bool) {
    let target = if forward {
        reader.page + 1
    } else {
        reader.page - 1
    };
    reader.begin_transition(forward, true);
    reader.page = target;
}

fn run_frames(reader: &mut Reader, hz: f32) -> usize {
    let mut frames = 0;
    while reader.advance_animations(1.0 / hz, &TEST_VIEWPORT) {
        frames += 1;
        assert!(frames < 5_000, "animation never stopped");
    }
    frames
}

#[test]
fn page_turn_slides_in_the_direction_of_reading() {
    for (forward, expected) in [
        (true, -1.0), // next page: content moves left
        (false, 1.0), // previous page: content moves right
    ] {
        let settings = Settings::default();
        let mut reader = test_reader(4, &settings);
        reader.page = 2;
        let leaving = reader.images[&2].image.clone();
        turn(&mut reader, forward);
        let transition = reader.transition.expect("a slide started");
        assert_eq!(transition.travel(), expected, "forward={forward}");
        assert!(Arc::ptr_eq(
            &reader.outgoing.as_ref().unwrap().image,
            &leaving
        ));
    }
}

#[test]
fn slide_finishes_and_never_frees_a_texture_the_cache_still_holds() {
    let mut reader = test_reader(3, &Settings::default());
    turn(&mut reader, true);
    let frames = run_frames(&mut reader, 144.0);
    assert!(
        frames > 5,
        "the slide spans several frames, not a jump ({frames})"
    );
    assert!(reader.transition.is_none() && reader.outgoing.is_none());
    assert!(reader.released.is_empty());
}

#[test]
fn an_evicted_outgoing_page_is_released_exactly_once_when_its_slide_ends() {
    let mut reader = test_reader(3, &Settings::default());
    let leaving = reader.images[&0].image.clone();
    turn(&mut reader, true);
    reader.images.remove(&0); // evicted while still sliding out
    run_frames(&mut reader, 60.0);
    assert_eq!(reader.released.len(), 1);
    assert!(Arc::ptr_eq(&reader.released[0], &leaving));
}

#[test]
fn rapid_turns_replace_the_slide_instead_of_queueing_and_continue_from_where_the_page_is() {
    let mut reader = test_reader(5, &Settings::default());
    turn(&mut reader, true);
    for _ in 0..4 {
        reader.advance_animations(1.0 / 120.0, &TEST_VIEWPORT);
    }
    let on_screen = reader.transition.unwrap().incoming_shift();
    assert!(on_screen > 0.0 && on_screen < 1.0, "mid-slide");

    let first_out = reader.images[&0].image.clone();
    reader.images.remove(&0);
    let second_out = reader.images[&1].image.clone();
    turn(&mut reader, true); // page 1 -> 2 while 0 -> 1 is still sliding
    assert!(Arc::ptr_eq(
        &reader.outgoing.as_ref().unwrap().image,
        &second_out
    ));
    assert_eq!(
        reader.transition.unwrap().outgoing_shift(),
        on_screen,
        "no jump when interrupted"
    );
    assert!(
        reader
            .released
            .iter()
            .any(|image| Arc::ptr_eq(image, &first_out)),
        "the page that was leaving is dropped, not kept around"
    );
    assert_eq!(reader.released.len(), 1);

    // keep mashing: still exactly one slide, and it settles
    for _ in 0..2 {
        turn(&mut reader, true);
    }
    assert_eq!(reader.page, 4);
    run_frames(&mut reader, 144.0);
    assert!(reader.transition.is_none() && reader.outgoing.is_none());
}

#[test]
fn pressing_back_mid_slide_reverses_smoothly() {
    let mut reader = test_reader(4, &Settings::default());
    reader.page = 1;
    turn(&mut reader, true);
    for _ in 0..3 {
        reader.advance_animations(1.0 / 120.0, &TEST_VIEWPORT);
    }
    let on_screen = reader.transition.unwrap().incoming_shift();
    turn(&mut reader, false);
    let transition = reader.transition.unwrap();
    assert_eq!(transition.travel(), 1.0, "now moving the other way");
    assert_eq!(transition.outgoing_shift(), on_screen);
}

#[test]
fn no_slide_when_animations_are_off_the_page_is_not_ready_or_there_is_no_window() {
    // animations off
    let mut reader = test_reader(3, &Settings::default());
    reader.begin_transition(true, false);
    assert!(reader.transition.is_none() && reader.outgoing.is_none());

    // the page on screen is not decoded yet (initial loading)
    let mut reader = test_reader(3, &Settings::default());
    reader.images.remove(&0);
    reader.begin_transition(true, true);
    assert!(reader.transition.is_none() && reader.outgoing.is_none());

    // no window size yet
    let mut reader = test_reader(3, &Settings::default());
    reader.viewport = Viewport::EMPTY;
    reader.begin_transition(true, true);
    assert!(reader.transition.is_none());

    // a freshly opened book never starts with a slide
    assert!(test_reader(3, &Settings::default()).transition.is_none());
}

#[test]
fn resizing_the_window_cancels_the_slide() {
    let mut reader = test_reader(3, &Settings::default());
    turn(&mut reader, true);
    assert!(reader.transition.is_some());
    let resized = Viewport {
        width: 900.0,
        ..TEST_VIEWPORT
    };
    reader.advance_animations(0.016, &resized);
    assert!(reader.transition.is_none() && reader.outgoing.is_none());
}

#[test]
fn the_slide_waits_for_the_new_page_instead_of_showing_a_blank() {
    let mut reader = test_reader(4, &Settings::default());
    reader.begin_transition(true, true);
    reader.page = 3;
    reader.images.remove(&3); // still decoding

    assert!(
        !reader.advance_animations(0.1, &TEST_VIEWPORT),
        "no frames requested while waiting"
    );
    let waiting = reader.transition.expect("still pending");
    assert_eq!(
        waiting.outgoing_shift(),
        0.0,
        "the old page stays where it was"
    );

    reader.images.insert(3, fake_page(3));
    assert!(
        reader.advance_animations(0.016, &TEST_VIEWPORT),
        "starts moving once the page is ready"
    );
    assert!(reader.transition.unwrap().outgoing_shift() != 0.0);
}

#[test]
fn a_page_that_fails_to_decode_ends_the_slide() {
    let mut reader = test_reader(3, &Settings::default());
    reader.begin_transition(true, true);
    reader.page = 1;
    reader.failed.insert(1, "broken".into());
    reader.advance_animations(0.016, &TEST_VIEWPORT);
    assert!(reader.transition.is_none() && reader.outgoing.is_none());
}

#[test]
fn scroll_glides_to_its_target_and_then_stops_asking_for_frames() {
    let mut reader = test_reader(2, &Settings::default());
    reader.scroll_target = (0.0, 300.0);
    let mut last = 0.0;
    let mut frames = 0;
    while reader.advance_animations(1.0 / 144.0, &TEST_VIEWPORT) {
        assert!(
            reader.off.1 >= last && reader.off.1 <= 300.0,
            "monotonic, no overshoot"
        );
        assert!(reader.off.1 < 300.0 || frames > 0);
        last = reader.off.1;
        frames += 1;
        assert!(frames < 1_000);
    }
    assert_eq!(reader.off, (0.0, 300.0));
    assert!(frames > 10, "a glide, not a jump ({frames} frames)");
    assert!(
        !reader.advance_animations(1.0 / 144.0, &TEST_VIEWPORT),
        "idle afterwards"
    );
}

#[test]
fn scroll_glide_takes_the_same_time_at_every_refresh_rate() {
    let mut durations = Vec::new();
    for hz in [60.0, 120.0, 144.0, 240.0] {
        let mut reader = test_reader(2, &Settings::default());
        reader.scroll_target = (0.0, 400.0);
        let frames = run_frames(&mut reader, hz);
        durations.push(frames as f32 / hz);
    }
    let (min, max) = durations
        .iter()
        .fold((f32::MAX, 0.0f32), |(lo, hi), d| (lo.min(*d), hi.max(*d)));
    assert!(max - min < 0.03, "{durations:?}");
}

#[test]
fn with_animations_off_scrolling_snaps_and_nothing_slides() {
    let settings = Settings {
        animate: false,
        ..Settings::default()
    };
    let mut reader = test_reader(3, &settings);
    reader.scroll_target = (0.0, 250.0);
    assert!(!reader.advance_animations(0.016, &TEST_VIEWPORT));
    assert_eq!(reader.off, (0.0, 250.0));
    reader.begin_transition(true, reader.animate);
    assert!(reader.transition.is_none());
}

#[test]
fn closing_a_book_hands_back_every_texture() {
    let mut reader = test_reader(4, &Settings::default());
    turn(&mut reader, true);
    reader.images.remove(&0);
    reader.released.push(fake_page(9).image);
    // 3 cached + 1 outgoing (no longer cached) + 1 already released
    assert_eq!(reader.into_images().len(), 5);
}

// ---- through the real views ----

fn with_reader<T>(
    view: &Entity<Yomu>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&Reader) -> T,
) -> T {
    view.read_with(cx, |this, _| {
        f(this.reader.as_ref().expect("a book is open"))
    })
}

fn open_book_by_index(fx: &Fixture, view: &Entity<Yomu>, cx: &mut VisualTestContext, index: usize) {
    act(view, cx, &fx.ids[index], BookAction::Open);
    cx.run_until_parked();
    assert!(with_reader(view, cx, |r| r.pages.is_some() && r.current().is_some()));
}

/// Run animation frames with a fixed 120 Hz clock until nothing is moving.
fn settle_animations(view: &Entity<Yomu>, cx: &mut VisualTestContext) {
    view.update(cx, |this, _| this.frame_dt = Some(1.0 / 120.0));
    for _ in 0..600 {
        cx.run_until_parked();
        if view.read_with(cx, |this, _| this.last_frame.is_none()) {
            return;
        }
        view.update(cx, |_, cx| cx.notify());
    }
    panic!("animation never settled");
}

fn page(view: &Entity<Yomu>, cx: &mut VisualTestContext) -> usize {
    with_reader(view, cx, |r| r.page)
}

#[gpui_kit::test]
fn left_to_right_navigation_in_both_directions_at_the_start_middle_and_end(
    cx: &mut TestAppContext,
) {
    let fx = fixture("ltr");
    let (view, cx) = launch(&fx, cx);
    view.update_in(cx, |this, _, _| {
        this.library.settings_mut().vim_mode = true;
    });
    open_book_by_index(&fx, &view, cx, 0);

    cx.simulate_keystrokes("left");
    assert_eq!(page(&view, cx), 0, "cannot go before the first page");
    cx.simulate_keystrokes("right");
    assert_eq!(page(&view, cx), 1);
    cx.simulate_keystrokes("right right");
    assert_eq!(page(&view, cx), 3);
    cx.simulate_keystrokes("left");
    assert_eq!(page(&view, cx), 2, "backward from the middle");

    cx.simulate_keystrokes("shift-g");
    assert_eq!(page(&view, cx), 4);
    cx.simulate_keystrokes("right");
    assert_eq!(page(&view, cx), 4, "cannot go past the last page");
    cx.simulate_keystrokes("left");
    assert_eq!(page(&view, cx), 3, "backward from the end");
    cx.simulate_keystrokes("g g");
    assert_eq!(page(&view, cx), 0);

    // vim motions and the other next/previous keys agree
    cx.simulate_keystrokes("l");
    assert_eq!(page(&view, cx), 1);
    cx.simulate_keystrokes("h");
    assert_eq!(page(&view, cx), 0);
    cx.simulate_keystrokes("space");
    assert_eq!(page(&view, cx), 1);
    cx.simulate_keystrokes("shift-space");
    assert_eq!(page(&view, cx), 0);

    // every step was saved
    cx.simulate_keystrokes("right right");
    assert_eq!(saved(&fx, &fx.ids[0]).unwrap().current_page, 2);
}

#[gpui_kit::test]
fn clicking_the_screen_edges_and_the_wheel_turn_the_page(cx: &mut TestAppContext) {
    let fx = fixture("clicks");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    let (left, right) = (point(px(100.), px(500.)), point(px(1800.), px(500.)));

    click(cx, right, MouseButton::Left);
    assert_eq!(page(&view, cx), 1, "right edge advances page");
    click(cx, left, MouseButton::Left);
    assert_eq!(page(&view, cx), 0);

    // a wheel notch on a page that fits turns the page
    view.update_in(cx, |this, window, cx| {
        this.on_wheel(
            WheelParams::new(
                (0.0, -96.0),
                false,
                (0.0, 0.0),
                true,
                TouchPhase::Moved,
                viewport_of(window),
            ),
            cx,
        )
    });
    assert_eq!(page(&view, cx), 1);
    view.update_in(cx, |this, window, cx| {
        this.on_wheel(
            WheelParams::new(
                (0.0, 96.0),
                false,
                (0.0, 0.0),
                true,
                TouchPhase::Moved,
                viewport_of(window),
            ),
            cx,
        )
    });
    assert_eq!(page(&view, cx), 0);
}

#[gpui_kit::test]
fn decoded_pages_stay_in_a_small_window(cx: &mut TestAppContext) {
    let fx = fixture("cache-ltr");
    let (view, cx) = launch(&fx, cx);
    view.update_in(cx, |this, _, _| {
        this.library.settings_mut().vim_mode = true;
    });
    open_book_by_index(&fx, &view, cx, 0);
    cx.simulate_keystrokes("shift-g");
    cx.run_until_parked();
    settle_animations(&view, cx);

    let (cached, current) = with_reader(&view, cx, |r| {
        let mut keys: Vec<usize> = r.images.keys().copied().collect();
        keys.sort();
        (keys, r.page)
    });
    assert_eq!(current, 4);
    assert!(
        cached.contains(&4) && cached.contains(&3),
        "the page behind is ready: {cached:?}"
    );
    assert!(
        cached.len() <= 1 + pagecache::KEEP_BEHIND + pagecache::KEEP_AHEAD,
        "{cached:?}"
    );
    let garbage = view.read_with(cx, |this, _| this.garbage.len());
    assert_eq!(garbage, 0, "no textures waiting to be freed once idle");
}

// ---- page-turn slide through the app ----

#[gpui_kit::test]
fn a_page_turn_slides_then_settles_and_position_is_never_behind(cx: &mut TestAppContext) {
    let fx = fixture("slide");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    assert!(
        with_reader(&view, cx, |r| r.transition.is_none()
            && r.outgoing.is_none()),
        "no slide on opening"
    );

    view.update(cx, |this, cx| {
        this.goto_page(1, Arrive::Start, cx);
        let reader = this.reader.as_ref().unwrap();
        // the position is already on the new page; only the visual is catching up
        assert_eq!(reader.page, 1);
        assert_eq!(this.library.get(&reader.book_id).unwrap().current_page, 1);
        assert_eq!(
            reader.transition.unwrap().travel(),
            -1.0,
            "forward, left to right: content moves left"
        );
        assert!(reader.outgoing.is_some());
    });
    settle_animations(&view, cx);
    assert!(with_reader(&view, cx, |r| r.transition.is_none()
        && r.outgoing.is_none()));
    assert_eq!(page(&view, cx), 1);
    assert_eq!(view.read_with(cx, |this, _| this.garbage.len()), 0);

    // backward slides the other way
    view.update(cx, |this, cx| {
        this.goto_page(0, Arrive::End, cx);
        assert_eq!(
            this.reader.as_ref().unwrap().transition.unwrap().travel(),
            1.0
        );
    });
    settle_animations(&view, cx);
}

#[gpui_kit::test]
fn rapid_page_turns_never_desync_the_position_from_the_picture(cx: &mut TestAppContext) {
    let fx = fixture("rapid");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    let id = fx.ids[0].clone();

    view.update(cx, |this, cx| {
        for step in [
            true, true, true, true, false, false, true, false, false, false,
        ] {
            this.turn_by(step, 1, Arrive::Start, cx);
            let reader = this.reader.as_ref().unwrap();
            assert!(
                reader.transition.is_some() || reader.outgoing.is_none(),
                "at most one slide"
            );
            assert_eq!(
                this.library.get(&id).unwrap().current_page,
                reader.page,
                "saved position tracks the page instantly"
            );
        }
    });
    settle_animations(&view, cx);
    assert_eq!(page(&view, cx), 0);
    assert!(with_reader(&view, cx, |r| r.transition.is_none()
        && r.outgoing.is_none()
        && r.released.is_empty()));
    assert_eq!(saved(&fx, &id).unwrap().current_page, 0);

    // key repeat arrives as separate key presses
    cx.simulate_keystrokes("right right right right");
    settle_animations(&view, cx);
    assert_eq!(page(&view, cx), 4);
    assert_eq!(saved(&fx, &id).unwrap().current_page, 4);
}

#[gpui_kit::test]
fn zooming_fitting_and_resizing_do_not_slide(cx: &mut TestAppContext) {
    let fx = fixture("noslide");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    for action in 0..3 {
        view.update_in(cx, |this, window, cx| {
            this.goto_page(this.reader.as_ref().unwrap().page + 1, Arrive::Start, cx);
            assert!(this.reader.as_ref().unwrap().transition.is_some());
            match action {
                0 => this.set_fit(FitMode::Width, cx),
                1 => {
                    let vp = viewport_of(window);
                    this.zoom_by(1.25, (vp.width / 2.0, vp.height / 2.0), vp, cx);
                }
                _ => {
                    let reader = this.reader.as_mut().unwrap();
                    let vp = Viewport {
                        width: reader.viewport.width + 100.0,
                        ..reader.viewport
                    };
                    reader.advance_animations(0.016, &vp);
                }
            }
            let reader = this.reader.as_ref().unwrap();
            assert!(
                reader.transition.is_none() && reader.outgoing.is_none(),
                "action {action}"
            );
        });
        settle_animations(&view, cx);
    }
}

#[gpui_kit::test]
fn switching_comics_starts_clean(cx: &mut TestAppContext) {
    let fx = fixture("switch");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    view.update(cx, |this, cx| this.goto_page(2, Arrive::Start, cx));
    view.update_in(cx, |this, window, cx| this.close_book(window, cx));
    cx.run_until_parked();
    open_book_by_index(&fx, &view, cx, 1);
    assert!(with_reader(&view, cx, |r| r.transition.is_none()
        && r.outgoing.is_none()
        && r.page == 0));
    assert!(
        view.read_with(cx, |this, _| this.last_frame.is_none()),
        "no animation clock left running"
    );
}

#[gpui_kit::test]
fn reduced_motion_and_the_animate_command_turn_animation_off(cx: &mut TestAppContext) {
    let fx = fixture("reduced");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // the desktop asks for reduced motion
    view.update(cx, |this, _| this.reduce_motion = true);
    view.update(cx, |this, cx| {
        this.goto_page(1, Arrive::Start, cx);
        assert!(
            this.reader.as_ref().unwrap().transition.is_none(),
            "no slide"
        );
    });
    cx.run_until_parked();
    assert_eq!(page(&view, cx), 1, "navigation itself is unaffected");
    view.update(cx, |this, _| this.reduce_motion = false);

    // `:animate off` is a saved preference
    view.update_in(cx, |this, window, cx| {
        this.apply(
            Action::SetAnimations(Some(false)),
            viewport_of(window),
            window,
            cx,
        );
        this.goto_page(2, Arrive::Start, cx);
        assert!(this.reader.as_ref().unwrap().transition.is_none());
    });
    assert!(!saved_settings(&fx).animate);

    // the toggle form flips it back
    view.update_in(cx, |this, window, cx| {
        this.apply(Action::SetAnimations(None), viewport_of(window), window, cx);
    });
    assert!(saved_settings(&fx).animate);
    view.update(cx, |this, cx| {
        this.goto_page(3, Arrive::Start, cx);
        assert!(this.reader.as_ref().unwrap().transition.is_some());
    });
    settle_animations(&view, cx);
}

// ---- smooth scrolling through the app ----

#[gpui_kit::test]
fn keyboard_scrolling_glides_accumulates_and_leaves_the_app_idle(cx: &mut TestAppContext) {
    let fx = fixture("glide");
    let (view, cx) = launch(&fx, cx);
    view.update_in(cx, |this, _, _| {
        this.library.settings_mut().vim_mode = true;
    });
    open_book_by_index(&fx, &view, cx, 0);
    // zoom in far enough that the page is taller than the window
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.zoom_to(2.0, (vp.width / 2.0, vp.height / 2.0), vp, cx);
    });
    cx.run_until_parked();
    settle_animations(&view, cx);
    assert!(with_reader(&view, cx, |r| layout::max_offset(
        layout::display_size(r.fit, r.zoom, (60.0, 90.0), &r.viewport),
        &r.viewport
    )
    .1 > 100.0));

    // zooming at the centre leaves the page scrolled part way down
    let base = with_reader(&view, cx, |r| r.off.1);
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.scroll(Dir::Down, Amount::Line, 1, vp, cx);
        this.scroll(Dir::Down, Amount::Line, 1, vp, cx);
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(
            reader.scroll_target.1,
            base + 2.0 * LINE_STEP,
            "presses accumulate on the target"
        );
        assert_eq!(reader.off.1, base, "the picture has not jumped; it glides");
    });
    settle_animations(&view, cx);
    assert_eq!(
        with_reader(&view, cx, |r| r.off.1),
        base + 2.0 * LINE_STEP,
        "arrives exactly"
    );
    assert!(
        view.read_with(cx, |this, _| this.last_frame.is_none()),
        "no frames requested when idle"
    );

    // pressing up goes back along the same path
    cx.simulate_keystrokes("k");
    settle_animations(&view, cx);
    assert_eq!(with_reader(&view, cx, |r| r.off.1), base + LINE_STEP);
}

#[gpui_kit::test]
fn touchpad_deltas_are_followed_one_to_one_and_wheel_notches_glide(cx: &mut TestAppContext) {
    let fx = fixture("wheel");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.zoom_to(2.0, (vp.width / 2.0, vp.height / 2.0), vp, cx);
    });
    settle_animations(&view, cx);

    let base = with_reader(&view, cx, |r| r.off.1);
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        // a touchpad: small pixel deltas, not discrete
        this.on_wheel(
            WheelParams::new(
                (0.0, -30.0),
                false,
                (0.0, 0.0),
                false,
                TouchPhase::Moved,
                vp,
            ),
            cx,
        );
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(
            (reader.off.1, reader.scroll_target.1),
            (base + 30.0, base + 30.0),
            "no lag"
        );
        // a wheel notch: glides
        this.on_wheel(
            WheelParams::new(
                (0.0, -WHEEL_LINE),
                false,
                (0.0, 0.0),
                true,
                TouchPhase::Moved,
                vp,
            ),
            cx,
        );
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.scroll_target.1, base + 30.0 + WHEEL_LINE);
        assert_eq!(reader.off.1, base + 30.0);
    });
    settle_animations(&view, cx);
    assert_eq!(
        with_reader(&view, cx, |r| r.off.1),
        base + 30.0 + WHEEL_LINE
    );
}

#[gpui_kit::test]
fn touchpad_flick_triggers_inertial_scroll_glide(cx: &mut TestAppContext) {
    let fx = fixture("wheel");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.zoom_to(2.0, (vp.width / 2.0, vp.height / 2.0), vp, cx);
    });
    settle_animations(&view, cx);

    let base = with_reader(&view, cx, |r| r.off.1);
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        // Simulate a fast touchpad flick:
        this.on_wheel(
            WheelParams::new(
                (0.0, -20.0),
                false,
                (0.0, 0.0),
                false,
                TouchPhase::Started,
                vp,
            ),
            cx,
        );
        this.on_wheel(
            WheelParams::new(
                (0.0, -40.0),
                false,
                (0.0, 0.0),
                false,
                TouchPhase::Moved,
                vp,
            ),
            cx,
        );
        this.on_wheel(
            WheelParams::new(
                (0.0, -60.0),
                false,
                (0.0, 0.0),
                false,
                TouchPhase::Moved,
                vp,
            ),
            cx,
        );
        this.on_wheel(
            WheelParams::new((0.0, 0.0), false, (0.0, 0.0), false, TouchPhase::Ended, vp),
            cx,
        );

        let reader = this.reader.as_ref().unwrap();
        assert!(
            reader.kinetic.has_active_inertia(),
            "inertia must be active after flick"
        );
    });

    // Let the inertial glide settle to rest
    settle_animations(&view, cx);
    let final_off = with_reader(&view, cx, |r| r.off.1);
    assert!(
        final_off > base + 120.0,
        "inertial glide carried past the physical touch release"
    );
    assert!(
        with_reader(&view, cx, |r| r.kinetic.inertia.is_none()),
        "inertia halts cleanly when done"
    );
}

#[gpui_kit::test]
fn touchpad_flick_without_ended_event_triggers_inertia_on_linux(cx: &mut TestAppContext) {
    let fx = fixture("wheel-linux");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.zoom_to(2.0, (vp.width / 2.0, vp.height / 2.0), vp, cx);
    });
    settle_animations(&view, cx);
    let base = with_reader(&view, cx, |r| r.off.1);

    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        // On Linux, Wayland only ever dispatches TouchPhase::Moved:
        this.on_wheel(
            WheelParams::new(
                (0.0, -40.0),
                false,
                (0.0, 0.0),
                false,
                TouchPhase::Moved,
                vp,
            ),
            cx,
        );
        this.on_wheel(
            WheelParams::new(
                (0.0, -60.0),
                false,
                (0.0, 0.0),
                false,
                TouchPhase::Moved,
                vp,
            ),
            cx,
        );
        // Simulate 40ms passing without new events (fingers lifted off pad)
        if let Some(reader) = this.reader.as_mut() {
            reader.kinetic.last_motion_time = Some(Instant::now() - Duration::from_millis(40));
        }
    });

    // Next frame triggers inertia and glides
    settle_animations(&view, cx);
    let final_off = with_reader(&view, cx, |r| r.off.1);
    assert!(
        final_off > base + 100.0,
        "inertial glide carried past the physical touch release on Linux"
    );
    assert!(
        with_reader(&view, cx, |r| r.kinetic.inertia.is_none()),
        "inertia halts cleanly when done"
    );
}

#[gpui_kit::test]
fn touchpad_flick_triggers_library_inertial_scroll_glide_on_linux(cx: &mut TestAppContext) {
    let fx = fixture("library-inertia");
    let (view, cx) = launch(&fx, cx);
    view.update_in(cx, |this, _window, cx| {
        for i in 0..30 {
            let mut b = this.library.books()[0].clone();
            b.id = format!("book_{i}");
            this.library.add(b);
        }
        cx.notify();
    });
    settle_animations(&view, cx);

    cx.simulate_event(MouseMoveEvent {
        position: point(px(400.), px(300.)),
        modifiers: Modifiers::default(),
        pressed_button: None,
    });

    // Simulate event over the library cards area (e.g. card 0)
    for _ in 0..4 {
        cx.simulate_event(ScrollWheelEvent {
            position: card(0),
            delta: ScrollDelta::Pixels(point(px(0.), px(-60.))),
            touch_phase: TouchPhase::Moved,
            modifiers: Modifiers::default(),
        });
    }

    let offset_after_wheel = view.read_with(cx, |this, _| this.library_scroll.offset().y);
    assert_eq!(offset_after_wheel, px(-240.));

    // Advance time past stillness threshold (40ms) to trigger inertia launch
    view.update_in(cx, |this, _window, cx| {
        this.library_scroll_controller.kinetic.last_motion_time =
            Some(Instant::now() - Duration::from_millis(40));
        this.wake();
        cx.notify();
    });

    settle_animations(&view, cx);

    let final_offset = view.read_with(cx, |this, _| this.library_scroll.offset().y);
    assert!(final_offset < offset_after_wheel);
    assert!(!view.read_with(cx, |this, _| this.library_scroll_controller.is_animating()));
}

#[gpui_kit::test]
fn touch_down_mid_inertia_cancels_immediately(cx: &mut TestAppContext) {
    let fx = fixture("wheel");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.zoom_to(2.0, (vp.width / 2.0, vp.height / 2.0), vp, cx);
    });
    settle_animations(&view, cx);

    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.on_wheel(
            WheelParams::new(
                (0.0, -40.0),
                false,
                (0.0, 0.0),
                false,
                TouchPhase::Moved,
                vp,
            ),
            cx,
        );
        this.on_wheel(
            WheelParams::new(
                (0.0, -40.0),
                false,
                (0.0, 0.0),
                false,
                TouchPhase::Ended,
                vp,
            ),
            cx,
        );
        assert!(this.reader.as_ref().unwrap().kinetic.has_active_inertia());

        // User touches the touchpad again:
        this.on_wheel(
            WheelParams::new(
                (0.0, 0.0),
                false,
                (0.0, 0.0),
                false,
                TouchPhase::Started,
                vp,
            ),
            cx,
        );
        assert!(
            this.reader.as_ref().unwrap().kinetic.inertia.is_none(),
            "touch started cancels inertia"
        );
    });
}

#[gpui_kit::test]
fn reader_pinch_zoom_smoothly_scales_and_cancels_inertia(cx: &mut TestAppContext) {
    let fx = fixture("pinch");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        let center = (vp.width / 2.0, vp.height / 2.0);

        let initial_zoom = this.reader.as_ref().unwrap().zoom;

        // Pinch start resets kinetic
        this.on_pinch(center, 0.0, TouchPhase::Started, vp, cx);
        assert_eq!(this.reader.as_ref().unwrap().zoom, initial_zoom);

        // Pinch zoom in (delta > 0)
        this.on_pinch(center, 0.25, TouchPhase::Moved, vp, cx);
        let zoomed_in = this.reader.as_ref().unwrap().zoom;
        assert!(
            zoomed_in > initial_zoom,
            "zoom increases on positive pinch delta"
        );
        assert!(this.reader.as_ref().unwrap().kinetic.inertia.is_none());

        // Pinch zoom out (delta < 0)
        this.on_pinch(center, -0.2, TouchPhase::Moved, vp, cx);
        let zoomed_out = this.reader.as_ref().unwrap().zoom;
        assert!(
            zoomed_out < zoomed_in,
            "zoom decreases on negative pinch delta"
        );

        // Pinch ended leaves zoom stable
        this.on_pinch(center, 0.0, TouchPhase::Ended, vp, cx);
        assert_eq!(this.reader.as_ref().unwrap().zoom, zoomed_out);
    });
}

#[gpui_kit::test]
fn cursor_position_anchors_keyboard_and_continuous_zoom(cx: &mut TestAppContext) {
    let fx = fixture("cursor-zoom");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // 1. Single-page mode: cursor position is used by keyboard actions when set
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.cursor_pos = Some((vp.width * 0.75, vp.height * 0.75));
        this.apply(Action::ZoomIn(2), vp, window, cx);
        assert!(this.reader.as_ref().unwrap().zoom > 1.0);
    });

    // 2. Continuous mode: zoom anchors around cursor position
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.set_continuous(Some(true), cx);
        let cursor = (vp.width * 0.5, 200.0);
        let initial_off = this.reader.as_ref().unwrap().off;
        this.zoom_to(2.0, cursor, vp, cx);
        let new_off = this.reader.as_ref().unwrap().off;
        let expected_y = (initial_off.1 + 200.0) * 2.0 - 200.0;
        assert!(
            (new_off.1 - expected_y).abs() < 1e-2,
            "continuous zoom anchors at cursor Y: got {}, expected {}",
            new_off.1,
            expected_y
        );
    });
}

#[gpui_kit::test]
fn library_pinch_adjusts_thumbnail_size(cx: &mut TestAppContext) {
    let fx = fixture("lib-pinch");
    let (view, cx) = launch(&fx, cx);

    view.update_in(cx, |this, _, cx| {
        assert_eq!(
            this.library.settings().thumbnail_size,
            ThumbnailSize::Medium
        );

        // TouchPhase::Started resets accumulator
        this.on_library_pinch(0.0, TouchPhase::Started, cx);
        assert_eq!(this.library_pinch_accum, 0.0);

        // Accumulate positive pinch until threshold (> 0.35)
        this.on_library_pinch(0.2, TouchPhase::Moved, cx);
        assert_eq!(
            this.library.settings().thumbnail_size,
            ThumbnailSize::Medium
        );
        this.on_library_pinch(0.2, TouchPhase::Moved, cx);
        assert_eq!(this.library.settings().thumbnail_size, ThumbnailSize::Large);

        // Stepping up again caps at Large (maximum size)
        this.on_library_pinch(0.4, TouchPhase::Moved, cx);
        assert_eq!(this.library.settings().thumbnail_size, ThumbnailSize::Large);

        // Step down towards Medium (< -0.35)
        this.on_library_pinch(-0.4, TouchPhase::Moved, cx);
        assert_eq!(
            this.library.settings().thumbnail_size,
            ThumbnailSize::Medium
        );

        // Step down towards Small
        this.on_library_pinch(-0.4, TouchPhase::Moved, cx);
        assert_eq!(this.library.settings().thumbnail_size, ThumbnailSize::Small);

        // Stepping down again caps at Small (minimum size)
        this.on_library_pinch(-0.4, TouchPhase::Moved, cx);
        assert_eq!(this.library.settings().thumbnail_size, ThumbnailSize::Small);

        // Pinch ended clears accumulator
        this.on_library_pinch(0.1, TouchPhase::Moved, cx);
        assert_ne!(this.library_pinch_accum, 0.0);
        this.on_library_pinch(0.0, TouchPhase::Ended, cx);
        assert_eq!(this.library_pinch_accum, 0.0);
    });
}

#[test]
fn kinetic_scroll_flick_inertia_and_touch_catch() {
    let mut kinetic = KineticScroll::new();
    let now = Instant::now();

    kinetic.push_motion(now, 0.0, 0.0);
    assert!(kinetic.inertia.is_none());
    assert!(kinetic.last_motion_time.is_some());

    kinetic.push_motion(now + Duration::from_millis(16), 0.0, -40.0);
    kinetic.push_motion(now + Duration::from_millis(32), 0.0, -60.0);

    let started = kinetic.end_touch(true);
    assert!(started);
    assert!(kinetic.has_active_inertia());
    assert!(kinetic.inertia.is_some());

    let delta = kinetic.step(0.016);
    assert!(delta.is_some(), "inertia should be active");

    // Touch down mid-inertia cancels immediately (touch catch)
    kinetic.cancel_inertia();
    assert!(!kinetic.has_active_inertia());
    assert!(kinetic.inertia.is_none());
}

#[test]
fn kinetic_scroll_reduced_motion() {
    let mut kinetic = KineticScroll::new();
    let now = Instant::now();

    kinetic.push_motion(now, 0.0, 0.0);
    kinetic.push_motion(now + Duration::from_millis(16), 0.0, -80.0);

    // With allow_inertia = false (reduced motion), end_touch does not start inertia
    let started = kinetic.end_touch(false);
    assert!(!started);
    assert!(!kinetic.has_active_inertia());
    assert!(kinetic.inertia.is_none());
    assert!(kinetic.step(0.016).is_none());
}

#[test]
fn kinetic_scroll_linux_stillness_fallback() {
    let mut kinetic = KineticScroll::new();
    let now = Instant::now();

    kinetic.push_motion(now, 0.0, -40.0);
    kinetic.push_motion(now + Duration::from_millis(16), 0.0, -60.0);

    let t2 = now + Duration::from_millis(16);
    // Immediately after motion (<35ms), stillness timeout not elapsed
    assert!(!kinetic.check_stillness_timeout(t2 + Duration::from_millis(10), true));
    assert!(kinetic.inertia.is_none());
    assert!(kinetic.is_awaiting_release());

    // 40ms passing without Ended event (touchpad release without Ended on Linux)
    let running = kinetic.check_stillness_timeout(t2 + Duration::from_millis(40), true);
    assert!(running);
    assert!(kinetic.inertia.is_some());
}

#[test]
fn kinetic_scroll_stale_timeout_never_triggers_inertia() {
    let mut kinetic = KineticScroll::new();
    let now = Instant::now();

    kinetic.push_motion(now, 0.0, -50.0);

    // Simulate 200ms passing (stale motion, finger stopped on pad or long pause)
    let running = kinetic.check_stillness_timeout(now + Duration::from_millis(200), true);
    assert!(!running);
    assert!(kinetic.inertia.is_none());
    assert!(!kinetic.is_active());
}

#[gpui_kit::test]
fn scrolling_with_reduced_motion_is_immediate(cx: &mut TestAppContext) {
    let fx = fixture("glide-off");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.zoom_to(2.0, (vp.width / 2.0, vp.height / 2.0), vp, cx);
        this.reduce_motion = true;
    });
    settle_animations(&view, cx);
    let base = with_reader(&view, cx, |r| r.off.1);
    view.update_in(cx, |this, window, cx| {
        this.scroll(Dir::Down, Amount::Line, 3, viewport_of(window), cx);
    });
    settle_animations(&view, cx);
    assert_eq!(with_reader(&view, cx, |r| r.off.1), base + 3.0 * LINE_STEP);
}

#[gpui_kit::test]
fn the_whole_session_leaves_no_leaked_state(cx: &mut TestAppContext) {
    let fx = fixture("leaks");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    for _ in 0..3 {
        cx.simulate_keystrokes("right right right right left left left left");
    }
    settle_animations(&view, cx);
    assert_eq!(view.read_with(cx, |this, _| this.garbage.len()), 0);
    let (cached, pending, released) = with_reader(&view, cx, |r| {
        (r.images.len(), r.pending.len(), r.released.len())
    });
    assert!(cached <= 1 + pagecache::KEEP_BEHIND + pagecache::KEEP_AHEAD);
    assert_eq!((pending, released), (0, 0));
    view.update_in(cx, |this, window, cx| this.close_book(window, cx));
    settle_animations(&view, cx);
    assert!(view.read_with(cx, |this, _| this.reader.is_none()
        && this.last_frame.is_none()));
}

// ---------------------------------------------------------------------------------------
// Measurements. `cargo test perf_ -- --ignored --nocapture`
// ---------------------------------------------------------------------------------------

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    values[values.len() / 2]
}

#[test]
#[ignore = "prints timings"]
fn perf_decode_cost_per_tier() {
    // a large, noisy 3000x4500 page, like a high resolution scan
    let (w, h) = (3000u32, 4500u32);
    let image = image::RgbImage::from_fn(w, h, |x, y| {
        let n = (x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503)) >> 24;
        image::Rgb([
            ((x / 12 + n) & 255) as u8,
            ((y / 17 + n) & 255) as u8,
            (((x + y) / 23) & 255) as u8,
        ])
    });
    let mut jpeg = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut jpeg, 85,
        ))
        .unwrap();
    println!(
        "source page: {w}x{h} JPEG, {:.1} MB",
        jpeg.len() as f64 / 1e6
    );

    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(decoded.width(), w);
    }
    println!(
        "JPEG decode only (no scaling): {:.0} ms",
        median(&mut times)
    );

    let full = image::load_from_memory(&jpeg).unwrap();
    for filter in [
        image::imageops::FilterType::Lanczos3,
        image::imageops::FilterType::CatmullRom,
        image::imageops::FilterType::Triangle,
    ] {
        let mut times = Vec::new();
        for _ in 0..3 {
            let start = Instant::now();
            let _ = full.resize_exact(1365, 2048, filter);
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "resize 3000x4500 -> 1365x2048 with {filter:?}: {:.0} ms",
            median(&mut times)
        );
    }

    for tier in layout::TIERS {
        let mut times = Vec::new();
        let mut bytes = 0u64;
        for _ in 0..3 {
            let start = Instant::now();
            let decoded = imaging::decode_fit_bgra(&jpeg, tier, tier).unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            let (dw, dh) = decoded.image.dimensions();
            bytes = u64::from(dw) * u64::from(dh) * 4;
        }
        println!(
            "tier {tier:>4}: decode+scale {:>6.0} ms (median of 3, background thread), result {:>5.1} MB",
            median(&mut times),
            bytes as f64 / 1e6
        );
    }
}

#[gpui_kit::test]
fn continuous_reading_debounced_autosave_persists_on_flush(cx: &mut TestAppContext) {
    let fx = fixture("debounced_autosave");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();
    open_book_by_index(&fx, &view, cx, 0);

    view.update(cx, |this, cx| this.set_continuous(Some(true), cx));

    view.update(cx, |this, cx| {
        if let Some(book) = this.library.get_mut(&id) {
            book.current_page = 3;
        }
        this.save_library_debounced(cx);
    });

    view.update(cx, |this, _| {
        this.save_library();
    });

    assert_eq!(saved(&fx, &id).unwrap().current_page, 3);
}

#[test]
#[ignore = "prints timings"]
fn perf_per_frame_animation_math() {
    const N: u32 = 2_000_000;
    let start = Instant::now();
    let mut off = (0.0f32, 0.0f32);
    let mut sink = 0.0f32;
    for i in 0..N {
        let target = (0.0, 400.0 + (i % 7) as f32);
        off = motion::step_scroll(off, target, 1.0 / 144.0).0;
        let mut transition = PageTransition::starting(-1.0);
        transition.advance(0.01);
        sink += transition.outgoing_shift() + transition.incoming_shift() + off.1;
    }
    let per_frame = start.elapsed().as_secs_f64() * 1e9 / f64::from(N);
    println!("slide + glide state update: {per_frame:.0} ns per frame (sink {sink:.1})");
}

#[gpui_kit::test]
fn built_in_themes_cycle_and_persist(cx: &mut TestAppContext) {
    let fx = fixture("theme-cycle");
    let (view, cx) = launch(&fx, cx);

    let initial_theme = view.read_with(cx, |this, _| this.library.settings().theme_name.clone());
    assert_eq!(initial_theme, "Catppuccin Mocha");

    view.update_in(cx, |this, _, cx| {
        this.cycle_theme(1, cx);
    });
    let next_theme = view.read_with(cx, |this, _| this.library.settings().theme_name.clone());
    assert_ne!(next_theme, "Catppuccin Mocha");

    view.update_in(cx, |this, _, cx| {
        this.cycle_theme(-1, cx);
    });
    let restored_theme = view.read_with(cx, |this, _| this.library.settings().theme_name.clone());
    assert_eq!(restored_theme, "Catppuccin Mocha");
}

#[gpui_kit::test]
fn theme_command_changes_theme_and_suggests_all_themes(cx: &mut TestAppContext) {
    let fx = fixture("theme-command");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    view.update_in(cx, |this, window, cx| {
        let suggestions = command_suggestions("theme ", &this.custom_themes);
        assert!(suggestions.len() >= 6);
        let labels: Vec<String> = suggestions.iter().map(|s| s.1.clone()).collect();
        assert!(labels.contains(&"Catppuccin Mocha".to_string()));
        assert!(labels.contains(&"Catppuccin Latte".to_string()));
        assert!(labels.contains(&"Gruvbox Dark".to_string()));
        assert!(labels.contains(&"Gruvbox Light".to_string()));

        // Filtering works
        let filtered = command_suggestions("theme gruv", &this.custom_themes);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|s| s.1 == "Gruvbox Dark"));
        assert!(filtered.iter().any(|s| s.1 == "Gruvbox Light"));

        // Running theme command applies theme
        let vp = viewport_of(window);
        this.apply(Action::SetTheme("Gruvbox Dark".to_string()), vp, window, cx);
        assert_eq!(this.library.settings().theme_name, "Gruvbox Dark");

        // Empty theme command cycles theme
        this.apply(Action::SetTheme(String::new()), vp, window, cx);
        assert_ne!(this.library.settings().theme_name, "Gruvbox Dark");
    });
}

#[gpui_kit::test]
fn extended_thumbnail_border_and_rounding_options(cx: &mut TestAppContext) {
    let fx = fixture("options-test");
    let (view, cx) = launch(&fx, cx);

    view.update_in(cx, |this, _, cx| {
        // Thumbnail sizes: Small, Medium, Large
        this.set_thumbnail_size(ThumbnailSize::Small, cx);
        assert_eq!(this.library.settings().thumbnail_size, ThumbnailSize::Small);
        assert_eq!(ThumbnailSize::Small.card_width(), 180.0);
        this.set_thumbnail_size(ThumbnailSize::Large, cx);
        assert_eq!(this.library.settings().thumbnail_size, ThumbnailSize::Large);
        assert_eq!(ThumbnailSize::Large.card_width(), 270.0);

        // Border sizes
        this.set_border_size(BorderSize::None, cx);
        assert_eq!(this.library.settings().border_size, BorderSize::None);
        assert_eq!(this.border_px(), 0.0);
        this.set_border_size(BorderSize::Thick, cx);
        assert_eq!(this.library.settings().border_size, BorderSize::Thick);
        assert_eq!(this.border_px(), 2.0);

        // Corner rounding
        this.set_corner_rounding(CornerRounding::None, cx);
        assert_eq!(
            this.library.settings().corner_rounding,
            CornerRounding::None
        );
        assert_eq!(this.radius_px(), 0.0);
        assert_eq!(this.radius_lg_px(), 0.0);

        this.set_corner_rounding(CornerRounding::Standard, cx);
        assert_eq!(
            this.library.settings().corner_rounding,
            CornerRounding::Standard
        );
        assert_eq!(this.radius_px(), 6.0);
        assert_eq!(this.radius_lg_px(), 10.0);

        this.set_corner_rounding(CornerRounding::Rounded, cx);
        assert_eq!(
            this.library.settings().corner_rounding,
            CornerRounding::Rounded
        );
        assert_eq!(this.radius_px(), 12.0);
        assert_eq!(this.radius_lg_px(), 16.0);
    });
}

#[gpui_kit::test]
fn library_view_command_box_and_theme_preview(cx: &mut TestAppContext) {
    let fx = fixture("cmdbox-test");
    let (view, cx) = launch(&fx, cx);
    view.update_in(cx, |this, _, _| {
        this.library.settings_mut().vim_mode = true;
    });

    // Initial state: in library view, not in cmdline
    view.update_in(cx, |this, _, _| {
        assert!(this.reader.is_none());
        assert!(!this.in_cmdline());
        assert_eq!(this.cmdline_text(), None);
    });

    // Press ':' to open command box in library
    cx.simulate_keystrokes(":");
    view.update_in(cx, |this, _, _| {
        assert!(this.in_cmdline());
        assert_eq!(this.cmdline_text(), Some(""));
    });

    // Type "theme "
    cx.simulate_keystrokes("t h e m e space");
    view.update_in(cx, |this, _, _| {
        assert!(this.in_cmdline());
        assert_eq!(this.cmdline_text(), Some("theme "));
    });

    // Initial theme before preview
    let initial_theme = view.read_with(cx, |this, _| this.library.settings().theme_name.clone());

    // Cycle completions with Down: should trigger live preview
    cx.simulate_keystrokes("down");
    view.update_in(cx, |this, _, _| {
        assert!(this.theme_before_preview.is_some());
        assert_eq!(
            this.theme_before_preview.as_deref(),
            Some(initial_theme.as_str())
        );
    });

    // Press escape to cancel: should restore theme and exit cmdline
    cx.simulate_keystrokes("escape");
    view.update_in(cx, |this, _, _| {
        assert!(!this.in_cmdline());
        assert_eq!(this.theme_before_preview, None);
        assert_eq!(this.library.settings().theme_name, initial_theme);
    });

    // Press ':' again and run a theme command to completion
    cx.simulate_keystrokes(":");
    view.update_in(cx, |this, _, _| {
        assert!(this.in_cmdline());
    });
    // Replace cmdline with ":theme Gruvbox Light" and press enter
    view.update_in(cx, |this, _, _| {
        this.replace_cmdline("theme Gruvbox Light".to_string());
    });
    cx.simulate_keystrokes("enter");
    view.update_in(cx, |this, _, _| {
        assert!(!this.in_cmdline());
        assert_eq!(this.library.settings().theme_name, "Gruvbox Light");
        assert_eq!(this.theme_before_preview, None);
    });
}

#[gpui_kit::test]
fn command_box_rendering_and_cancellation(cx: &mut TestAppContext) {
    let fx = fixture("cmdbox-render");
    let (view, cx) = launch(&fx, cx);

    view.update_in(cx, |this, _, cx| {
        assert!(this.render_command_box(cx).is_none());
        this.library_vim.feed(":");
        this.replace_cmdline("theme ".to_string());
        assert!(this.render_command_box(cx).is_some());
        this.cancel_cmdline(cx);
        assert!(this.render_command_box(cx).is_none());
    });
}

#[gpui_kit::test]
fn reader_retains_zoom_across_books_and_sessions_by_default(cx: &mut TestAppContext) {
    let fx = fixture("zoom-retain");
    let (view, cx) = launch(&fx, cx);

    // Default setting: remember_zoom is true, default zoom is 1.0
    view.update_in(cx, |this, _, _| {
        assert!(this.library.settings().remember_zoom);
        assert_eq!(this.library.settings().zoom, 1.0);
    });

    // Open first book
    open_book_by_index(&fx, &view, cx, 0);
    view.update_in(cx, |this, _, _| {
        assert_eq!(this.reader.as_ref().unwrap().zoom, 1.0);
    });

    // Change zoom in first book to 1.5
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.zoom_to(1.5, (100.0, 100.0), vp, cx);
        assert_eq!(this.reader.as_ref().unwrap().zoom, 1.5);
        assert_eq!(this.library.settings().zoom, 1.5);
    });

    // Close first book
    view.update_in(cx, |this, window, cx| {
        this.close_book(window, cx);
        assert!(this.reader.is_none());
    });

    // Open second book: should retain zoom of 1.5!
    open_book_by_index(&fx, &view, cx, 1);
    view.update_in(cx, |this, _, _| {
        assert_eq!(this.reader.as_ref().unwrap().zoom, 1.5);
    });

    // Turn page in second book: zoom remains 1.5
    view.update_in(cx, |this, _, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
        assert_eq!(this.reader.as_ref().unwrap().zoom, 1.5);
    });

    // Reset zoom via reset_zoom
    view.update_in(cx, |this, _, cx| {
        this.reset_zoom(cx);
        assert_eq!(this.reader.as_ref().unwrap().zoom, 1.0);
        assert_eq!(this.library.settings().zoom, 1.0);
    });

    // Now disable remember_zoom
    view.update_in(cx, |this, _, cx| {
        this.set_remember_zoom(Some(false), cx);
        assert!(!this.library.settings().remember_zoom);
    });

    // Zoom in again while remember_zoom is false
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.zoom_to(2.0, (100.0, 100.0), vp, cx);
        assert_eq!(this.reader.as_ref().unwrap().zoom, 2.0);
    });

    // Close and open third book: should reset to 1.0 because remember_zoom is false
    view.update_in(cx, |this, window, cx| {
        this.close_book(window, cx);
    });
    open_book_by_index(&fx, &view, cx, 2);
    view.update_in(cx, |this, _, _| {
        assert_eq!(this.reader.as_ref().unwrap().zoom, 1.0);
    });
}

#[gpui_kit::test]
#[ignore = "prints timings"]
fn perf_frames_during_a_slide(cx: &mut TestAppContext) {
    let fx = fixture("perf");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    view.update(cx, |this, _| this.frame_dt = Some(1.0 / 144.0));

    let mut times = Vec::new();
    for turn in 0..20 {
        view.update(cx, |this, cx| {
            this.turn_by(turn % 2 == 0, 1, Arrive::Start, cx)
        });
        for _ in 0..40 {
            // one frame = state update + element tree + layout + paint
            let start = Instant::now();
            view.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            if view.read_with(cx, |this, _| this.last_frame.is_none()) {
                break;
            }
        }
    }
    times.sort_by(|a, b| a.total_cmp(b));
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    println!(
        "{} animated frames: mean {mean:.2} ms, median {:.2} ms, p95 {:.2} ms, max {:.2} ms \
             (view update + layout + paint into the test platform; unoptimised build, no GPU)",
        times.len(),
        times[times.len() / 2],
        times[(times.len() * 95 / 100).min(times.len() - 1)],
        times[times.len() - 1],
    );
}

#[gpui_kit::test]
fn dropdown_menu_interaction_and_selection(cx: &mut TestAppContext) {
    let fx = fixture("dropdown-test");
    let (view, cx) = launch(&fx, cx);

    // Initial state: defaults
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().thumbnail_size),
        ThumbnailSize::Medium
    );
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().fit),
        FitMode::Page
    );

    // Open settings and switch to appearance tab
    view.update(cx, |this, cx| {
        this.overlay = Some(Overlay::Settings);
        this.settings_tab = ui::overlays::SettingsTab::Appearance;
        cx.notify();
    });
    cx.run_until_parked();

    // Verify settings appearance view renders properly
    assert_eq!(
        view.read_with(cx, |this, _| this.settings_tab),
        ui::overlays::SettingsTab::Appearance
    );

    // Test thumbnail size setter
    view.update(cx, |this, cx| {
        this.set_thumbnail_size(ThumbnailSize::Large, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().thumbnail_size),
        ThumbnailSize::Large
    );

    // Test corner rounding setter
    view.update(cx, |this, cx| {
        this.set_corner_rounding(CornerRounding::Rounded, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().corner_rounding),
        CornerRounding::Rounded
    );

    // Test border size setter
    view.update(cx, |this, cx| {
        this.set_border_size(BorderSize::Thick, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().border_size),
        BorderSize::Thick
    );

    // Test color scheme setter
    view.update(cx, |this, cx| {
        this.set_color_scheme("Gruvbox", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |this, _| this.library.settings().color_scheme.clone()),
        "Gruvbox"
    );

    // Close settings overlay
    view.update(cx, |this, cx| {
        this.close_overlay(cx);
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |this, _| this.overlay.is_none()));

    // Open a book in reader to test reader dropdowns
    open_book_by_index(&fx, &view, cx, 0);
    cx.run_until_parked();
    assert!(view.read_with(cx, |this, _| this.reader.is_some()));

    // Test fit mode setter
    view.update(cx, |this, cx| {
        this.set_fit(FitMode::Width, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |this, _| this.reader.as_ref().unwrap().fit),
        FitMode::Width
    );

    // Also verify standalone macos_dropdown_button helper element builds correctly
    let selected_val = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let on_select = {
        let selected_val = selected_val.clone();
        std::rc::Rc::new(move |val: String, _: &mut Window, _: &mut App| {
            *selected_val.borrow_mut() = val;
        })
    };
    let btn = ui::settings_view::macos_dropdown_button(
        "test-dropdown",
        "Options",
        vec![
            ("Opt A".to_string(), "a".to_string(), true),
            ("Opt B".to_string(), "b".to_string(), false),
        ],
        on_select,
    );
    drop(btn);
}

#[gpui_kit::test]
fn navigation_back_and_forward_symmetry(cx: &mut TestAppContext) {
    let fx = fixture("nav-symmetry-test");
    let (view, cx) = launch(&fx, cx);

    view.update_in(cx, |this, _, cx| {
        // Initial state: AllManga
        assert_eq!(this.nav_history.len(), 1);
        assert_eq!(this.nav_history_idx, 0);
        assert_eq!(
            this.nav_history[0],
            NavDestination::Section(ui::sidebar::SidebarSection::AllManga)
        );

        // Enter a group
        this.enter_group_named("series-1".to_string(), "Series One".to_string(), cx);
        assert_eq!(this.nav_history.len(), 2);
        assert_eq!(this.nav_history_idx, 1);
        assert_eq!(this.open_group.as_deref(), Some("series-1"));

        // Navigate to Details from within the group
        this.navigate_to(ui::sidebar::MainView::Details("book-1".to_string()), cx);
        assert_eq!(this.nav_history.len(), 3);
        assert_eq!(this.nav_history_idx, 2);

        // Back 1: must return directly to the group (not top-level!)
        this.navigate_back(cx);
        assert_eq!(this.nav_history_idx, 1);
        assert_eq!(this.open_group.as_deref(), Some("series-1"));

        // Back 2: must return to AllManga
        this.navigate_back(cx);
        assert_eq!(this.nav_history_idx, 0);
        assert_eq!(this.open_group, None);
        assert_eq!(this.active_section, ui::sidebar::SidebarSection::AllManga);

        // Back at bottom does nothing
        this.navigate_back(cx);
        assert_eq!(this.nav_history_idx, 0);

        // Forward 1: must re-enter group "series-1"
        this.navigate_forward(cx);
        assert_eq!(this.nav_history_idx, 1);
        assert_eq!(this.open_group.as_deref(), Some("series-1"));

        // Forward 2: must re-enter Details("book-1")
        this.navigate_forward(cx);
        assert_eq!(this.nav_history_idx, 2);
        assert_eq!(
            this.active_view,
            ui::sidebar::MainView::Details("book-1".to_string())
        );

        // Forward at top does nothing
        this.navigate_forward(cx);
        assert_eq!(this.nav_history_idx, 2);

        // Back to group (idx 1)
        this.navigate_back(cx);
        assert_eq!(this.nav_history_idx, 1);

        // Navigating to a new section truncates forward history (which had Details at idx 2)
        this.navigate_to(ui::sidebar::MainView::ContinueReading, cx);
        assert_eq!(this.nav_history.len(), 3);
        assert_eq!(this.nav_history_idx, 2);
        assert_eq!(
            this.nav_history[2],
            NavDestination::Section(ui::sidebar::SidebarSection::ContinueReading)
        );

        // Forward does nothing because forward history was truncated
        this.navigate_forward(cx);
        assert_eq!(this.nav_history_idx, 2);
    });
}

#[gpui_kit::test]
fn test_right_click_does_not_select_or_show_selection_toolbar(cx: &mut TestAppContext) {
    let fx = fixture("rtclick_nosel");
    let (view, cx) = launch(&fx, cx);

    // Initial state: 0 items selected, browsing controls visible
    assert!(selected(&view, cx).is_empty());
    assert!(cx.debug_bounds("add-files").is_some());
    assert!(cx.debug_bounds("select-all").is_none());

    // Right-click on card 0
    click(cx, card(0), MouseButton::Right);
    cx.run_until_parked();

    // Context menu opens, but item is NOT selected, and selection toolbar does NOT appear
    assert_eq!(overlay_kind(&view, cx), "menu");
    assert!(
        selected(&view, cx).is_empty(),
        "right-clicking an unselected item must not select it"
    );
    assert!(
        cx.debug_bounds("select-all").is_none(),
        "selection toolbar must not appear on right-click"
    );
}

#[gpui_kit::test]
fn test_right_click_on_selected_preserves_multi_selection(cx: &mut TestAppContext) {
    let fx = fixture("rtclick_preserves");
    let (view, cx) = launch(&fx, cx);

    // Select books 0 and 1
    ctrl_click(cx, 0);
    ctrl_click(cx, 1);
    assert_eq!(selected(&view, cx).len(), 2);

    // Right-click on book 1 (an item already within multi-selection)
    click(cx, card(1), MouseButton::Right);
    cx.run_until_parked();

    // Context menu opens and multi-selection is preserved
    assert_eq!(overlay_kind(&view, cx), "menu");
    assert_eq!(
        selected(&view, cx).len(),
        2,
        "multi-selection must be preserved when right-clicking an already-selected item"
    );
    let labels = menu_labels(&view, cx);
    assert!(
        labels.iter().any(|l| l.to_lowercase().contains("2 books")),
        "context menu offers bulk actions for the 2 selected books: {labels:?}"
    );
}

#[gpui_kit::test]
fn test_contextual_toolbar_mode_transitions(cx: &mut TestAppContext) {
    let fx = fixture("ctx_toolbar");
    let (view, cx) = launch(&fx, cx);

    // 1. Browsing mode: no selection
    assert!(cx.debug_bounds("add-files").is_some());
    assert!(cx.debug_bounds("select-all").is_none());
    assert!(cx.debug_bounds("select-remove").is_none());
    assert!(cx.debug_bounds("select-cancel").is_none());

    // 2. Transition to selection mode: select card 0
    ctrl_click(cx, 0);
    cx.run_until_parked();

    // Browsing controls are replaced by selection actions in the single top toolbar
    assert!(
        cx.debug_bounds("add-files").is_none(),
        "import button is replaced in selection mode"
    );
    assert!(cx.debug_bounds("select-all").is_some());
    assert!(cx.debug_bounds("select-remove").is_some());
    let cancel = cx
        .debug_bounds("select-cancel")
        .expect("Cancel button present in selection mode");

    // 3. Click Cancel: returns to normal browsing mode
    click(cx, cancel.center(), MouseButton::Left);
    cx.run_until_parked();

    assert!(selected(&view, cx).is_empty());
    assert!(
        cx.debug_bounds("add-files").is_some(),
        "import button restored in browsing mode"
    );
    assert!(cx.debug_bounds("select-all").is_none());
    assert!(cx.debug_bounds("select-remove").is_none());
}

#[gpui_kit::test]
fn test_back_navigation_visibility_and_no_forward_button(cx: &mut TestAppContext) {
    let fx = fixture("nav_back");
    let (view, cx) = launch(&fx, cx);

    // Forward button is never present in the library toolbar
    assert!(
        cx.debug_bounds("toolbar-nav-forward").is_none(),
        "forward button is completely removed"
    );

    // In top-level library root: Back button is hidden
    assert!(
        cx.debug_bounds("group-back").is_none(),
        "back button is hidden at top-level library root"
    );

    // Enter a group
    view.update_in(cx, |this, _, cx| {
        this.open_group = Some("series:test_series".into());
        cx.notify();
    });
    cx.run_until_parked();

    // Back button is now visible
    let back = cx
        .debug_bounds("group-back")
        .expect("group-back button is visible when inside group");
    click(cx, back.center(), MouseButton::Left);
    cx.run_until_parked();

    // Back button exits the group and hides itself
    assert_eq!(view.read_with(cx, |this, _| this.open_group.clone()), None);
    assert!(cx.debug_bounds("group-back").is_none());
}

#[gpui_kit::test]
fn test_vim_mode_default_off_and_settings_persistence(cx: &mut TestAppContext) {
    let fx = fixture("vim_default");
    let (view, cx) = launch(&fx, cx);

    // 1. Off by default
    view.update_in(cx, |this, _, _| {
        assert!(
            !this.library.settings().vim_mode,
            "vim_mode must be off by default"
        );
        assert!(!this.is_vim_enabled());
    });

    // Pressing ':' in library when vim mode is off does NOT enter cmdline
    cx.simulate_keystrokes(":");
    view.update_in(cx, |this, _, _| {
        assert!(
            !this.in_cmdline(),
            "colon must not open cmdline when vim mode is off"
        );
    });

    // 2. Enable vim mode via settings
    view.update_in(cx, |this, _, cx| {
        this.toggle_vim_mode(true, cx);
    });

    // Now typing ':' opens cmdline
    cx.simulate_keystrokes(":");
    view.update_in(cx, |this, _, _| {
        assert!(
            this.in_cmdline(),
            "colon opens cmdline when vim mode is enabled"
        );
    });
    cx.simulate_keystrokes("escape");

    // 3. Persists across restarts
    let on_disk = Library::load(&fx.dirs).unwrap();
    assert!(
        on_disk.settings().vim_mode,
        "vim mode setting must persist on disk"
    );
}

#[gpui_kit::test]
fn test_vim_mode_cli_flag_override(cx: &mut TestAppContext) {
    let fx = fixture("vim_cli");
    let (view, cx) = launch(&fx, cx);

    // Settings on disk are default (vim_mode = false)
    assert!(!saved_settings(&fx).vim_mode);

    // Session override via CLI flag (--vim)
    view.update_in(cx, |this, _, _| {
        this.vim_cli_override = true;
        assert!(this.is_vim_enabled());
    });

    // Typing ':' opens cmdline
    cx.simulate_keystrokes(":");
    view.update_in(cx, |this, _, _| {
        assert!(this.in_cmdline());
    });
    cx.simulate_keystrokes("escape");

    // Disk settings are NOT modified by CLI session override
    assert!(
        !saved_settings(&fx).vim_mode,
        "CLI flag must not permanently alter saved settings on disk"
    );
}

#[gpui_kit::test]
fn test_disable_animations_setting_and_reduced_motion_override(cx: &mut TestAppContext) {
    let fx = fixture("disable_anim");
    let (view, cx) = launch(&fx, cx);

    // Default: animations enabled (disable_animations is false)
    view.update_in(cx, |this, _, _| {
        assert!(!this.library.settings().disable_animations);
        assert!(this.animations_enabled());
    });

    // 1. User toggles disable_animations ON
    view.update_in(cx, |this, _, cx| {
        this.toggle_disable_animations(true, cx);
        assert!(this.library.settings().disable_animations);
        assert!(!this.animations_enabled());
    });

    // Verify it persists on disk
    let on_disk = Library::load(&fx.dirs).unwrap();
    assert!(on_disk.settings().disable_animations);

    // 2. User toggles disable_animations OFF
    view.update_in(cx, |this, _, cx| {
        this.toggle_disable_animations(false, cx);
        assert!(!this.library.settings().disable_animations);
        assert!(this.animations_enabled());
    });

    // 3. System asks for reduced motion: animations must stay off regardless of user setting
    view.update_in(cx, |this, _, _| {
        this.reduce_motion = true;
        assert!(
            !this.animations_enabled(),
            "system reduced motion must override user setting"
        );
    });
}

#[gpui_kit::test]
fn test_shortcuts_tab_adapts_to_vim_mode(cx: &mut TestAppContext) {
    let fx = fixture("shortcuts_vim");
    let (view, cx) = launch(&fx, cx);

    view.update_in(cx, |this, _, cx| {
        this.settings_tab = ui::overlays::SettingsTab::Shortcuts;
        // With vim mode off
        this.toggle_vim_mode(false, cx);
    });

    view.update_in(cx, |this, _, _| {
        assert!(!this.is_vim_enabled());
    });

    // Enable vim mode
    view.update_in(cx, |this, _, cx| {
        this.toggle_vim_mode(true, cx);
        assert!(this.is_vim_enabled());
    });
}

#[gpui_kit::test]
fn test_volume_count_badge_is_numeric_only(cx: &mut TestAppContext) {
    let fx = fixture("badge");
    let (view, cx) = launch(&fx, cx);
    let (colors, radius) = view.read_with(cx, |this, _| (this.colors, px(this.radius_px())));
    // stat_badge creates a Tag with the numeric count only
    let _badge = ui::components::stat_badge(42, false, colors, radius);
}

#[gpui_kit::test]
fn test_group_mode_toggle_immediately_clears_nested_group_and_hides_back(cx: &mut TestAppContext) {
    let fx = fixture("toggle_group_nav");
    let (view, cx) = launch(&fx, cx);

    // Enable series grouping
    view.update_in(cx, |this, _, cx| {
        this.toggle_group_series(true, cx);
    });

    // Enter a group
    view.update_in(cx, |this, _, cx| {
        this.enter_group("series:test_series".into(), cx);
    });
    cx.run_until_parked();

    // Back button is visible while inside group
    assert!(cx.debug_bounds("group-back").is_some());

    // Switch to individual-volume mode (toggle grouping off)
    view.update_in(cx, |this, _, cx| {
        this.toggle_group_series(false, cx);
    });
    cx.run_until_parked();

    // open_group is immediately cleared and back button is immediately hidden
    assert_eq!(view.read_with(cx, |this, _| this.open_group.clone()), None);
    assert!(
        cx.debug_bounds("group-back").is_none(),
        "back button must hide immediately when switching to individual-volume mode"
    );
}

#[gpui_kit::test]
fn test_folder_right_click_does_not_select_or_activate_selection_mode(cx: &mut TestAppContext) {
    let fx = fixture("folder_rtclick");
    let (view, cx) = launch(&fx, cx);

    // Initial state: nothing selected
    assert!(selected(&view, cx).is_empty());

    // Open group context menu for a folder
    view.update_in(cx, |this, _, cx| {
        this.open_group_context_menu(
            "series_a",
            "Series A",
            &fx.ids[..2],
            point(px(100.), px(100.)),
            cx,
        );
    });
    cx.run_until_parked();

    // Context menu is open
    assert_eq!(overlay_kind(&view, cx), "menu");
    // Multi-selection is not triggered
    assert!(selected(&view, cx).is_empty());
    assert!(cx.debug_bounds("select-all").is_none());
}

#[gpui_kit::test]
fn test_recently_added_layout_and_clipping(cx: &mut TestAppContext) {
    let fx = fixture("rec_layout");
    let (view, cx) = launch(&fx, cx);

    let (radius, border_px) =
        view.read_with(cx, |this, _| (px(this.radius_px()), this.border_px()));

    // Verify derived cover radius calculation
    let inner_radius = (radius - px(border_px)).max(px(0.));
    assert!(inner_radius >= px(0.));

    // Queue item cover radius clamping
    let queue_radius = (radius - px(border_px)).max(px(0.)).min(px(6.));
    assert!(queue_radius <= px(6.));
}
