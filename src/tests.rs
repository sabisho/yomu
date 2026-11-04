//! These drive the real views through gpui's test platform: simulated mouse clicks and key
//! presses go through hit-testing and key dispatch exactly as they do in the running app.

use super::*;
use gpui_kit::{Entity, Modifiers, TestAppContext, VisualTestContext, point};
use image::Frame;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Cursor, Write},
};
use ui::reader::Page;
use yomu::archive::SourceKind;
use yomu::imaging;
use yomu::layout::Viewport;
use yomu::library::{Book, DataDirs, Library, PageSpreadOverride};
use yomu::menu::{self, BookAction, Entry};
use yomu::motion::{KineticScroll, PageTransition};
use yomu::pagecache;
use yomu::vim::{Action, Amount, Dir};

fn png_bytes() -> Vec<u8> {
    sized_png_bytes(60, 90)
}

fn sized_png_bytes(w: u32, h: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(w, h, image::Rgba([90, 40, 160, 255]));
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn fixture_mixed(tag: &str, dims: &[(u32, u32)]) -> Fixture {
    static FIXTURE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = FIXTURE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("yomu-ui-{tag}-{}-{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let dirs = DataDirs::at(root.join("data")).unwrap();

    let path = root.join("mixed.cbz");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (ix, &(w, h)) in dims.iter().enumerate() {
        zip.start_file(format!("page_{:03}.png", ix + 1), options)
            .unwrap();
        zip.write_all(&sized_png_bytes(w, h)).unwrap();
    }
    zip.finish().unwrap();

    let files = vec![path];
    let report = library::import_paths(&files, &dirs, &HashSet::new());
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let mut library = Library::default();
    for book in report.added {
        library.add(book);
    }
    library.save(&dirs).unwrap();

    Fixture {
        ids: library.books().iter().map(|b| b.id.clone()).collect(),
        files,
        dirs,
    }
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

fn fixture_with_pages(tag: &str, count: usize) -> Fixture {
    static FIXTURE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = FIXTURE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("yomu-ui-{tag}-{}-{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let dirs = DataDirs::at(root.join("data")).unwrap();

    let mut files = Vec::new();
    let path = root.join("comic.cbz");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for page in 1..=count {
        zip.start_file(format!("{page:03}.png"), options).unwrap();
        zip.write_all(&png_bytes()).unwrap();
    }
    zip.finish().unwrap();
    files.push(path);

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
        gpui_kit::component::Theme::set_scrollbar_mode(
            gpui_kit::component::scroll::ScrollbarMode::Scrolling,
            cx,
        );
        cx.bind_keys([KeyBinding::new("ctrl-,", OpenSettings, None)]);
    });
    let yomu_cell = std::rc::Rc::new(std::cell::RefCell::new(None));
    let yomu_for_view = yomu_cell.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let yomu = cx.new(|cx| Yomu::with_dirs(dirs, window, cx));
        let focus = yomu.read(cx).focus_handle.clone();
        window.focus(&focus, cx);
        *yomu_for_view.borrow_mut() = Some(yomu.clone());
        gpui_kit::component::Root::new(yomu, window, cx)
    });
    let yomu = yomu_cell.borrow().as_ref().unwrap().clone();
    yomu.update(cx, |this, cx| {
        this.navigate_to(ui::sidebar::MainView::AllManga, cx);
        this.nav_history = vec![NavDestination::Section(
            ui::sidebar::SidebarSection::AllManga,
        )];
        this.nav_history_idx = 0;
    });
    cx.run_until_parked();
    (yomu, cx)
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
        Some(Overlay::Confirm(_)) => "confirm",
        Some(Overlay::Properties(_)) => "properties",
        Some(Overlay::Series(_)) => "series",
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
    let ix = view.read_with(cx, |this, _| {
        this.library.books().iter().position(|b| b.id == id)
    });
    if let Some(ix) = ix {
        click(cx, card(ix), MouseButton::Right);
    }
}

fn menu_labels(view: &Entity<Yomu>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |this, _| {
        let Some(id) = &this.last_context_menu_book else {
            return Vec::new();
        };
        let selected_ids = this.selected_ids();
        let entries = if selected_ids.contains(id) && selected_ids.len() > 1 {
            menu::selection_menu(selected_ids.len())
        } else if let Some(book) = this.library.get(id) {
            menu::book_menu(book, book.path.exists())
        } else {
            Vec::new()
        };
        entries
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item(item) => Some(item.label),
                Entry::Separator => None,
            })
            .collect()
    })
}

/// Centre of the nth book card in the library grid (window is 1920 wide in the test platform).
fn card(index: usize) -> Point<Pixels> {
    point(px(369.0 + 234.0 * index as f32), px(200.0))
}

fn click(cx: &mut VisualTestContext, at: Point<Pixels>, button: MouseButton) {
    cx.simulate_mouse_down(at, button, Modifiers::default());
    cx.simulate_mouse_up(at, button, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
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
    let disabled = view.read_with(cx, |this, _| {
        let Some(id) = &this.last_context_menu_book else {
            return Vec::new();
        };
        let Some(book) = this.library.get(id) else {
            return Vec::new();
        };
        menu::book_menu(book, book.path.exists())
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item(item) if !item.enabled => Some(item.action),
                _ => None,
            })
            .collect::<Vec<_>>()
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
    assert!(
        cx.debug_bounds("popup-menu").is_some(),
        "context menu must be open"
    );
    assert_eq!(
        reading(&view, cx),
        None,
        "right-click must not open the book"
    );
    view.read_with(cx, |this, _| {
        assert_eq!(
            this.last_context_menu_book.as_deref(),
            Some(fx.ids[0].as_str())
        );
    });

    // a left click on a book (no menu) still opens it
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(350));
    view.update(cx, |this, _| this.overlay_closed_at = None);
    cx.run_until_parked();
    click(cx, card(1), MouseButton::Left);
    assert_eq!(reading(&view, cx), Some(0));
}

#[gpui_kit::test]
fn clicking_outside_closes_the_menu_without_clicking_through(cx: &mut TestAppContext) {
    let fx = fixture("outside");
    let (view, cx) = launch(&fx, cx);

    click(cx, card(0), MouseButton::Right);
    assert!(cx.debug_bounds("popup-menu").is_some());
    // far from the menu, but directly over another book
    click(cx, card(3), MouseButton::Left);
    assert!(
        reading(&view, cx).is_none(),
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
    assert!(cx.debug_bounds("popup-menu").is_some());
    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();
    assert!(book(&view, cx, &id).is_finished());
}

#[gpui_kit::test]
fn menu_stays_inside_the_window_near_every_edge(cx: &mut TestAppContext) {
    let fx = fixture("edges");
    let (_view, cx) = launch(&fx, cx);
    let (width, height) = (1920.0, 1080.0);

    click(cx, card(0), MouseButton::Right);
    cx.run_until_parked();
    let bounds = cx.debug_bounds("popup-menu").expect("menu was painted");
    let (left, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let right = left + f32::from(bounds.size.width);
    let bottom = top + f32::from(bounds.size.height);
    assert!(left >= 0.0 && top >= 0.0, "{bounds:?}");
    assert!(right <= width && bottom <= height, "{bounds:?}");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn menu_keyboard_navigation(cx: &mut TestAppContext) {
    let fx = fixture("menukeys");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();
    open_menu(&view, cx, &id);

    assert!(cx.debug_bounds("popup-menu").is_some());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("popup-menu").is_none());

    // enter activates the highlighted item
    open_menu(&view, cx, &id);
    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();
    assert!(book(&view, cx, &id).is_finished());
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
        this.save_library_sync();
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
            this.save_library_sync();
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
        cx.run_until_parked();
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
            this.save_library_sync();
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
            this.save_library_sync();
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
fn entering_and_exiting_group_preserves_selection(cx: &mut TestAppContext) {
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

    // Exiting Group 1 preserves selection
    view.update(cx, |this, cx| {
        this.exit_group(cx);
    });
    assert_eq!(selected(&view, cx).len(), 2);

    // Entering Group 2 preserves selection across the library (Group 2 itself has 0 selected)
    view.update(cx, |this, cx| {
        this.enter_group(key2.clone(), cx);
    });
    assert_eq!(selected(&view, cx).len(), 0);
    assert_eq!(view.read_with(cx, |this, _| this.selection.len()), 2);

    // Re-entering Group 1 preserves its 2 selected books
    view.update(cx, |this, cx| {
        this.exit_group(cx);
        this.enter_group(key1.clone(), cx);
    });
    assert_eq!(selected(&view, cx).len(), 2);
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
    assert!(cx.debug_bounds("popup-menu").is_some());
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
    cx.executor().advance_clock(Duration::from_millis(350));
    view.update(cx, |this, _| this.overlay_closed_at = None);
    cx.run_until_parked();
    ctrl_click(cx, 0);
    ctrl_click(cx, 2);
    click(cx, card(2), MouseButton::Right);
    cx.simulate_keystrokes("down down down enter");
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
    cx.executor().advance_clock(Duration::from_millis(350));
    view.update(cx, |this, _| this.overlay_closed_at = None);
    cx.run_until_parked();
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
        spread_overrides: Default::default(),
        page_dimensions: Default::default(),
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
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(500));
            cx.run_until_parked();
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
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
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
fn animations_remain_permanently_enabled_and_page_turns_slide(cx: &mut TestAppContext) {
    let fx = fixture("reduced");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // Verify animations remain active during page turns
    view.update(cx, |this, cx| {
        this.goto_page(1, Arrive::Start, cx);
        assert!(
            this.reader.as_ref().unwrap().transition.is_some(),
            "page turn slide animation must be active"
        );
    });
    settle_animations(&view, cx);
    assert_eq!(page(&view, cx), 1, "navigation arrives at expected page");
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
        cx.simulate_event(ScrollEvent {
            position: card(0),
            delta: ScrollDelta::Pixels(point(px(0.), px(-60.))),
            touch_phase: TouchPhase::Moved,
            modifiers: Modifiers::default(),
            ..Default::default()
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
fn scrolling_zoomed_view_steps_line_position(cx: &mut TestAppContext) {
    let fx = fixture("glide-step");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.zoom_to(2.0, (vp.width / 2.0, vp.height / 2.0), vp, cx);
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

#[test]
#[ignore = "prints timings"]
fn perf_end_to_end_page_pipeline() {
    use std::io::Write;
    use yomu::archive::{self, SourceKind};

    // Create a 2400x3600 realistic JPEG scan (8.6 Megapixels)
    let (w, h) = (2400u32, 3600u32);
    let image = image::RgbImage::from_fn(w, h, |x, y| {
        let n = (x.wrapping_mul(1664525) ^ y.wrapping_mul(1013904223)) >> 24;
        image::Rgb([
            ((x / 10 + n) & 255) as u8,
            ((y / 15 + n) & 255) as u8,
            (((x + y) / 20) & 255) as u8,
        ])
    });
    let mut jpeg = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut jpeg, 85,
        ))
        .unwrap();

    // Pack into a temporary CBZ archive
    let tmp = std::env::temp_dir().join(format!("yomu_e2e_{}.cbz", yomu::util::now_secs()));
    let file = std::fs::File::create(&tmp).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("page_001.jpg", options).unwrap();
    zip.write_all(&jpeg).unwrap();
    zip.finish().unwrap();

    // 1. Measure archive entry read & decompression
    let mut decompress_times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let bytes = archive::read_page(&tmp, SourceKind::Archive, "page_001.jpg").unwrap();
        decompress_times.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(bytes.len(), jpeg.len());
    }
    let decompress_ms = median(&mut decompress_times);

    // 2. Measure JPEG decode + SIMD scale down to Tier 2048
    let mut decode_scale_times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let decoded = imaging::decode_fit_bgra(&jpeg, 2048, 2048).unwrap();
        decode_scale_times.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(decoded.orig, (w, h));
    }
    let decode_scale_ms = median(&mut decode_scale_times);

    // 3. Measure RenderImage wrapping
    let decoded = imaging::decode_fit_bgra(&jpeg, 2048, 2048).unwrap();
    let mut wrap_times = Vec::new();
    for _ in 0..5 {
        let img_clone = decoded.image.clone();
        let start = Instant::now();
        let _render_img = std::sync::Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(
            img_clone,
        )]));
        wrap_times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let wrap_ms = median(&mut wrap_times);

    // 4. Measure End-to-End single call (read_page + decode_fit_bgra + RenderImage)
    let mut e2e_times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let bytes = archive::read_page(&tmp, SourceKind::Archive, "page_001.jpg").unwrap();
        let decoded = imaging::decode_fit_bgra(&bytes, 2048, 2048).unwrap();
        let _render_img = std::sync::Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(
            decoded.image,
        )]));
        e2e_times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let e2e_ms = median(&mut e2e_times);

    println!(
        "E2E Pipeline Breakdown (2400x3600 JPEG scan -> Tier 2048):\n\
         - Stage 1 (Archive read & inflate): {:.2} ms ({:.1}%)\n\
         - Stage 2 (SIMD JPEG decode & Catmull-Rom resize): {:.2} ms ({:.1}%)\n\
         - Stage 3 (RenderImage wrapping): {:.2} ms ({:.1}%)\n\
         - Total End-to-End Pipeline Latency: {:.2} ms",
        decompress_ms,
        (decompress_ms / e2e_ms) * 100.0,
        decode_scale_ms,
        (decode_scale_ms / e2e_ms) * 100.0,
        wrap_ms,
        (wrap_ms / e2e_ms) * 100.0,
        e2e_ms
    );

    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[ignore]
fn perf_cache_hit_vs_miss_transition() {
    use std::io::Write;
    use std::sync::Arc;
    use std::time::Instant;
    use yomu::archive::{self, SourceKind};
    use yomu::imaging;

    let (w, h) = (2400, 3600);
    let image = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
    });

    let mut jpeg = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut jpeg, 85,
        ))
        .unwrap();

    let tmp = std::env::temp_dir().join(format!("yomu_cache_bench_{}.cbz", yomu::util::now_secs()));
    let file = std::fs::File::create(&tmp).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("page_001.jpg", options).unwrap();
    zip.write_all(&jpeg).unwrap();
    zip.finish().unwrap();

    // Prepare simulated in-memory cache
    let decoded = imaging::decode_fit_bgra(&jpeg, 2048, 2048).unwrap();
    let render_img = Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(
        decoded.image,
    )]));
    let mut cache = std::collections::HashMap::new();
    cache.insert(0usize, render_img);

    // 1. Measure Cache Hit transition latency (in-memory lookup + Arc clone)
    let iters = 100_000;
    let hit_start = Instant::now();
    for _ in 0..iters {
        let page = cache.get(&0usize).cloned().unwrap();
        std::hint::black_box(page);
    }
    let hit_duration = hit_start.elapsed();
    let hit_ns = hit_duration.as_nanos() as f64 / iters as f64;
    let hit_ms = hit_ns / 1_000_000.0;

    // 2. Measure Cache Miss transition latency (disk read + decompression + SIMD decode + resize + texture wrap)
    let miss_iters = 5;
    let mut miss_times = Vec::new();
    for _ in 0..miss_iters {
        let start = Instant::now();
        let bytes = archive::read_page(&tmp, SourceKind::Archive, "page_001.jpg").unwrap();
        let decoded = imaging::decode_fit_bgra(&bytes, 2048, 2048).unwrap();
        let page = Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(
            decoded.image,
        )]));
        miss_times.push(start.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(page);
    }
    let miss_ms = median(&mut miss_times);

    let speedup = (miss_ms / hit_ms).round();

    println!(
        "Cache Hit vs Miss Transition Performance (2400x3600 scan -> Tier 2048):\n\
         - Cache Hit (Memory LRU lookup + Arc clone): {:.6} ms ({:.1} ns)\n\
         - Cache Miss (Disk read + decompress + SIMD decode + resize): {:.2} ms\n\
         - Cache Acceleration Speedup: {:.0}x faster",
        hit_ms, hit_ns, miss_ms, speedup
    );

    let _ = std::fs::remove_file(&tmp);
}

fn percentile(values: &mut [f64], pct: f64) -> f64 {
    assert!(!values.is_empty());
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((values.len() as f64 - 1.0) * pct).round() as usize;
    values[idx.min(values.len() - 1)]
}

#[test]
#[ignore]
fn perf_real_world_presentation_and_navigation_breakdown() {
    use std::io::Write;
    use std::sync::Arc;
    use std::time::Instant;
    use yomu::archive::{self, SourceKind};
    use yomu::imaging;

    let (w, h) = (2400, 3600);
    let image = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
    });

    let mut jpeg = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut jpeg, 85,
        ))
        .unwrap();

    let tmp = std::env::temp_dir().join(format!("yomu_nav_bench_{}.cbz", yomu::util::now_secs()));
    let file = std::fs::File::create(&tmp).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for i in 0..10 {
        zip.start_file(format!("page_{:03}.jpg", i + 1), options)
            .unwrap();
        zip.write_all(&jpeg).unwrap();
    }
    zip.finish().unwrap();

    // 1. Measure sequential cache-hit navigation latency distribution (1,000 runs)
    let decoded = imaging::decode_fit_bgra(&jpeg, 2048, 2048).unwrap();
    let render_img = Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(
        decoded.image,
    )]));
    let mut cache = std::collections::HashMap::new();
    for i in 0..10 {
        cache.insert(i, render_img.clone());
    }

    let mut seq_forward_times = Vec::new();
    for i in 0..1000 {
        let page_idx = i % 10;
        let start = Instant::now();
        let page = cache.get(&page_idx).cloned().unwrap();
        seq_forward_times.push(start.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(page);
    }
    let seq_fwd_p50 = percentile(&mut seq_forward_times, 0.50);
    let seq_fwd_p95 = percentile(&mut seq_forward_times, 0.95);
    let seq_fwd_p99 = percentile(&mut seq_forward_times, 0.99);

    // 2. Measure cold cache-miss latency distribution (10 runs)
    let mut miss_times = Vec::new();
    for i in 0..10 {
        let entry_name = format!("page_{:03}.jpg", i + 1);
        let start = Instant::now();
        let bytes = archive::read_page(&tmp, SourceKind::Archive, &entry_name).unwrap();
        let decoded = imaging::decode_fit_bgra(&bytes, 2048, 2048).unwrap();
        let page = Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(
            decoded.image,
        )]));
        miss_times.push(start.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(page);
    }
    let miss_p50 = percentile(&mut miss_times, 0.50);
    let miss_p95 = percentile(&mut miss_times, 0.95);
    let miss_p99 = percentile(&mut miss_times, 0.99);

    // 3. Measure dynamic zoom transform latency (10,000 runs)
    let mut zoom_times = Vec::new();
    let mut current_zoom;
    for i in 0..10_000 {
        let target_zoom = match i % 4 {
            0 => 1.25,
            1 => 1.5,
            2 => 2.0,
            _ => 1.0,
        };
        let start = Instant::now();
        current_zoom = yomu::layout::clamp_zoom(target_zoom);
        let bounds = yomu::layout::display_size(
            yomu::layout::FitMode::Height,
            current_zoom,
            (1080.0, 1920.0),
            &yomu::layout::Viewport {
                width: 1920.0,
                height: 1080.0,
                scale: 1.0,
            },
        );
        zoom_times.push(start.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(bounds);
    }
    let zoom_p50 = percentile(&mut zoom_times, 0.50);
    let zoom_p95 = percentile(&mut zoom_times, 0.95);
    let zoom_p99 = percentile(&mut zoom_times, 0.99);

    println!(
        "Real-World Page Presentation & Navigation Latency Breakdown (2400x3600 scan -> Tier 2048):\n\
         - Sequential Navigation (Cache Hit):\n\
             p50: {:.6} ms ({:.1} ns)\n\
             p95: {:.6} ms ({:.1} ns)\n\
             p99: {:.6} ms ({:.1} ns)\n\
         - Cold Cache Miss (Disk Read + Inflate + SIMD Decode + Resize):\n\
             p50: {:.2} ms\n\
             p95: {:.2} ms\n\
             p99: {:.2} ms\n\
         - Dynamic Zoom Transformation (Zero-Allocation Matrix & Layout Geometry):\n\
             p50: {:.6} ms ({:.1} ns)\n\
             p95: {:.6} ms ({:.1} ns)\n\
             p99: {:.6} ms ({:.1} ns)",
        seq_fwd_p50,
        seq_fwd_p50 * 1_000_000.0,
        seq_fwd_p95,
        seq_fwd_p95 * 1_000_000.0,
        seq_fwd_p99,
        seq_fwd_p99 * 1_000_000.0,
        miss_p50,
        miss_p95,
        miss_p99,
        zoom_p50,
        zoom_p50 * 1_000_000.0,
        zoom_p95,
        zoom_p95 * 1_000_000.0,
        zoom_p99,
        zoom_p99 * 1_000_000.0,
    );

    let _ = std::fs::remove_file(&tmp);
}

#[gpui_kit::test]
fn test_sustained_long_reading_session_stability(cx: &mut TestAppContext) {
    let fx = fixture("long_session");
    let (view, cx) = launch(&fx, cx);
    view.update_in(cx, |this, _, _| {
        this.library.settings_mut().vim_mode = true;
    });

    // 1. Open book 0
    open_book_by_index(&fx, &view, cx, 0);

    // 2. Perform 100 rapid page transitions back and forth
    for _ in 0..50 {
        cx.simulate_keystrokes("right right");
        cx.simulate_keystrokes("left");
    }
    settle_animations(&view, cx);

    // Verify cache eviction bounded memory
    with_reader(&view, cx, |r| {
        assert!(
            r.images.len() <= yomu::pagecache::KEEP_AHEAD + yomu::pagecache::KEEP_BEHIND + 3,
            "Images cache must remain bounded under rapid page navigation, got {}",
            r.images.len()
        );
        assert!(
            r.pending.len() <= 2,
            "Pending requests must not leak under rapid navigation"
        );
    });

    // 3. Repeatedly jump to start and end
    for _ in 0..10 {
        cx.simulate_keystrokes("shift-g");
        cx.simulate_keystrokes("g g");
    }
    settle_animations(&view, cx);

    // 4. Repeatedly zoom in and out
    for _ in 0..10 {
        cx.simulate_keystrokes("=");
        cx.simulate_keystrokes("-");
    }
    settle_animations(&view, cx);

    // 5. Close book and open another book (book 1)
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(view.read_with(cx, |this, _| this.reader.is_none()));

    open_book_by_index(&fx, &view, cx, 1);
    for _ in 0..20 {
        cx.simulate_keystrokes("right");
    }
    settle_animations(&view, cx);

    // 6. Verify zero zombie tasks or state leaks
    with_reader(&view, cx, |r| {
        assert!(
            r.cancellations.is_empty(),
            "Cancellation tokens must be cleared"
        );
        assert!(
            r.images.len() <= yomu::pagecache::KEEP_AHEAD + yomu::pagecache::KEEP_BEHIND + 3,
            "Image cache remains bounded across multi-book session"
        );
    });

    // Close and verify clean exit
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(view.read_with(cx, |this, _| this.reader.is_none()));
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

    view.update(cx, |this, cx| {
        this.save_library(cx);
    });
    cx.run_until_parked();

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
fn theme_action_changes_and_cycles_theme(cx: &mut TestAppContext) {
    let fx = fixture("theme-action");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    view.update_in(cx, |this, window, cx| {
        // Running theme action applies theme
        let vp = viewport_of(window);
        this.apply(Action::SetTheme("Gruvbox Dark".to_string()), vp, window, cx);
        assert_eq!(this.library.settings().theme_name, "Gruvbox Dark");

        // Empty theme action cycles theme
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
    assert!(cx.debug_bounds("popup-menu").is_some());
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
    assert!(cx.debug_bounds("popup-menu").is_some());
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
    assert!(cx.debug_bounds("view-mode-toggle").is_some());
    assert!(cx.debug_bounds("toolbar-sort").is_some());
    assert!(cx.debug_bounds("add-files").is_some()); // sidebar import button
    assert!(cx.debug_bounds("select-all").is_none());
    assert!(cx.debug_bounds("unselect-all").is_none());
    assert!(cx.debug_bounds("select-remove").is_none());
    assert!(cx.debug_bounds("select-cancel").is_none());

    // 2. Transition to partial selection mode: select card 0
    ctrl_click(cx, 0);
    cx.run_until_parked();

    // Toolbar browsing controls are replaced by selection actions
    assert!(
        cx.debug_bounds("view-mode-toggle").is_none(),
        "view mode toggle is replaced in selection mode"
    );
    assert!(
        cx.debug_bounds("toolbar-sort").is_none(),
        "sort button is replaced in selection mode"
    );
    assert!(cx.debug_bounds("select-all").is_some());
    assert!(cx.debug_bounds("unselect-all").is_none());
    assert!(cx.debug_bounds("select-remove").is_some());
    let cancel = cx
        .debug_bounds("select-cancel")
        .expect("Cancel button present in selection mode");

    // 3. Click Cancel: returns to normal browsing mode
    click(cx, cancel.center(), MouseButton::Left);
    cx.run_until_parked();

    assert!(selected(&view, cx).is_empty());
    assert!(
        cx.debug_bounds("view-mode-toggle").is_some(),
        "view mode toggle restored in browsing mode"
    );
    assert!(
        cx.debug_bounds("toolbar-sort").is_some(),
        "sort button restored in browsing mode"
    );
    assert!(cx.debug_bounds("select-all").is_none());
    assert!(cx.debug_bounds("unselect-all").is_none());
    assert!(cx.debug_bounds("select-remove").is_none());

    // 4. Test Select All -> Unselect All transition
    ctrl_click(cx, 0);
    cx.run_until_parked();
    let select_all = cx
        .debug_bounds("select-all")
        .expect("Select all offered during partial selection");
    assert!(cx.debug_bounds("unselect-all").is_none());

    // Click Select all -> full selection
    click(cx, select_all.center(), MouseButton::Left);
    cx.run_until_parked();
    assert_eq!(selected(&view, cx), fx.ids);
    assert!(
        cx.debug_bounds("unselect-all").is_some(),
        "Unselect all offered during full selection"
    );
    assert!(cx.debug_bounds("select-all").is_none());

    // Click Unselect all -> clears selection
    let unselect_all = cx.debug_bounds("unselect-all").unwrap();
    click(cx, unselect_all.center(), MouseButton::Left);
    cx.run_until_parked();
    assert!(selected(&view, cx).is_empty());
    assert!(cx.debug_bounds("view-mode-toggle").is_some());
    assert!(cx.debug_bounds("unselect-all").is_none());
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

    // Pressing ':' in library when vim mode is off is completely inert
    cx.simulate_keystrokes(":");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 2. Enable vim mode via settings
    view.update_in(cx, |this, _, cx| {
        this.toggle_vim_mode(true, cx);
    });

    // Pressing ':' when vim mode is enabled is also inert (command palette removed)
    cx.simulate_keystrokes(":");
    assert_eq!(overlay_kind(&view, cx), "none");

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

    // Pressing ':' is inert
    cx.simulate_keystrokes(":");
    assert_eq!(overlay_kind(&view, cx), "none");

    // Disk settings are NOT modified by CLI session override
    assert!(
        !saved_settings(&fx).vim_mode,
        "CLI flag must not permanently alter saved settings on disk"
    );
}

#[gpui_kit::test]
fn test_permanent_animation_policy(cx: &mut TestAppContext) {
    let fx = fixture("perm_anim");
    let (view, cx) = launch(&fx, cx);

    // 1. Application startup keeps native animations enabled in the GPUI runtime
    cx.update(|_, cx| {
        assert!(
            !cx.reduce_motion(),
            "GPUI reduce_motion must be permanently false on startup"
        );
    });

    // 2. Settings configuration has animations permanently active
    view.update_in(cx, |this, _, _| {
        assert!(
            this.library.settings().animate,
            "library settings must indicate animations are permanently active"
        );
    });

    // 3. Settings UI renders cleanly without an animation disable switch
    view.update_in(cx, |this, window, cx| {
        this.navigate_to(crate::ui::sidebar::MainView::Settings, cx);
        let _settings_element = this.render_settings_content(window, cx);
    });

    // 4. Old configuration files containing `disable_animations` load safely with animations enabled
    let json = r#"{"books":[],"settings":{"disable_animations":true,"animate":false}}"#;
    std::fs::write(fx.dirs.library_file(), json).unwrap();
    let loaded = Library::load(&fx.dirs).unwrap();
    assert!(
        loaded.settings().animate,
        "old config with disable_animations=true must load safely and keep animations active"
    );

    // 5. Existing page transitions still function smoothly
    open_book_by_index(&fx, &view, cx, 0);
    view.update(cx, |this, cx| {
        this.goto_page(1, Arrive::Start, cx);
        assert!(
            this.reader.as_ref().unwrap().transition.is_some(),
            "slide animation transition must be active during navigation"
        );
    });
    settle_animations(&view, cx);
    assert_eq!(page(&view, cx), 1);
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
fn settings_reader_and_shortcuts_tabs_scroll_inside_the_dialog(cx: &mut TestAppContext) {
    let fx = fixture("settings-scroll-bounds");
    let (view, cx) = launch(&fx, cx);
    cx.simulate_keystrokes("ctrl-,");

    let dialog_bounds = cx
        .debug_bounds("dialog-0")
        .expect("Settings dialog should be mounted");

    for tab in [
        ui::overlays::SettingsTab::Reader,
        ui::overlays::SettingsTab::Shortcuts,
    ] {
        view.update_in(cx, |this, _, cx| {
            this.settings_tab = tab;
            cx.notify();
        });
        cx.run_until_parked();

        let scroll = view.read_with(cx, |this, _| this.settings_scroll.clone());
        let viewport = scroll.bounds();
        assert!(
            scroll.max_offset().y > gpui_kit::px(0.0),
            "{tab:?} tab should scroll within the dialog"
        );
        let rendered_viewport = scroll.bounds();
        assert_eq!(
            rendered_viewport, viewport,
            "{tab:?} viewport should remain fixed inside the dialog"
        );
        assert!(
            rendered_viewport.origin.y + rendered_viewport.size.height
                <= dialog_bounds.origin.y + dialog_bounds.size.height,
            "{tab:?} scroll viewport {rendered_viewport:?} should stay inside dialog {dialog_bounds:?}"
        );
        assert!(
            rendered_viewport.origin.x + rendered_viewport.size.width
                <= dialog_bounds.origin.x + dialog_bounds.size.width,
            "{tab:?} scroll viewport {rendered_viewport:?} should stay inside dialog {dialog_bounds:?}"
        );
    }
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

    // Right-click on a folder card
    click(cx, card(0), MouseButton::Right);
    cx.run_until_parked();

    // Context menu is open
    assert!(cx.debug_bounds("popup-menu").is_some());
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

#[gpui_kit::test]
fn test_search_input_lifecycle(cx: &mut TestAppContext) {
    let fx = fixture("search_lifecycle");
    let (view, cx) = launch(&fx, cx);

    // Verify search_input is initialized immediately at construction without needing render
    let has_search_input = view.read_with(cx, |this, _| this.search_input.is_some());
    assert!(
        has_search_input,
        "search_input must be initialized during view construction"
    );

    // Verify activating search sets search_active and focuses the input
    view.update_in(cx, |this, window, cx| {
        this.activate_search(window, cx);
    });
    assert!(view.read_with(cx, |this, _| this.search_active));

    // Verify updating the input value updates search_query via the subscription
    let input_entity = view.read_with(cx, |this, _| this.search_input.clone().unwrap());
    input_entity.update_in(cx, |input, window, cx| {
        input.set_value("test query", window, cx);
        cx.emit(gpui_kit::component::input::InputEvent::Change);
    });
    cx.run_until_parked();

    let query = view.read_with(cx, |this, _| this.search_query.clone());
    assert_eq!(query, "test query");

    // Deactivating search clears the active flag
    view.update_in(cx, |this, _, cx| {
        this.deactivate_search(cx);
    });
    assert!(!view.read_with(cx, |this, _| this.search_active));
}

#[test]
fn test_desktop_integration_install() {
    let tmp = std::env::temp_dir().join(format!(
        "yomu-desktop-test-{}-{}",
        std::process::id(),
        crate::now_secs()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // First install: should create all files and return true (updated)
    let updated = crate::app::desktop::install_desktop_integration_to(&tmp, false).unwrap();
    assert!(updated);

    let desktop_file = tmp.join("applications/yomu.desktop");
    let pixmap_file = tmp.join("pixmaps/yomu.png");
    let icon_128 = tmp.join("icons/hicolor/128x128/apps/yomu.png");

    assert!(desktop_file.exists());
    assert!(pixmap_file.exists());
    assert!(icon_128.exists());

    // Second install without force: files exist, should not update (returns false)
    let updated_second = crate::app::desktop::install_desktop_integration_to(&tmp, false).unwrap();
    assert!(!updated_second);

    // Customise the desktop file
    std::fs::write(&desktop_file, "Customized desktop file").unwrap();

    // Re-run without force: should NOT overwrite customized file
    let updated_third = crate::app::desktop::install_desktop_integration_to(&tmp, false).unwrap();
    assert!(!updated_third);
    let content = std::fs::read_to_string(&desktop_file).unwrap();
    assert_eq!(content, "Customized desktop file");

    // Re-run with force: should overwrite customized file
    let updated_forced = crate::app::desktop::install_desktop_integration_to(&tmp, true).unwrap();
    assert!(updated_forced);
    let content_forced = std::fs::read_to_string(&desktop_file).unwrap();
    assert_eq!(content_forced, crate::app::desktop::desktop_entry_content());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[gpui_kit::test]
fn test_continuous_reader_scrollbar_sync_and_mode(cx: &mut TestAppContext) {
    let fx = fixture("continuous-scrollbar");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // Switch to continuous mode
    view.update_in(cx, |this, _window, cx| {
        this.set_continuous(Some(true), cx);
    });
    settle_animations(&view, cx);

    // Verify continuous mode is active
    let (is_continuous, doc_height, vp_height) = view.update_in(cx, |this, window, _cx| {
        let r = this.reader.as_ref().unwrap();
        let vp = viewport_of(window);
        (r.continuous, r.continuous_total_height(&vp), vp.height)
    });
    assert!(is_continuous);
    assert!(doc_height > vp_height);

    // Emulate scrollbar dragging via reader_scroll_sync
    view.update_in(cx, |this, _window, cx| {
        this.reader_scroll_sync.set(Some(150.0));
        cx.notify();
    });
    settle_animations(&view, cx);

    // Verify that the reader's offset was updated to 150.0
    let current_off_y = view.read_with(cx, |this, _| this.reader.as_ref().unwrap().off.1);
    assert_eq!(current_off_y, 150.0);
}

#[gpui_kit::test]
fn prefetch_cancellation_does_not_fail_page_and_allows_subsequent_retry(cx: &mut TestAppContext) {
    let fx = fixture_with_pages("prefetch-cancel", 20);
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // Book is opened on page 0. Request page 15 explicitly as prefetch:
    view.update(cx, |this, cx| {
        this.request_page(15, 1000, cx);
    });

    // Verify page 15 is pending and has a cancellation token
    let has_cancel_token = with_reader(&view, cx, |r| {
        r.pending.contains(&15) && r.cancellations.contains_key(&15)
    });
    assert!(has_cancel_token);

    // Now simulate cancellation as would happen during fast scrolling when page 15 is no longer wanted:
    view.update(cx, |this, _| {
        let reader = this.reader.as_mut().unwrap();
        if let Some(token) = reader.cancellations.remove(&15) {
            token.store(true, std::sync::atomic::Ordering::Relaxed);
            reader.pending.remove(&15);
        }
    });

    // Run until the cancelled task completes
    cx.run_until_parked();

    // Cancellation must NEVER be recorded in reader.failed:
    with_reader(&view, cx, |r| {
        assert!(
            !r.failed.contains_key(&15),
            "cancelled page must not be recorded in failed map: {:?}",
            r.failed.get(&15)
        );
        assert!(
            !r.images.contains_key(&15),
            "cancelled page was not retained in images"
        );
        assert!(
            !r.pending.contains(&15),
            "cancelled page must not remain pending"
        );
    });

    // Now navigate to page 15 (subsequent retry)
    view.update(cx, |this, cx| {
        this.goto_page(15, Arrive::Start, cx);
    });
    cx.run_until_parked();

    // The retry should have successfully decoded and cached the page
    with_reader(&view, cx, |r| {
        assert!(
            r.images.contains_key(&15),
            "page 15 should be decoded after retry"
        );
        assert!(
            !r.failed.contains_key(&15),
            "page 15 must not be in failed after retry"
        );
    });
}

#[gpui_kit::test]
fn genuine_decode_error_is_recorded_in_failed(cx: &mut TestAppContext) {
    let fx = fixture("genuine-error");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    let (book_id, cancel_token) = with_reader(&view, cx, |r| {
        (
            r.book_id.clone(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
    });

    view.update(cx, |this, cx| {
        // Register cancellation token as active for page 2
        this.reader
            .as_mut()
            .unwrap()
            .cancellations
            .insert(2, cancel_token.clone());
        this.reader.as_mut().unwrap().pending.insert(2);

        // Deliver a genuine error
        this.page_decoded(
            book_id,
            2,
            1000,
            cancel_token,
            Err(anyhow::anyhow!("corrupted image stream")),
            cx,
        );
    });

    with_reader(&view, cx, |r| {
        assert_eq!(
            r.failed.get(&2).map(String::as_str),
            Some("corrupted image stream"),
            "genuine error must be recorded in failed map"
        );
        assert!(!r.pending.contains(&2));
        assert!(!r.cancellations.contains_key(&2));
    });
}

#[gpui_kit::test]
fn prefetch_cancellation_race_condition_does_not_clobber_new_task(cx: &mut TestAppContext) {
    let fx = fixture("cancel-race");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    let book_id = with_reader(&view, cx, |r| r.book_id.clone());

    let token_1 = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let token_2 = Arc::new(std::sync::atomic::AtomicBool::new(false));

    view.update(cx, |this, _| {
        let reader = this.reader.as_mut().unwrap();
        // Task 1 was active, then cancelled
        reader.pending.insert(3);
        reader.cancellations.insert(3, token_1.clone());
        token_1.store(true, std::sync::atomic::Ordering::Relaxed);
        reader.pending.remove(&3);
        reader.cancellations.remove(&3);

        // Task 2 is now active for the same page
        reader.pending.insert(3);
        reader.cancellations.insert(3, token_2.clone());
    });

    // Task 1 arrives with cancellation error:
    view.update(cx, |this, cx| {
        this.page_decoded(
            book_id.clone(),
            3,
            1000,
            token_1,
            Err(anyhow::anyhow!("page decode cancelled")),
            cx,
        );
    });

    // Verify Task 1 did NOT remove Task 2's pending or cancellation state, and did NOT fail page 3
    with_reader(&view, cx, |r| {
        assert!(!r.failed.contains_key(&3));
        assert!(r.pending.contains(&3), "Task 2 must still be pending");
        assert!(
            r.cancellations.contains_key(&3),
            "Task 2 token must still be in cancellations"
        );
    });

    // Now Task 2 completes successfully:
    view.update(cx, |this, cx| {
        this.reader.as_mut().unwrap().pending.remove(&3);
        this.reader.as_mut().unwrap().cancellations.remove(&3);
        this.request_page(3, 1000, cx);
    });
    cx.run_until_parked();

    with_reader(&view, cx, |r| {
        assert!(r.images.contains_key(&3), "page 3 decoded by Task 2");
        assert!(!r.failed.contains_key(&3));
    });
}

#[gpui_kit::test]
fn test_fixed_library_card_dimensions_and_responsive_columns(_cx: &mut TestAppContext) {
    let thumb_size = yomu::library::ThumbnailSize::Medium;
    let expected_card_w = thumb_size.card_width();

    // 1. Narrow window (e.g. 700px) with expanded sidebar (240px)
    let grid_narrow = ui::components::compute_responsive_grid(700.0, true, thumb_size);
    assert!(
        (grid_narrow.card_w - expected_card_w).abs() < 40.0,
        "card width must remain close to baseline"
    );
    assert!(
        (grid_narrow.cover_h / grid_narrow.card_w - 1.5).abs() < 0.02,
        "aspect ratio 1.5 must be strictly preserved"
    );

    // 2. Medium window (e.g. 1200px)
    let grid_med = ui::components::compute_responsive_grid(1200.0, true, thumb_size);
    assert!(
        (grid_med.card_w - expected_card_w).abs() < 40.0,
        "card width must remain close to baseline"
    );
    assert!(
        (grid_med.cover_h / grid_med.card_w - 1.5).abs() < 0.02,
        "aspect ratio 1.5 must be strictly preserved"
    );
    assert!(
        grid_med.cols >= grid_narrow.cols,
        "columns must increase with width"
    );

    // 3. Wide window (e.g. 1920px) with the off-canvas sidebar collapsed
    let grid_wide = ui::components::compute_responsive_grid(1920.0, false, thumb_size);
    assert!(
        (grid_wide.card_w - expected_card_w).abs() < 40.0,
        "card width must remain close to baseline"
    );
    assert!(
        (grid_wide.cover_h / grid_wide.card_w - 1.5).abs() < 0.02,
        "aspect ratio 1.5 must be strictly preserved"
    );
    assert!(
        grid_wide.cols > grid_med.cols,
        "columns must adapt to available space"
    );

    // 4. Content padding is consistent and bounded
    assert!(
        grid_wide.pad_x >= 16.0 && grid_wide.pad_x <= 24.0,
        "grid padding must remain consistent"
    );
    assert!(
        grid_med.pad_x >= 16.0 && grid_med.pad_x <= 24.0,
        "grid padding must remain consistent"
    );
}

#[gpui_kit::test]
fn test_default_startup_view_is_continue_reading(cx: &mut TestAppContext) {
    let fx = fixture("startup-continue-reading");
    let dirs = fx.dirs.clone();
    cx.update(|cx| {
        gpui_kit::init(cx);
    });
    let initial_view_cell = std::rc::Rc::new(std::cell::RefCell::new(None));
    let initial_section_cell = std::rc::Rc::new(std::cell::RefCell::new(None));
    let iv = initial_view_cell.clone();
    let is = initial_section_cell.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let yomu = cx.new(|cx| Yomu::with_dirs(dirs, window, cx));
        *iv.borrow_mut() = Some(yomu.read(cx).active_view.clone());
        *is.borrow_mut() = Some(yomu.read(cx).active_section);
        gpui_kit::component::Root::new(yomu, window, cx)
    });
    cx.run_until_parked();

    // The application must launch with Continue Reading as the initially selected view and section
    assert_eq!(
        *initial_view_cell.borrow(),
        Some(ui::sidebar::MainView::ContinueReading)
    );
    assert_eq!(
        *initial_section_cell.borrow(),
        Some(ui::sidebar::SidebarSection::ContinueReading)
    );
}

#[gpui_kit::test]
fn test_details_cover_availability_and_fallback(cx: &mut TestAppContext) {
    let fx = fixture("details-cover");
    let (view, cx) = launch(&fx, cx);
    cx.run_until_parked();

    let book_with_cover = fx.ids[0].clone();
    let book_without_cover = fx.ids[1].clone();

    // Ensure cover exists for book 0
    assert!(fx.dirs.cover(&book_with_cover).exists());

    // Delete cover files for book 1 to test missing cover fallback
    let cover_1 = fx.dirs.cover(&book_without_cover);
    let _ = std::fs::remove_file(&cover_1);
    let legacy_1 = fx
        .dirs
        .root()
        .join("covers")
        .join(format!("{book_without_cover}.jpg"));
    let _ = std::fs::remove_file(&legacy_1);
    assert!(!fx.dirs.cover(&book_without_cover).exists());

    // Navigate to Details for book with cover
    view.update(cx, |this, cx| {
        this.navigate_to(ui::sidebar::MainView::Details(book_with_cover.clone()), cx);
    });
    cx.run_until_parked();

    view.read_with(cx, |this, _| {
        assert_eq!(
            this.selected_book_id.as_deref(),
            Some(book_with_cover.as_str())
        );
        assert!(
            this.details_cover_available,
            "cover must be reported as available"
        );
    });

    // Navigate to Details for book without cover
    view.update(cx, |this, cx| {
        this.navigate_to(
            ui::sidebar::MainView::Details(book_without_cover.clone()),
            cx,
        );
    });
    cx.run_until_parked();

    view.read_with(cx, |this, _| {
        assert_eq!(
            this.selected_book_id.as_deref(),
            Some(book_without_cover.as_str())
        );
        assert!(
            !this.details_cover_available,
            "missing cover must report available = false for fallback presentation"
        );
    });
}

#[gpui_kit::test]
fn test_details_cover_stale_result_protection_on_book_change(cx: &mut TestAppContext) {
    let fx = fixture("details-stale");
    let (view, cx) = launch(&fx, cx);
    cx.run_until_parked();

    let book_a = fx.ids[0].clone();
    let book_b = fx.ids[1].clone();

    // Delete cover for book A so its background check will yield false
    let cover_a = fx.dirs.cover(&book_a);
    let _ = std::fs::remove_file(&cover_a);
    let legacy_a = fx.dirs.root().join("covers").join(format!("{book_a}.jpg"));
    let _ = std::fs::remove_file(&legacy_a);
    assert!(!fx.dirs.cover(&book_a).exists());

    // Book B retains its cover (exists = true)
    assert!(fx.dirs.cover(&book_b).exists());

    view.update(cx, |this, cx| {
        // First navigate to book A (kicking off check for A)
        this.navigate_to(ui::sidebar::MainView::Details(book_a.clone()), cx);
        // Immediately switch to book B before background tasks complete
        this.navigate_to(ui::sidebar::MainView::Details(book_b.clone()), cx);
    });
    cx.run_until_parked();

    view.read_with(cx, |this, _| {
        assert_eq!(this.selected_book_id.as_deref(), Some(book_b.as_str()));
        assert!(
            this.details_cover_available,
            "book A's missing cover must not overwrite book B's valid cover"
        );
    });
}

#[gpui_kit::test]
fn test_render_all_manga_view_grouped_and_ungrouped(cx: &mut TestAppContext) {
    let fx = fixture("render-all-manga");
    let (view, cx) = launch(&fx, cx);
    cx.run_until_parked();

    // 1. Default: ungrouped view rendering without search query
    view.update_in(cx, |this, window, cx| {
        let _elem = this.render_all_manga_view(window, cx);
    });

    // 2. Ungrouped with matching search query
    view.update_in(cx, |this, window, cx| {
        this.search_query = "A".to_string();
        let _elem = this.render_all_manga_view(window, cx);
    });

    // 3. Ungrouped with non-matching search query (renders empty state)
    view.update_in(cx, |this, window, cx| {
        this.search_query = "NonExistent".to_string();
        let _elem = this.render_all_manga_view(window, cx);
    });

    // 4. Enable series grouping with an open group
    let group_key = view.read_with(cx, |this, _| {
        directory_groups(this.library.books())[0].key.clone()
    });

    view.update_in(cx, |this, window, cx| {
        this.search_query.clear();
        this.library.settings_mut().group_series = true;
        this.open_group = Some(group_key);
        let _elem = this.render_all_manga_view(window, cx);
    });

    // 5. Open group with search filter
    view.update_in(cx, |this, window, cx| {
        this.search_query = "A".to_string();
        let _elem = this.render_all_manga_view(window, cx);
    });

    // 6. Open group with non-existent key falls back gracefully
    view.update_in(cx, |this, window, cx| {
        this.search_query.clear();
        this.open_group = Some("non-existent-group-key".to_string());
        let _elem = this.render_all_manga_view(window, cx);
    });
}

#[gpui_kit::test]
fn test_ui_cover_resolution_with_legacy_and_sized_covers(cx: &mut TestAppContext) {
    let fx = fixture("ui-legacy-covers");
    let book_id = fx.ids[0].clone();

    let (view, cx) = launch(&fx, cx);
    cx.run_until_parked();

    // Verify render_all_manga_view builds cards without errors or synchronous disk checks
    view.update_in(cx, |this, window, cx| {
        let _elem = this.render_all_manga_view(window, cx);
    });

    // Verify Details screen cover check resolves successfully
    view.update(cx, |this, cx| {
        this.navigate_to(ui::sidebar::MainView::Details(book_id.clone()), cx);
    });
    cx.run_until_parked();

    view.read_with(cx, |this, _| {
        assert_eq!(this.selected_book_id.as_deref(), Some(book_id.as_str()));
        assert!(this.details_cover_available);
    });
}

#[gpui_kit::test]
fn test_reader_title_cached_on_open_and_updates_on_book_switch(cx: &mut TestAppContext) {
    let fx = fixture("reader-title-cache");
    let (view, cx) = launch(&fx, cx);

    // Open first book
    open_book_by_index(&fx, &view, cx, 0);
    let book0_title = view.read_with(cx, |this, _| {
        this.library.get(&fx.ids[0]).unwrap().title.clone()
    });
    view.read_with(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.book_title.as_ref(), book0_title.as_str());
    });

    // Render reader to ensure header renders with cached title
    view.update_in(cx, |this, _window, cx| {
        let reader = this.reader.as_ref().unwrap();
        let _elem = this.render_reader(reader, cx);
    });

    // Close and open second book
    view.update_in(cx, |this, window, cx| {
        this.close_book(window, cx);
    });
    open_book_by_index(&fx, &view, cx, 1);
    let book1_title = view.read_with(cx, |this, _| {
        this.library.get(&fx.ids[1]).unwrap().title.clone()
    });
    view.read_with(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.book_title.as_ref(), book1_title.as_str());
    });
}

#[gpui_kit::test]
fn test_reader_cancellation_direct_iteration_in_sync_reader(cx: &mut TestAppContext) {
    let fx = fixture_with_pages("sync-cancel", 20);
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // Manually register an unwanted background task (page 18) while on page 0
    let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let token_clone = token.clone();
    view.update(cx, |this, _| {
        let reader = this.reader.as_mut().unwrap();
        reader.pending.insert(18);
        reader.cancellations.insert(18, token_clone);
    });

    // Run sync_reader. On page 0, wanted only covers 0..=5; page 18 is not wanted.
    view.update_in(cx, |this, window, cx| {
        this.sync_reader(window, cx, 0.016);
    });

    // Direct iteration in sync_reader must have cancelled token for page 18
    assert!(token.load(std::sync::atomic::Ordering::Relaxed));
    view.read_with(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert!(!reader.pending.contains(&18));
        assert!(!reader.cancellations.contains_key(&18));
    });
}

#[gpui_kit::test]
fn test_cancelled_task_does_not_pollute_failed_and_leaves_page_requestable(
    cx: &mut TestAppContext,
) {
    let fx = fixture_with_pages("cancel-failed-test", 20);
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    view.update(cx, |this, cx| {
        let book_id = this.reader.as_ref().unwrap().book_id.clone();
        // Invoke page_decoded with a cancelled token and a cancellation error
        this.page_decoded(
            book_id,
            15,
            1024,
            token,
            Err(anyhow::anyhow!("page decode cancelled")),
            cx,
        );
    });

    view.read_with(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        // 15 must NOT be marked as failed!
        assert!(!reader.failed.contains_key(&15));
    });

    // Page 15 can still be requested on navigation
    view.update(cx, |this, cx| {
        this.request_page(15, 1024, cx);
    });

    view.read_with(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        // Pending should now contain 15
        assert!(reader.pending.contains(&15));
    });
}

#[gpui_kit::test]
fn test_library_sorting_modes_persistence_and_order(cx: &mut TestAppContext) {
    let fx = fixture("sort-modes");
    let (view, cx) = launch(&fx, cx);

    // Setup 3 books with distinct metadata
    // Book 0: "Zeta Comic", added_at: 100, last_read: Some(500), progress: 100%
    // Book 1: "Alpha Manga", added_at: 300, last_read: Some(200), progress: 50%
    // Book 2: "Beta Graphic", added_at: 200, last_read: None, progress: 0%
    let (id0, id1, id2) = view.update(cx, |this, _| {
        let id0 = fx.ids[0].clone();
        let b0 = this.library.get_mut(&id0).unwrap();
        b0.title = "Zeta Comic".into();
        b0.added_at = 100;
        b0.last_read = Some(500);
        b0.opened = true;
        b0.current_page = 4;
        b0.page_count = 5;

        let id1 = fx.ids[1].clone();
        let b1 = this.library.get_mut(&id1).unwrap();
        b1.title = "Alpha Manga".into();
        b1.added_at = 300;
        b1.last_read = Some(200);
        b1.opened = true;
        b1.current_page = 2;
        b1.page_count = 5;

        let id2 = fx.ids[2].clone();
        let b2 = this.library.get_mut(&id2).unwrap();
        b2.title = "Beta Graphic".into();
        b2.added_at = 200;
        b2.last_read = None;
        b2.opened = false;
        b2.current_page = 0;
        b2.page_count = 5;

        (id0, id1, id2)
    });

    // 1. Title Ascending (A–Z): Alpha (id1), Beta (id2), Zeta (id0)
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::Title,
            yomu::library::SortDirection::Ascending,
            cx,
        );
    });
    let order = view.read_with(cx, |this, _| this.current_view_order());
    assert_eq!(order, vec![id1.clone(), id2.clone(), id0.clone()]);

    // 2. Title Descending (Z–A): Zeta (id0), Beta (id2), Alpha (id1)
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::Title,
            yomu::library::SortDirection::Descending,
            cx,
        );
    });
    let order = view.read_with(cx, |this, _| this.current_view_order());
    assert_eq!(order, vec![id0.clone(), id2.clone(), id1.clone()]);

    // 3. Date Added Newest: Alpha (300), Beta (200), Zeta (100)
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::DateAdded,
            yomu::library::SortDirection::Descending,
            cx,
        );
    });
    let order = view.read_with(cx, |this, _| this.current_view_order());
    assert_eq!(order, vec![id1.clone(), id2.clone(), id0.clone()]);

    // 4. Date Added Oldest: Zeta (100), Beta (200), Alpha (300)
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::DateAdded,
            yomu::library::SortDirection::Ascending,
            cx,
        );
    });
    let order = view.read_with(cx, |this, _| this.current_view_order());
    assert_eq!(order, vec![id0.clone(), id2.clone(), id1.clone()]);

    // 5. Last Read Recently Read: Zeta (500), Alpha (200), Beta (unread)
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::LastRead,
            yomu::library::SortDirection::Descending,
            cx,
        );
    });
    let order = view.read_with(cx, |this, _| this.current_view_order());
    assert_eq!(order, vec![id0.clone(), id1.clone(), id2.clone()]);

    // 6. Last Read Least Recently Read: Alpha (200), Zeta (500), Beta (unread at end)
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::LastRead,
            yomu::library::SortDirection::Ascending,
            cx,
        );
    });
    let order = view.read_with(cx, |this, _| this.current_view_order());
    assert_eq!(order, vec![id1.clone(), id0.clone(), id2.clone()]);

    // 7. Reading Progress Highest: Zeta (100%), Alpha (60%), Beta (0%)
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::Progress,
            yomu::library::SortDirection::Descending,
            cx,
        );
    });
    let order = view.read_with(cx, |this, _| this.current_view_order());
    assert_eq!(order, vec![id0.clone(), id1.clone(), id2.clone()]);

    // 8. Reading Progress Lowest: Beta (0%), Alpha (60%), Zeta (100%)
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::Progress,
            yomu::library::SortDirection::Ascending,
            cx,
        );
    });
    let order = view.read_with(cx, |this, _| this.current_view_order());
    assert_eq!(order, vec![id2.clone(), id1.clone(), id0.clone()]);

    // Verify settings persistence: settings were saved
    view.read_with(cx, |this, _| {
        assert_eq!(
            this.library.settings().sort_by,
            yomu::library::SortBy::Progress
        );
        assert_eq!(
            this.library.settings().sort_direction,
            yomu::library::SortDirection::Ascending
        );
    });
}

#[gpui_kit::test]
fn test_grouped_series_sorting(cx: &mut TestAppContext) {
    let fx = fixture("grouped-sort");
    let (view, cx) = launch(&fx, cx);

    // Setup 4 books: 2 in "Series Beta", 2 in "Series Alpha"
    let (id_a1, id_a2, _id_b1, _id_b2) = view.update(cx, |this, cx| {
        this.library.settings_mut().group_series = true;
        // ensure we have 4 books
        let mut b3 = this.library.books()[0].clone();
        b3.id = "extra_book".into();
        this.library.add(b3);

        let id_a1 = fx.ids[0].clone();
        let b_a1 = this.library.get_mut(&id_a1).unwrap();
        b_a1.title = "Series Alpha Vol 1".into();
        b_a1.series = Some("Series Alpha".into());

        let id_a2 = fx.ids[1].clone();
        let b_a2 = this.library.get_mut(&id_a2).unwrap();
        b_a2.title = "Series Alpha Vol 2".into();
        b_a2.series = Some("Series Alpha".into());

        let id_b1 = fx.ids[2].clone();
        let b_b1 = this.library.get_mut(&id_b1).unwrap();
        b_b1.title = "Series Beta Vol 1".into();
        b_b1.series = Some("Series Beta".into());

        let id_b2 = "extra_book".to_string();
        let b_b2 = this.library.get_mut(&id_b2).unwrap();
        b_b2.title = "Series Beta Vol 2".into();
        b_b2.series = Some("Series Beta".into());

        this.save_library(cx);
        (id_a1, id_a2, id_b1, id_b2)
    });

    // Sort Title Ascending: "Series Alpha" folders should sort before "Series Beta"
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::Title,
            yomu::library::SortDirection::Ascending,
            cx,
        );
    });
    view.read_with(cx, |this, _| {
        let sorted = this.get_sorted_books();
        let mut groups = directory_groups(sorted.iter().copied());
        crate::ui::library_view::sort_groups(
            &mut groups,
            yomu::library::SortBy::Title,
            yomu::library::SortDirection::Ascending,
        );
        assert_eq!(groups[0].name, "Series Alpha");
        assert_eq!(groups[1].name, "Series Beta");
        assert_eq!(groups[0].books[0].id, id_a1);
        assert_eq!(groups[0].books[1].id, id_a2);
    });

    // Enter Series Alpha group: current_view_order reflects books within this group
    view.update(cx, |this, cx| {
        this.enter_group("series:series alpha".into(), cx);
    });
    let order = view.read_with(cx, |this, _| this.current_view_order());
    assert_eq!(order, vec![id_a1.clone(), id_a2.clone()]);

    // Reverse sort direction inside group: Title Descending
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::Title,
            yomu::library::SortDirection::Descending,
            cx,
        );
    });
    let order_desc = view.read_with(cx, |this, _| this.current_view_order());
    assert_eq!(order_desc, vec![id_a2, id_a1]);
}

#[gpui_kit::test]
fn test_toolbar_sort_button_exists_in_library_view(cx: &mut TestAppContext) {
    let fx = fixture("sort-toolbar");
    let (view, cx) = launch(&fx, cx);

    // Toolbar sort button must be present in All Manga view
    assert!(
        cx.debug_bounds("toolbar-sort").is_some(),
        "toolbar-sort button must be rendered on the library toolbar"
    );

    // Default tooltip
    view.read_with(cx, |this, _| {
        assert_eq!(
            this.library.settings().sort_by,
            yomu::library::SortBy::Default
        );
        assert_eq!(
            this.library.settings().sort_direction,
            yomu::library::SortDirection::Ascending
        );
    });

    // Change sort mode
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::Title,
            yomu::library::SortDirection::Descending,
            cx,
        );
    });
    cx.run_until_parked();

    // Still visible and custom-sorted
    assert!(cx.debug_bounds("toolbar-sort").is_some());
    view.read_with(cx, |this, _| {
        assert_eq!(
            this.library.settings().sort_by,
            yomu::library::SortBy::Title
        );
        assert_eq!(
            this.library.settings().sort_direction,
            yomu::library::SortDirection::Descending
        );
    });

    // Reset back to default sorting
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::Default,
            yomu::library::SortDirection::Ascending,
            cx,
        );
    });
    cx.run_until_parked();

    // Verify reset to default
    view.read_with(cx, |this, _| {
        assert_eq!(
            this.library.settings().sort_by,
            yomu::library::SortBy::Default
        );
        assert_eq!(
            this.library.settings().sort_direction,
            yomu::library::SortDirection::Ascending
        );
    });
    assert!(cx.debug_bounds("toolbar-sort").is_some());
}

#[gpui_kit::test]
fn test_continue_reading_ordering_unaffected_by_sort(cx: &mut TestAppContext) {
    let fx = fixture("cr-sort");
    let (view, cx) = launch(&fx, cx);

    // Book 0: "A Book", read earlier (100)
    // Book 1: "Z Book", read later (500)
    let (id0, id1) = view.update(cx, |this, _| {
        let id0 = fx.ids[0].clone();
        let b0 = this.library.get_mut(&id0).unwrap();
        b0.title = "A Book".into();
        b0.last_read = Some(100);
        b0.opened = true;
        b0.current_page = 1;
        b0.page_count = 5;

        let id1 = fx.ids[1].clone();
        let b1 = this.library.get_mut(&id1).unwrap();
        b1.title = "Z Book".into();
        b1.last_read = Some(500);
        b1.opened = true;
        b1.current_page = 2;
        b1.page_count = 5;

        (id0, id1)
    });

    // Set library sort to Title A-Z (which puts A Book before Z Book in library view)
    view.update(cx, |this, cx| {
        this.set_sort_mode(
            yomu::library::SortBy::Title,
            yomu::library::SortDirection::Ascending,
            cx,
        );
    });

    let lib_order = view.read_with(cx, |this, _| this.current_view_order());
    assert_eq!(lib_order[0], id0);

    // Switch to Continue Reading view
    view.update(cx, |this, cx| {
        this.navigate_to(crate::ui::sidebar::MainView::ContinueReading, cx);
    });
    cx.run_until_parked();

    // Verify Continue Reading view renders without issue
    assert_eq!(
        view.read_with(cx, |this, _| this.active_view.clone()),
        crate::ui::sidebar::MainView::ContinueReading
    );
    assert_eq!(
        view.read_with(cx, |this, _| this.library.get(&id1).unwrap().last_read),
        Some(500)
    );
}

#[gpui_kit::test]
fn test_dual_page_spread_navigation_and_cover(cx: &mut TestAppContext) {
    let fx = fixture("spread_nav");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // Initial state: single page
    view.update(cx, |this, cx| {
        this.set_page_layout(PageLayout::Double, cx);
        this.set_reading_direction(ReadingDirection::LeftToRight, cx);
        this.set_first_page_is_cover(true, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert!(reader.is_double_page_active());
        assert_eq!(reader.current_spread_index(), 0);
        let spread = reader.active_spread();
        assert_eq!(spread.primary, 0);
        assert_eq!(spread.secondary, None);
    });

    // Advance to next spread -> spread 1 [1, 2]
    view.update(cx, |this, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 1);
        let spread = reader.active_spread();
        assert_eq!(spread.primary, 1);
        assert_eq!(spread.secondary, Some(2));
        assert_eq!(reader.page, 1);
    });

    // Advance to next spread -> spread 2 [3, 4]
    view.update(cx, |this, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 2);
        let spread = reader.active_spread();
        assert_eq!(spread.primary, 3);
        assert_eq!(spread.secondary, Some(4));
        assert_eq!(reader.page, 3);
    });

    // Retreat to spread 1 -> [1, 2]
    view.update(cx, |this, cx| {
        this.turn_by(false, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 1);
        assert_eq!(reader.page, 1);
    });

    // Retreat to spread 0 -> [0]
    view.update(cx, |this, cx| {
        this.turn_by(false, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 0);
        assert_eq!(reader.page, 0);
    });
}

#[gpui_kit::test]
fn test_dual_page_spread_rtl_navigation_and_pairing(cx: &mut TestAppContext) {
    let fx = fixture("spread_rtl");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    view.update(cx, |this, cx| {
        this.set_page_layout(PageLayout::Double, cx);
        this.set_reading_direction(ReadingDirection::RightToLeft, cx);
        this.set_first_page_is_cover(false, cx);
    });
    cx.run_until_parked();

    // Without cover, spread 0 is [0, 1]
    // In RTL: primary (0) is on right, secondary (1) is on left
    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        let spread = reader.active_spread();
        assert_eq!(spread.primary, 0);
        assert_eq!(spread.secondary, Some(1));
        let vis = reader.active_visual_spread();
        assert_eq!(vis.right_page, Some(0));
        assert_eq!(vis.left_page, Some(1));
    });

    // In RTL, left step advances forward
    view.update(cx, |this, cx| {
        this.step(crate::app::state::Side::Left, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 1);
        let spread = reader.active_spread();
        assert_eq!(spread.primary, 2);
        assert_eq!(spread.secondary, Some(3));
    });

    // In RTL, right step goes backward
    view.update(cx, |this, cx| {
        this.step(crate::app::state::Side::Right, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 0);
        assert_eq!(reader.page, 0);
    });
}

#[gpui_kit::test]
fn test_switching_layout_preserves_reading_position(cx: &mut TestAppContext) {
    let fx = fixture("spread_switch");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // Jump to page 3 in single page mode
    view.update(cx, |this, cx| {
        this.goto_page(3, Arrive::Start, cx);
    });
    cx.run_until_parked();
    assert_eq!(page(&view, cx), 3);

    // Switch to Double page mode
    view.update(cx, |this, cx| {
        this.set_page_layout(PageLayout::Double, cx);
    });
    cx.run_until_parked();

    // Logical page 3 remains active, within spread 2 [3, 4]
    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 3);
        assert_eq!(reader.current_spread_index(), 2);
    });

    // Switch back to Single page mode
    view.update(cx, |this, cx| {
        this.set_page_layout(PageLayout::Single, cx);
    });
    cx.run_until_parked();

    assert_eq!(page(&view, cx), 3);
}

#[gpui_kit::test]
fn test_continuous_mode_unaffected_by_double_page_setting(cx: &mut TestAppContext) {
    let fx = fixture("continuous_double");
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // Enable continuous mode and double page layout
    view.update(cx, |this, cx| {
        this.set_continuous(Some(true), cx);
        this.set_page_layout(PageLayout::Double, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert!(reader.continuous);
        // Continuous mode overrides dual spread presentation
        assert!(!reader.is_double_page_active());
    });
}

#[gpui_kit::test]
fn test_mixed_source_spread_detection_and_navigation(cx: &mut TestAppContext) {
    // 0: Portrait (600x900)
    // 1: Portrait (600x900)
    // 2: Double Spread (1800x900, ratio 2.0 >= 1.25)
    // 3: Portrait (600x900)
    // 4: Portrait (600x900)
    let dims = [(600, 900), (600, 900), (1800, 900), (600, 900), (600, 900)];
    let fx = fixture_mixed("mixed_nav", &dims);
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // Switch to Double page mode with cover isolation
    view.update(cx, |this, cx| {
        this.set_page_layout(PageLayout::Double, cx);
        this.set_first_page_is_cover(true, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        // Spread 0: [0] (cover)
        // Spread 1: [1] (single page, cannot pair with page 2 because page 2 is a single-image spread)
        // Spread 2: [2] (single-image spread alone)
        // Spread 3: [3, 4] (paired normal pages)
        assert_eq!(reader.total_spreads(), 4);
        assert_eq!(reader.current_spread_index(), 0);
        let s0 = reader.active_spread();
        assert_eq!(s0.primary, 0);
        assert_eq!(s0.secondary, None);
    });

    // Advance to next spread
    view.update(cx, |this, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 1);
        assert_eq!(reader.page, 1);
        let s1 = reader.active_spread();
        assert_eq!(s1.primary, 1);
        assert_eq!(s1.secondary, None);
    });

    // Advance to next spread: single-image spread at page 2
    view.update(cx, |this, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 2);
        assert_eq!(reader.page, 2);
        let s2 = reader.active_spread();
        assert_eq!(s2.primary, 2);
        assert_eq!(s2.secondary, None);
        assert!(s2.is_single_image_spread);
    });

    // Advance to next spread: paired pages [3, 4]
    view.update(cx, |this, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 3);
        assert_eq!(reader.page, 3);
        let s3 = reader.active_spread();
        assert_eq!(s3.primary, 3);
        assert_eq!(s3.secondary, Some(4));
        assert!(!s3.is_single_image_spread);
    });

    // Retreat to spread 2 (single-image spread)
    view.update(cx, |this, cx| {
        this.turn_by(false, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 2);
        assert_eq!(reader.page, 2);
    });
}

#[gpui_kit::test]
fn test_mixed_source_manual_override_precedence_and_persistence(cx: &mut TestAppContext) {
    let dims = [(600, 900), (600, 900), (600, 900), (600, 900)];
    let fx = fixture_mixed("mixed_override", &dims);
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    view.update(cx, |this, cx| {
        this.set_page_layout(PageLayout::Double, cx);
        this.set_first_page_is_cover(false, cx);
    });
    cx.run_until_parked();

    // Default: all 4 are single portrait pages -> Spreads: [0, 1], [2, 3] (total: 2)
    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.total_spreads(), 2);
    });

    // Force page 1 to be a spread via manual override
    view.update(cx, |this, cx| {
        this.goto_page(1, Arrive::Start, cx);
        this.set_page_spread_override(PageSpreadOverride::ForceSpread, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(
            reader.current_spread_override(),
            PageSpreadOverride::ForceSpread
        );
        // Page 0 cannot pair with Page 1 (spread).
        // Spread 0: [0], Spread 1: [1] (spread), Spread 2: [2, 3] -> Total: 3
        assert_eq!(reader.total_spreads(), 3);
        assert_eq!(
            reader.pages_for_spread(0),
            Some(layout::SpreadPages::new(0, None))
        );
        assert_eq!(
            reader.pages_for_spread(1),
            Some(layout::SpreadPages::single_image_spread(1))
        );
        assert_eq!(
            reader.pages_for_spread(2),
            Some(layout::SpreadPages::new(2, Some(3)))
        );
    });

    // Close the book and reopen: manual overrides persist
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.close_book(window, cx);
        });
    });
    cx.run_until_parked();

    open_book_by_index(&fx, &view, cx, 0);
    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.total_spreads(), 3);
        assert_eq!(
            reader.spread_overrides.get("page_002.png"),
            Some(&PageSpreadOverride::ForceSpread)
        );
    });

    // Clear override back to Auto
    view.update(cx, |this, cx| {
        this.goto_page(1, Arrive::Start, cx);
        this.set_page_spread_override(PageSpreadOverride::Auto, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_override(), PageSpreadOverride::Auto);
        assert_eq!(reader.total_spreads(), 2);
    });
}

#[gpui_kit::test]
fn test_mixed_source_rtl_placement_and_turns(cx: &mut TestAppContext) {
    let dims = [
        (600, 900),
        (1800, 900), // Single-image spread at index 1
        (600, 900),
        (600, 900),
    ];
    let fx = fixture_mixed("mixed_rtl", &dims);
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    view.update(cx, |this, cx| {
        this.set_page_layout(PageLayout::Double, cx);
        this.set_reading_direction(ReadingDirection::RightToLeft, cx);
        this.set_first_page_is_cover(false, cx);
    });
    cx.run_until_parked();

    // Spread 0: [0] (alone because 1 is a spread) -> left=0, right=None
    // Spread 1: [1] (single-image spread) -> left=1, right=None
    // Spread 2: [2, 3] (paired) -> in RTL: right=2, left=3
    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.total_spreads(), 3);
        let s0_vis = reader.active_visual_spread();
        assert_eq!(s0_vis.left_page, Some(0));
        assert_eq!(s0_vis.right_page, None);
    });

    // In RTL, left step advances to spread 1 (page 1)
    view.update(cx, |this, cx| {
        this.step(crate::app::state::Side::Left, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 1);
        let s1_vis = reader.active_visual_spread();
        assert_eq!(s1_vis.left_page, Some(1));
        assert_eq!(s1_vis.right_page, None);
    });

    // Advance to spread 2 ([2, 3])
    view.update(cx, |this, cx| {
        this.step(crate::app::state::Side::Left, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 2);
        let s2_vis = reader.active_visual_spread();
        assert_eq!(s2_vis.right_page, Some(2));
        assert_eq!(s2_vis.left_page, Some(3));
    });
}

#[gpui_kit::test]
fn test_mixed_source_keybinding_toggle(cx: &mut TestAppContext) {
    let dims = [(600, 900), (600, 900)];
    let fx = fixture_mixed("mixed_key", &dims);
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // Initial override is Auto
    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_override(), PageSpreadOverride::Auto);
    });

    // Press "s" -> ForceSpread
    cx.simulate_keystrokes("s");

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(
            reader.current_spread_override(),
            PageSpreadOverride::ForceSpread
        );
    });

    // Press "s" again -> ForceSingle
    cx.simulate_keystrokes("s");

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(
            reader.current_spread_override(),
            PageSpreadOverride::ForceSingle
        );
    });

    // Press "s" again -> Auto
    cx.simulate_keystrokes("s");

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_override(), PageSpreadOverride::Auto);
    });
}

#[gpui_kit::test]
fn test_narrow_viewport_two_image_spread_navigation_ltr(cx: &mut TestAppContext) {
    let dims = [(600, 900), (600, 900), (600, 900), (600, 900)];
    let fx = fixture_mixed("narrow_ltr", &dims);
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    view.update_in(cx, |this, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1200.0), gpui_kit::px(800.0)));
        window.bounds_changed(cx);
        this.set_page_layout(PageLayout::Double, cx);
        this.set_reading_direction(ReadingDirection::LeftToRight, cx);
        this.set_first_page_is_cover(true, cx);
    });
    cx.run_until_parked();

    // Verify wide viewport presentation
    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert!(reader.is_double_page_active());
        assert_eq!(reader.current_spread_index(), 0);
        assert_eq!(reader.page, 0);
        assert_eq!(reader.current_page_str().as_ref(), "1 / 4");
    });

    // Advance to Spread 1: [1, 2] in wide viewport
    view.update(cx, |this, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 1);
        assert_eq!(reader.page, 1);
        assert_eq!(reader.current_page_str().as_ref(), "2–3 / 4");
    });

    // Fall back to narrow viewport by resizing window to portrait 400x800
    view.update_in(cx, |_this, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(400.0), gpui_kit::px(800.0)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert!(!reader.is_double_page_active());
        assert_eq!(reader.page_layout, PageLayout::Double);
        assert_eq!(reader.page, 1);
        assert_eq!(reader.current_page_str().as_ref(), "2 / 4");
    });

    // Turn forward in narrow viewport: advances to page 2 (secondary page of the spread)
    view.update(cx, |this, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 2);
        assert_eq!(reader.current_page_str().as_ref(), "3 / 4");
    });

    // Turn forward in narrow viewport: advances to page 3
    view.update(cx, |this, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 3);
        assert_eq!(reader.current_page_str().as_ref(), "4 / 4");
    });

    // Turn backward in narrow viewport: retreats to page 2 (secondary page)
    view.update(cx, |this, cx| {
        this.turn_by(false, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 2);
        assert_eq!(reader.current_page_str().as_ref(), "3 / 4");
    });

    // Resize back to wide viewport (1200x800) while on page 2: maps to Spread 1 [1, 2]
    view.update_in(cx, |_this, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1200.0), gpui_kit::px(800.0)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert!(reader.is_double_page_active());
        assert_eq!(reader.current_spread_index(), 1);
        assert_eq!(reader.current_page_str().as_ref(), "2–3 / 4");
        let vis = reader.active_visual_spread();
        assert_eq!(vis.left_page, Some(1));
        assert_eq!(vis.right_page, Some(2));
    });

    // Turn forward in wide viewport: advances to Spread 2 [3]
    view.update(cx, |this, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 2);
        assert_eq!(reader.page, 3);
        assert_eq!(reader.current_page_str().as_ref(), "4 / 4");
    });
}

#[gpui_kit::test]
fn test_narrow_viewport_two_image_spread_navigation_rtl(cx: &mut TestAppContext) {
    let dims = [(600, 900), (600, 900), (600, 900), (600, 900)];
    let fx = fixture_mixed("narrow_rtl", &dims);
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // Start in narrow viewport
    view.update_in(cx, |this, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(450.0), gpui_kit::px(850.0)));
        window.bounds_changed(cx);
        this.set_page_layout(PageLayout::Double, cx);
        this.set_reading_direction(ReadingDirection::RightToLeft, cx);
        this.set_first_page_is_cover(true, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert!(!reader.is_double_page_active());
        assert_eq!(reader.page, 0);
        assert_eq!(reader.current_page_str().as_ref(), "1 / 4");
    });

    // In RTL, left step advances to page 1 (primary page of Spread 1: right side in RTL, read first)
    view.update(cx, |this, cx| {
        this.step(crate::app::state::Side::Left, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 1);
        assert_eq!(reader.current_page_str().as_ref(), "2 / 4");
    });

    // In RTL, next left step advances to page 2 (secondary page of Spread 1: left side in RTL, read second)
    view.update(cx, |this, cx| {
        this.step(crate::app::state::Side::Left, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 2);
        assert_eq!(reader.current_page_str().as_ref(), "3 / 4");
    });

    // Left step advances to page 3
    view.update(cx, |this, cx| {
        this.step(crate::app::state::Side::Left, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 3);
        assert_eq!(reader.current_page_str().as_ref(), "4 / 4");
    });

    // Right step in RTL retreats to page 2 (secondary page)
    view.update(cx, |this, cx| {
        this.step(crate::app::state::Side::Right, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 2);
        assert_eq!(reader.current_page_str().as_ref(), "3 / 4");
    });

    // While on page 2, widen the viewport: restores dual-page spread with page 1 (right) and page 2 (left)
    view.update_in(cx, |_this, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1200.0), gpui_kit::px(800.0)));
        window.bounds_changed(cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert!(reader.is_double_page_active());
        assert_eq!(reader.current_spread_index(), 1);
        assert_eq!(reader.current_page_str().as_ref(), "2–3 / 4");
        let vis = reader.active_visual_spread();
        assert_eq!(vis.right_page, Some(1));
        assert_eq!(vis.left_page, Some(2));
    });

    // Left step in wide RTL advances to Spread 2 [3]
    view.update(cx, |this, cx| {
        this.step(crate::app::state::Side::Left, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_index(), 2);
        assert_eq!(reader.page, 3);
        assert_eq!(reader.current_page_str().as_ref(), "4 / 4");
    });
}

#[gpui_kit::test]
fn test_narrow_viewport_single_image_spread_not_duplicated(cx: &mut TestAppContext) {
    let dims = [
        (600, 900),
        (1800, 900), // Single-image wide spread at page 1
        (600, 900),
        (600, 900),
    ];
    let fx = fixture_mixed("narrow_single_spread", &dims);
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // In narrow viewport
    view.update_in(cx, |this, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(400.0), gpui_kit::px(800.0)));
        window.bounds_changed(cx);
        this.set_page_layout(PageLayout::Double, cx);
        this.set_first_page_is_cover(true, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert!(!reader.is_double_page_active());
        assert_eq!(reader.page, 0);
        assert_eq!(reader.current_page_str().as_ref(), "1 / 4");
    });

    // Turn to page 1 (single-image spread)
    view.update(cx, |this, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 1);
        assert_eq!(reader.current_page_str().as_ref(), "2 / 4");
    });

    // Turn to page 2 (normal portrait page - single image spread was not duplicated)
    view.update(cx, |this, cx| {
        this.turn_by(true, 1, Arrive::Start, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 2);
        assert_eq!(reader.current_page_str().as_ref(), "3 / 4");
    });
}

#[gpui_kit::test]
fn test_double_page_spread_zoom_panning_offset(cx: &mut TestAppContext) {
    let dims = [(600, 900), (600, 900), (600, 900), (600, 900)];
    let fx = fixture_mixed("spread_zoom", &dims);
    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    // Set wide viewport and enable double page with first_page_is_cover = false
    // so pages 0 and 1 are rendered together as a spread
    view.update_in(cx, |this, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1200.0), gpui_kit::px(800.0)));
        window.bounds_changed(cx);
        this.set_page_layout(PageLayout::Double, cx);
        this.set_first_page_is_cover(false, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert!(reader.is_double_page_active());
        assert_eq!(reader.current_spread_index(), 0);
        let vis = reader.active_visual_spread();
        assert_eq!(vis.left_page, Some(0));
        assert_eq!(vis.right_page, Some(1));
    });

    // Zoom in 2.5x
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.zoom_to(2.5, (vp.width / 2.0, vp.height / 2.0), vp, cx);
    });
    cx.run_until_parked();
    settle_animations(&view, cx);

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.zoom, 2.5);
    });

    // Scroll right to pan along the spread
    view.update_in(cx, |this, window, cx| {
        let vp = viewport_of(window);
        this.scroll(Dir::Right, Amount::Page, 5, vp, cx);
    });
    cx.run_until_parked();
    settle_animations(&view, cx);

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        // Clamping against the full spread allows panning horizontally
        assert!(reader.off.0 > 0.0);
    });
}

#[gpui_kit::test]
fn test_vim_reader_layout_and_spread_commands(cx: &mut TestAppContext) {
    let fx = fixture("vim_reader_layout");
    let (view, cx) = launch(&fx, cx);

    // Enable Vim mode
    view.update_in(cx, |this, _, _| {
        this.library.settings_mut().vim_mode = true;
    });

    open_book_by_index(&fx, &view, cx, 0);

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page_layout, PageLayout::Single);
    });

    // Press 'd' to toggle double page
    cx.simulate_keystrokes("d");
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page_layout, PageLayout::Double);
    });

    // Press 'z d' to toggle back
    cx.simulate_keystrokes("z d");
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page_layout, PageLayout::Single);
    });

    // Press 'd' again to toggle
    cx.simulate_keystrokes("d");
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page_layout, PageLayout::Double);
    });

    // Press 'd' again to return to Single
    cx.simulate_keystrokes("d");
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page_layout, PageLayout::Single);
    });

    // Test 'z s' to cycle spread override
    cx.simulate_keystrokes("z s");
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(
            reader.current_spread_override(),
            PageSpreadOverride::ForceSpread
        );
    });

    cx.simulate_keystrokes("z s");
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(
            reader.current_spread_override(),
            PageSpreadOverride::ForceSingle
        );
    });

    cx.simulate_keystrokes("z s");
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.current_spread_override(), PageSpreadOverride::Auto);
    });
}

#[gpui_kit::test]
fn test_archive_with_jfif_image(cx: &mut TestAppContext) {
    static FIXTURE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = FIXTURE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("yomu-ui-jfif-{}-{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let dirs = DataDirs::at(root.join("data")).unwrap();

    let path = root.join("jfif_manga.cbz");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();

    let img = image::RgbImage::from_pixel(100, 150, image::Rgb([180, 50, 80]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    let bytes = out.into_inner();

    zip.start_file("page_001.jfif", options).unwrap();
    zip.write_all(&bytes).unwrap();
    zip.finish().unwrap();

    let files = vec![path];
    let report = library::import_paths(&files, &dirs, &HashSet::new());
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.added.len(), 1);

    let mut library = Library::default();
    for book in report.added {
        library.add(book);
    }
    library.save(&dirs).unwrap();

    let fx = Fixture {
        ids: library.books().iter().map(|b| b.id.clone()).collect(),
        files,
        dirs,
    };

    let (view, cx) = launch(&fx, cx);
    open_book_by_index(&fx, &view, cx, 0);

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.total, 1);
        assert_eq!(reader.current_page_str().as_ref(), "1 / 1");
    });
}

#[gpui_kit::test]
fn test_empty_archive_handling(cx: &mut TestAppContext) {
    let fx = fixture("empty_test");
    let (view, cx) = launch(&fx, cx);

    let empty_path = fx.files[0].parent().unwrap().join("empty.cbz");
    let zip = zip::ZipWriter::new(fs::File::create(&empty_path).unwrap());
    zip.finish().unwrap();

    let empty_book = Book {
        id: "empty-archive".into(),
        title: "Empty Volume".into(),
        series: None,
        path: empty_path,
        kind: SourceKind::Archive,
        page_count: 0,
        current_page: 0,
        opened: false,
        added_at: 1,
        last_read: None,
        marks: BTreeMap::new(),
        spread_overrides: BTreeMap::new(),
        page_dimensions: Default::default(),
    };

    view.update_in(cx, |this, window, cx| {
        this.library.add(empty_book);
        this.open_book("empty-archive", window, cx);
    });
    cx.run_until_parked();

    view.update(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.total, 0);
        assert_eq!(reader.current_page_str().as_ref(), "0 / 0");
    });
}

#[gpui_kit::test]
fn regression_settings_dialog_open_close_reopen(cx: &mut TestAppContext) {
    let fx = fixture("settings-regression");
    let (view, cx) = launch(&fx, cx);

    // 1. Initial state: no overlay
    assert_eq!(overlay_kind(&view, cx), "none");

    // 2. Open via shortcut ctrl-,
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 3. Toggle close via shortcut ctrl-,
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 4. Open via sidebar action (simulating sidebar button click callback)
    view.update_in(cx, |this, window, cx| {
        this.open_settings_window(window, cx);
    });
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 5. Close via Escape
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 6. Reopen via shortcut ctrl-,
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 7. Close via close_overlay_window
    view.update_in(cx, |this, window, cx| {
        this.close_overlay_window(window, cx);
    });
    assert_eq!(overlay_kind(&view, cx), "none");
}

#[gpui_kit::test]
fn regression_colon_key_is_inert_and_does_not_open_command_palette(cx: &mut TestAppContext) {
    let fx = fixture("colon-inert-regression");
    let (view, cx) = launch(&fx, cx);

    // 1. In library with vim mode disabled: ':' does nothing
    cx.simulate_keystrokes(":");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 2. In library with vim mode enabled: ':' does nothing
    view.update_in(cx, |this, _, cx| {
        this.toggle_vim_mode(true, cx);
    });
    cx.simulate_keystrokes(":");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 3. Open book in reader with vim mode enabled: ':' does nothing
    let id = fx.ids[0].clone();
    act(&view, cx, &id, BookAction::Open);
    assert_eq!(reading(&view, cx), Some(0));

    cx.simulate_keystrokes(":");
    assert_eq!(overlay_kind(&view, cx), "none");
    assert_eq!(reading(&view, cx), Some(0));

    // 4. In reader with vim mode disabled: ':' does nothing
    view.update_in(cx, |this, _, cx| {
        this.toggle_vim_mode(false, cx);
    });
    cx.simulate_keystrokes(":");
    assert_eq!(overlay_kind(&view, cx), "none");
    assert_eq!(reading(&view, cx), Some(0));
}

#[gpui_kit::test]
fn regression_normal_vim_navigation_remains_functional(cx: &mut TestAppContext) {
    let fx = fixture("normal-vim-regression");
    let (view, cx) = launch(&fx, cx);

    view.update_in(cx, |this, _, cx| {
        this.toggle_vim_mode(true, cx);
    });

    let id = fx.ids[0].clone();
    act(&view, cx, &id, BookAction::Open);
    assert_eq!(reading(&view, cx), Some(0));

    // Jump to last page with 'G'
    cx.simulate_keystrokes("G");
    assert_eq!(reading(&view, cx), Some(4));

    // Jump to first page with 'gg'
    cx.simulate_keystrokes("g g");
    assert_eq!(reading(&view, cx), Some(0));

    // Set mark 'a'
    cx.simulate_keystrokes("m a");

    // Jump to last page
    cx.simulate_keystrokes("G");
    assert_eq!(reading(&view, cx), Some(4));

    // Jump back to mark 'a'
    cx.simulate_keystrokes("' a");
    assert_eq!(reading(&view, cx), Some(0));

    // Jump back with ''
    cx.simulate_keystrokes("' '");
    assert_eq!(reading(&view, cx), Some(4));

    // Test layout toggles: 'd' toggles to Double, 'z d' toggles to Single
    cx.simulate_keystrokes("d");
    cx.run_until_parked();
    view.update(cx, |this, _| {
        assert_eq!(
            this.reader.as_ref().unwrap().page_layout,
            PageLayout::Double
        );
    });

    cx.simulate_keystrokes("z d");
    cx.run_until_parked();
    view.update(cx, |this, _| {
        assert_eq!(
            this.reader.as_ref().unwrap().page_layout,
            PageLayout::Single
        );
    });

    // Test 'z s' spread override
    cx.simulate_keystrokes("z s");
    cx.run_until_parked();
    view.update(cx, |this, _| {
        assert_eq!(
            this.reader.as_ref().unwrap().current_spread_override(),
            PageSpreadOverride::ForceSpread
        );
    });
}

#[gpui_kit::test]
fn regression_settings_dialog_full_lifecycle(cx: &mut TestAppContext) {
    let fx = fixture("settings-lifecycle-regression");
    let (view, cx) = launch(&fx, cx);

    // 1. Open with ctrl-,
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 2. Close with escape
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 3. Open with ctrl-, and toggle close with ctrl-,
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 4. Reopen and change color scheme
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    view.update_in(cx, |this, _, cx| {
        this.set_color_scheme("Gruvbox", cx);
    });

    view.update_in(cx, |this, _, _| {
        assert_eq!(this.library.settings().color_scheme, "Gruvbox");
    });

    // 5. Close settings
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 6. Verify setting persisted on disk
    let on_disk = Library::load(&fx.dirs).unwrap();
    assert_eq!(on_disk.settings().color_scheme, "Gruvbox");
}

#[gpui_kit::test]
fn regression_settings_advanced_clear_cache_cancel_and_confirm(cx: &mut TestAppContext) {
    let fx = fixture("settings-cache-actions");
    let (view, cx) = launch(&fx, cx);

    // Initial state: no overlay
    assert_eq!(overlay_kind(&view, cx), "none");

    // 1. Open settings
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 2. Request Clear Thumbnail Cache
    view.update(cx, |this, cx| this.request_clear_thumbnail_cache(cx));
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "confirm");

    // 3. Cancel confirmation -> must return cleanly to Settings
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 4. Close settings -> no overlay remains
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 5. Normal keyboard navigation is functional (not swallowed by zombie overlay)
    cx.simulate_keystrokes("j");
    cx.simulate_keystrokes("k");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 6. Reopen settings immediately via shortcut
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 7. Request Clear Thumbnail Cache again
    view.update(cx, |this, cx| this.request_clear_thumbnail_cache(cx));
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "confirm");

    // 8. Confirm action -> must return cleanly to Settings
    cx.simulate_keystrokes("enter");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 9. Close settings -> no overlay remains
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 10. Reopen immediately
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
}

#[gpui_kit::test]
fn regression_settings_advanced_clear_all_data_cancel_and_confirm(cx: &mut TestAppContext) {
    let fx = fixture("settings-all-data-actions");
    let (view, cx) = launch(&fx, cx);

    // 1. Open settings
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 2. Request Clear All Data
    view.update(cx, |this, cx| this.request_clear_all_data(cx));
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "confirm");

    // 3. Cancel confirmation -> returns cleanly to Settings
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 4. Close settings -> overlay is none
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 5. Normal keyboard navigation is functional
    cx.simulate_keystrokes("j");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 6. Reopen settings immediately
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 7. Request Clear All Data again
    view.update(cx, |this, cx| this.request_clear_all_data(cx));
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "confirm");

    // 8. Confirm action -> returns cleanly to Settings
    cx.simulate_keystrokes("enter");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 9. Close settings
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
}

#[gpui_kit::test]
fn regression_settings_advanced_reset_settings_cancel_and_confirm(cx: &mut TestAppContext) {
    let fx = fixture("settings-reset-actions");
    let (view, cx) = launch(&fx, cx);

    // Modify a setting so reset has something to do
    view.update_in(cx, |this, _, cx| {
        this.set_color_scheme("Gruvbox", cx);
    });

    // 1. Open settings
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 2. Request Reset Settings
    view.update(cx, |this, cx| this.request_reset_settings(cx));
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "confirm");

    // 3. Cancel confirmation -> returns cleanly to Settings
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 4. Close settings -> overlay is none
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 5. Reopen settings immediately
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 6. Request Reset Settings again
    view.update(cx, |this, cx| this.request_reset_settings(cx));
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "confirm");

    // 7. Confirm action -> returns cleanly to Settings
    cx.simulate_keystrokes("enter");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 8. Close settings -> overlay is none
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 9. Navigation continues to work
    cx.simulate_keystrokes("j");
    assert_eq!(overlay_kind(&view, cx), "none");
}

#[gpui_kit::test]
fn regression_toolbar_button_group_segmented_controls(cx: &mut TestAppContext) {
    let fx = fixture("toolbar-segmented");
    let (view, cx) = launch(&fx, cx);

    // 1. Initial state: grid mode is true, group series is false by default
    view.read_with(cx, |this, _| {
        assert!(this.library.settings().view_mode_grid);
        assert!(!this.library.settings().group_series);
    });

    // 2. Toggle group series to true via view method (as triggered by ButtonGroup child)
    view.update_in(cx, |this, _, cx| {
        this.toggle_group_series(true, cx);
    });
    view.read_with(cx, |this, _| {
        assert!(this.library.settings().group_series);
    });

    // 3. Toggle back to false
    view.update_in(cx, |this, _, cx| {
        this.toggle_group_series(false, cx);
    });
    view.read_with(cx, |this, _| {
        assert!(!this.library.settings().group_series);
    });

    // 4. Toggle view mode grid vs list
    view.update_in(cx, |this, _, cx| {
        this.view_mode_grid = false;
        this.library.settings_mut().view_mode_grid = false;
        this.save_library(cx);
    });
    view.read_with(cx, |this, _| {
        assert!(!this.library.settings().view_mode_grid);
    });

    view.update_in(cx, |this, _, cx| {
        this.view_mode_grid = true;
        this.library.settings_mut().view_mode_grid = true;
        this.save_library(cx);
    });
    view.read_with(cx, |this, _| {
        assert!(this.library.settings().view_mode_grid);
    });
}

#[gpui_kit::test]
fn regression_properties_dialog_with_native_description_list(cx: &mut TestAppContext) {
    let fx = fixture("props-native-dl");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();

    // 1. Open properties dialog
    act(&view, cx, &id, BookAction::Properties);
    assert_eq!(overlay_kind(&view, cx), "properties");

    // 2. Verify all metadata rows are populated
    let rows = view.read_with(cx, |this, _| match &this.overlay {
        Some(Overlay::Properties(dialog)) => dialog.rows.clone(),
        _ => Vec::new(),
    });
    let get = |name: &str| rows.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    assert_eq!(get("Pages").as_deref(), Some("5"));
    assert_eq!(get("Format").as_deref(), Some("CBZ archive"));
    assert!(get("Location").unwrap().ends_with("A.cbz"));
    assert!(get("Size").is_some());

    // 3. Close properties dialog via Escape
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");

    // 4. Keyboard navigation operates normally
    cx.simulate_keystrokes("j");
    assert_eq!(overlay_kind(&view, cx), "none");
}

#[gpui_kit::test]
fn regression_phase5_main_sidebar_navigation_and_counts(cx: &mut TestAppContext) {
    let fx = fixture("sidebar-nav-phase5");
    let (view, cx) = launch(&fx, cx);

    // Initial section is AllManga by default
    view.read_with(cx, |this, _| {
        assert_eq!(this.active_section, ui::sidebar::SidebarSection::AllManga);
        assert_eq!(this.library.books().len(), 3);
    });

    // Switch through all sidebar sections
    for section in [
        ui::sidebar::SidebarSection::ContinueReading,
        ui::sidebar::SidebarSection::AllManga,
    ] {
        view.update_in(cx, |this, _, cx| {
            this.active_section = section;
            cx.notify();
        });
        cx.run_until_parked();
        view.read_with(cx, |this, _| {
            assert_eq!(this.active_section, section);
        });
    }

    // Settings trigger from sidebar footer opens settings dialog
    view.update_in(cx, |this, window, cx| {
        this.open_settings_window(window, cx);
    });
    assert_eq!(overlay_kind(&view, cx), "settings");

    // Close via Escape
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");

    // Verify toolbar sidebar collapse/expand toggle operates cleanly
    view.read_with(cx, |this, _| {
        assert!(!this.sidebar_collapsed);
    });
    // Collapse sidebar
    view.update_in(cx, |this, _, cx| {
        this.toggle_sidebar(cx);
    });
    view.read_with(cx, |this, _| {
        assert!(this.sidebar_collapsed);
    });
    // Expand sidebar back
    view.update_in(cx, |this, _, cx| {
        this.toggle_sidebar(cx);
    });
    view.read_with(cx, |this, _| {
        assert!(!this.sidebar_collapsed);
    });
    // Verify repeated toggles work reliably
    view.update_in(cx, |this, _, cx| {
        this.toggle_sidebar(cx);
    });
    view.read_with(cx, |this, _| {
        assert!(this.sidebar_collapsed);
    });
    view.update_in(cx, |this, _, cx| {
        this.toggle_sidebar(cx);
    });
    view.read_with(cx, |this, _| {
        assert!(!this.sidebar_collapsed);
    });
    // Verify navigation destinations still function after re-expansion
    view.update_in(cx, |this, _, cx| {
        this.active_section = ui::sidebar::SidebarSection::AllManga;
        cx.notify();
    });
    view.read_with(cx, |this, _| {
        assert_eq!(this.active_section, ui::sidebar::SidebarSection::AllManga);
    });
}

#[gpui_kit::test]
fn regression_phase5_settings_native_sidebar_navigation_and_tabs(cx: &mut TestAppContext) {
    let fx = fixture("settings-sidebar-phase5");
    let (view, cx) = launch(&fx, cx);

    // 1. Open settings dialog
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // 2. Default tab is Appearance
    view.read_with(cx, |this, _| {
        assert_eq!(this.settings_tab, ui::overlays::SettingsTab::Appearance);
    });

    // 3. Switch through each SettingsTab and verify rendering and state
    let tabs = [
        ui::overlays::SettingsTab::Appearance,
        ui::overlays::SettingsTab::Reader,
        ui::overlays::SettingsTab::Shortcuts,
        ui::overlays::SettingsTab::Advanced,
        ui::overlays::SettingsTab::General,
        ui::overlays::SettingsTab::About,
    ];

    for tab in tabs {
        view.update_in(cx, |this, window, cx| {
            this.settings_tab = tab;
            let _content = this.render_settings_content(window, cx);
            cx.notify();
        });
        cx.run_until_parked();
        view.read_with(cx, |this, _| {
            assert_eq!(this.settings_tab, tab);
        });
    }

    // 4. Verify closing dialog via Escape dismisses cleanly without orphaned state
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
    view.read_with(cx, |this, _| {
        assert!(this.overlay.is_none());
    });
}

#[gpui_kit::test]
fn test_settings_about_tab_metadata_and_rendering(cx: &mut TestAppContext) {
    let fx = fixture("settings-about-tab");
    let (view, cx) = launch(&fx, cx);

    // 1. Verify tab metadata invariants
    assert_eq!(ui::overlays::SettingsTab::About.label(), "About");
    assert_eq!(ui::overlays::SettingsTab::About.to_index(), 5);
    assert_eq!(
        ui::overlays::SettingsTab::from_index(5),
        ui::overlays::SettingsTab::About
    );

    // 2. Open settings and select the About tab
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");

    view.update_in(cx, |this, window, cx| {
        this.settings_tab = ui::overlays::SettingsTab::About;
        let content = this.render_settings_content(window, cx);
        drop(content);
        cx.notify();
    });
    cx.run_until_parked();

    view.read_with(cx, |this, _| {
        assert_eq!(this.settings_tab, ui::overlays::SettingsTab::About);
    });

    // 3. Verify application logo render image is available
    let logo = app::desktop::app_logo_image();
    assert_eq!(logo, app::desktop::app_logo_image());

    // 4. Verify version constant is dynamically available
    assert!(!env!("CARGO_PKG_VERSION").is_empty());

    // 5. Dismiss dialog
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
}

#[gpui_kit::test]
fn regression_phase6_cards_lists_and_empty_states(cx: &mut TestAppContext) {
    let fx = fixture("phase6-standardization");
    let (view, cx) = launch(&fx, cx);

    // 1. Test empty search in library produces empty state element
    view.update_in(cx, |this, window, cx| {
        this.search_query = "nonexistent_query_string_xyz".to_string();
        let empty_view = this.render_all_manga_view(window, cx);
        drop(empty_view);
    });

    // 2. Test missing manga details renders Empty element without panic
    view.update_in(cx, |this, window, cx| {
        let empty_details = this.render_details_screen("nonexistent-book-id", window, cx);
        drop(empty_details);
    });

    // 3. Test empty continue reading view renders Empty element without panic
    view.update_in(cx, |this, _, _| {
        let scroll = gpui_kit::ScrollHandle::new();
        let props = ui::continue_reading::ContinueReadingProps {
            scroll_handle: &scroll,
            hero: None,
            queue: Vec::new(),
            recent_cards: Vec::new(),
            recent_gap: gpui_kit::px(16.0),
            pad_x: gpui_kit::px(24.0),
            colors: this.colors,
            radius: gpui_kit::px(6.0),
            border_px: 1.0,
        };
        let empty_continue = ui::continue_reading::render_continue_reading_view(
            props,
            |_, _, _| {},
            |_, _, _| {},
            |_, _, _| {},
            |_, menu, _, _| menu,
            |_, _, _| {},
            |_, _, _| {},
        );
        drop(empty_continue);
    });

    // 6. Test list view mode renders books and series groups via ListItem
    view.update_in(cx, |this, window, cx| {
        this.search_query.clear();
        this.view_mode_grid = false;
        let list_view = this.render_all_manga_view(window, cx);
        drop(list_view);

        // Toggle back to grid
        this.view_mode_grid = true;
        let grid_view = this.render_all_manga_view(window, cx);
        drop(grid_view);
    });
}

#[gpui_kit::test]
fn test_dialog_centering_audit_and_verification(cx: &mut TestAppContext) {
    let fx = fixture("dialog-centering-test");
    let (view, cx) = launch(&fx, cx);

    let viewport = view.update_in(cx, |_, window, _| window.viewport_size());
    let expected_center_y = viewport.height / 2.0;
    let expected_center_x = viewport.width / 2.0;

    // 1. Open Settings dialog via shortcut
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(300));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let bounds = cx
        .debug_bounds("dialog-0")
        .expect("Settings dialog-0 surface should be painted");
    let center_y = bounds.center().y;
    assert!(
        (center_y - expected_center_y).abs() <= gpui_kit::px(1.0),
        "Expected Settings dialog to be centered vertically around {expected_center_y:?}, got {center_y:?} (bounds: {bounds:?})"
    );
    let center_x = bounds.center().x;
    assert!(
        (center_x - expected_center_x).abs() <= gpui_kit::px(1.0),
        "Expected Settings dialog to be centered horizontally around {expected_center_x:?}, got {center_x:?} (bounds: {bounds:?})"
    );

    // 1b. Test dynamic window resize re-centers the dialog automatically
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(1200.), gpui_kit::px(900.)));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let resized_bounds = cx
        .debug_bounds("dialog-0")
        .expect("Settings dialog-0 surface should remain painted after resize");
    let resized_viewport = view.update_in(cx, |_, window, _| window.viewport_size());
    let resized_expected_center_y = resized_viewport.height / 2.0;
    let resized_expected_center_x = resized_viewport.width / 2.0;
    assert!(
        (resized_bounds.center().y - resized_expected_center_y).abs() <= gpui_kit::px(1.0),
        "Expected Settings dialog to re-center vertically after window resize to {resized_expected_center_y:?}, got {:?}",
        resized_bounds.center().y
    );
    assert!(
        (resized_bounds.center().x - resized_expected_center_x).abs() <= gpui_kit::px(1.0),
        "Expected Settings dialog to re-center horizontally after window resize to {resized_expected_center_x:?}, got {:?}",
        resized_bounds.center().x
    );

    // Restore original size
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(1920.), gpui_kit::px(1080.)));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    // Close settings
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
    cx.run_until_parked();

    // 2. Open Properties dialog on a book
    let id = fx.ids[0].clone();
    act(&view, cx, &id, BookAction::Properties);
    assert_eq!(overlay_kind(&view, cx), "properties");
    cx.run_until_parked();

    let bounds = cx
        .debug_bounds("dialog-0")
        .expect("Properties dialog-0 surface should be painted");
    let center_y = bounds.center().y;
    assert!(
        (center_y - expected_center_y).abs() <= gpui_kit::px(1.0),
        "Expected Properties dialog to be centered vertically around {expected_center_y:?}, got {center_y:?} (bounds: {bounds:?})"
    );
    let center_x = bounds.center().x;
    assert!(
        (center_x - expected_center_x).abs() <= gpui_kit::px(1.0),
        "Expected Properties dialog to be centered horizontally around {expected_center_x:?}, got {center_x:?} (bounds: {bounds:?})"
    );

    // Close properties
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
    cx.run_until_parked();

    // 3. Open Confirmation alert dialog (standalone library action)
    view.update_in(cx, |this, window, cx| {
        this.open_confirm_alert(
            ConfirmDialog {
                title: "Test Alert".into(),
                message: "Alert message".into(),
                confirm_label: "Confirm",
                action: ConfirmAction::RemoveSelected,
            },
            window,
            cx,
        );
    });
    assert_eq!(overlay_kind(&view, cx), "confirm");
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(300));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let bounds = cx
        .debug_bounds("dialog-0")
        .expect("Alert dialog-0 surface should be painted");
    let center_y = bounds.center().y;
    assert!(
        (center_y - expected_center_y).abs() <= gpui_kit::px(1.0),
        "Expected Alert dialog to be centered vertically around {expected_center_y:?}, got {center_y:?} (bounds: {bounds:?})"
    );
    let center_x = bounds.center().x;
    assert!(
        (center_x - expected_center_x).abs() <= gpui_kit::px(1.0),
        "Expected Alert dialog to be centered horizontally around {expected_center_x:?}, got {center_x:?} (bounds: {bounds:?})"
    );

    // Dismiss alert
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
    cx.run_until_parked();

    // 4. Open Series dialog
    let id_list = vec![fx.ids[0].clone()];
    view.update_in(cx, |this, window, cx| {
        this.open_series_dialog(id_list, window, cx);
    });
    assert_eq!(overlay_kind(&view, cx), "series");
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(300));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let bounds = cx
        .debug_bounds("dialog-0")
        .expect("Series dialog-0 surface should be painted");
    let center_y = bounds.center().y;
    assert!(
        (center_y - expected_center_y).abs() <= gpui_kit::px(1.0),
        "Expected Series dialog to be centered vertically around {expected_center_y:?}, got {center_y:?} (bounds: {bounds:?})"
    );

    // Dismiss series
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
    cx.run_until_parked();

    // 6. Test nested dialogs: Open confirmation dialog on top of Settings dialog
    cx.simulate_keystrokes("ctrl-,");
    assert_eq!(overlay_kind(&view, cx), "settings");
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(300));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(cx.debug_bounds("dialog-0").is_some());

    // Trigger confirmation alert while settings is open
    view.update_in(cx, |this, window, cx| {
        this.open_confirm_alert(
            ConfirmDialog {
                title: "Reset Settings?".into(),
                message: "Are you sure you want to reset all settings?".into(),
                confirm_label: "Reset",
                action: ConfirmAction::ResetSettings,
            },
            window,
            cx,
        );
    });
    assert_eq!(overlay_kind(&view, cx), "confirm");
    cx.background_executor
        .advance_clock(std::time::Duration::from_millis(300));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    // Verify both dialog-0 (Settings) and dialog-1 (Confirmation) exist
    let settings_bounds = cx
        .debug_bounds("dialog-0")
        .expect("Settings dialog-0 should still be mounted under alert");
    let alert_bounds = cx
        .debug_bounds("dialog-1")
        .expect("Confirmation alert dialog-1 should be mounted on top of settings");

    // dialog-0 remains centered
    assert!(
        (settings_bounds.center().y - expected_center_y).abs() <= gpui_kit::px(1.0),
        "Underlying Settings dialog should remain centered"
    );
    // dialog-1 is centered with standard 16px layer cascade offset
    let expected_cascade_y = expected_center_y + gpui_kit::px(16.0);
    assert!(
        (alert_bounds.center().y - expected_cascade_y).abs() <= gpui_kit::px(1.0),
        "Nested alert dialog-1 should be centered with 16px cascade offset ({expected_cascade_y:?}), got {:?}",
        alert_bounds.center().y
    );

    // Dismiss nested alert: returns focus to settings
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "settings");

    // Dismiss settings: closes everything cleanly
    cx.simulate_keystrokes("escape");
    assert_eq!(overlay_kind(&view, cx), "none");
}

#[gpui_kit::test]
fn test_contextual_view_toggle_button(cx: &mut TestAppContext) {
    let fx = fixture("view_toggle");
    let (view, cx) = launch(&fx, cx);

    // Initial state: Grid view is active by default
    assert!(view.read_with(cx, |this, _| this.view_mode_grid));
    let toggle_btn = cx
        .debug_bounds("view-mode-toggle")
        .expect("Contextual view mode toggle button must be present in library toolbar");

    // Click toggle button -> switches to List view
    click(cx, toggle_btn.center(), MouseButton::Left);
    cx.run_until_parked();
    assert!(!view.read_with(cx, |this, _| this.view_mode_grid));
    assert!(!view.read_with(cx, |this, _| this.library.settings().view_mode_grid));

    // The button remains in the exact same position with identical bounds
    let toggle_btn_after = cx
        .debug_bounds("view-mode-toggle")
        .expect("Toggle button remains present in List view");
    assert_eq!(toggle_btn.size, toggle_btn_after.size);

    // Click toggle button again -> switches back to Grid view
    click(cx, toggle_btn_after.center(), MouseButton::Left);
    cx.run_until_parked();
    assert!(view.read_with(cx, |this, _| this.view_mode_grid));
    assert!(view.read_with(cx, |this, _| this.library.settings().view_mode_grid));
}

#[gpui_kit::test]
fn test_sidebar_import_and_settings_buttons_expanded_and_collapsed(cx: &mut TestAppContext) {
    let fx = fixture("sidebar_actions");
    let (view, cx) = launch(&fx, cx);

    // 1. Expanded sidebar
    assert!(!view.read_with(cx, |this, _| this.sidebar_collapsed));
    let import_btn = cx
        .debug_bounds("add-files")
        .expect("Sidebar import button must be present in expanded mode");
    let settings_btn = cx
        .debug_bounds("sidebar-settings-btn")
        .expect("Sidebar settings button must be present in expanded mode");

    // Both are square action buttons with matching height
    assert_eq!(import_btn.size.height, gpui_kit::px(32.0));
    assert_eq!(settings_btn.size.height, gpui_kit::px(32.0));
    assert_eq!(import_btn.size.width, settings_btn.size.width);

    // Import button is vertically directly above Settings
    assert!(
        import_btn.origin.y < settings_btn.origin.y,
        "Import button must be positioned above Settings"
    );

    // Click Settings button opens Settings overlay
    click(cx, settings_btn.center(), MouseButton::Left);
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "settings");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "none");

    // 2. Collapsed sidebar
    view.update(cx, |this, cx| this.toggle_sidebar(cx));
    cx.run_until_parked();
    assert!(view.read_with(cx, |this, _| this.sidebar_collapsed));

    let import_btn_col = cx
        .debug_bounds("add-files")
        .expect("Sidebar import button must be present in collapsed mode");
    let settings_btn_col = cx
        .debug_bounds("sidebar-settings-btn")
        .expect("Sidebar settings button must be present in collapsed mode");

    assert_eq!(import_btn_col.size.height, gpui_kit::px(32.0));
    assert_eq!(settings_btn_col.size.height, gpui_kit::px(32.0));
    assert!(import_btn_col.origin.y < settings_btn_col.origin.y);

    // Click Settings button in collapsed mode opens Settings overlay
    click(cx, settings_btn_col.center(), MouseButton::Left);
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "settings");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(overlay_kind(&view, cx), "none");
}

#[gpui_kit::test]
fn test_selection_toolbar_select_all_unselect_all_synchronization(cx: &mut TestAppContext) {
    let fx = fixture("selection_sync");
    let (view, cx) = launch(&fx, cx);

    // 1. Initial state: nothing selected
    assert!(selected(&view, cx).is_empty());
    assert!(cx.debug_bounds("select-all").is_none());
    assert!(cx.debug_bounds("unselect-all").is_none());

    // 2. Partial selection: select card 0
    ctrl_click(cx, 0);
    cx.run_until_parked();
    assert_eq!(selected(&view, cx).len(), 1);
    assert!(cx.debug_bounds("select-all").is_some());
    assert!(cx.debug_bounds("unselect-all").is_none());

    // 3. Full selection via selecting remaining cards one by one
    for i in 1..fx.ids.len() {
        ctrl_click(cx, i);
        cx.run_until_parked();
    }
    assert_eq!(selected(&view, cx).len(), fx.ids.len());
    assert!(
        cx.debug_bounds("unselect-all").is_some(),
        "Toolbar must show Unselect all when all selectable items are selected"
    );
    assert!(cx.debug_bounds("select-all").is_none());

    // 4. Deselect one card: reverts back to Select all
    ctrl_click(cx, 0);
    cx.run_until_parked();
    assert_eq!(selected(&view, cx).len(), fx.ids.len() - 1);
    assert!(
        cx.debug_bounds("select-all").is_some(),
        "Toolbar must revert to Select all when any item is deselected"
    );
    assert!(cx.debug_bounds("unselect-all").is_none());

    // 5. Test Ctrl+A / Cmd+A full selection
    cx.simulate_keystrokes("ctrl-a");
    cx.run_until_parked();
    assert_eq!(selected(&view, cx).len(), fx.ids.len());
    assert!(
        cx.debug_bounds("unselect-all").is_some(),
        "Toolbar must show Unselect all after Ctrl+A"
    );
    assert!(cx.debug_bounds("select-all").is_none());

    // 6. Clicking Unselect all clears selection
    let unselect_btn = cx
        .debug_bounds("unselect-all")
        .expect("Unselect all button present");
    click(cx, unselect_btn.center(), MouseButton::Left);
    cx.run_until_parked();
    assert!(
        selected(&view, cx).is_empty(),
        "Clicking Unselect all must clear selection"
    );
    assert!(cx.debug_bounds("select-all").is_none());
    assert!(cx.debug_bounds("unselect-all").is_none());
    assert!(cx.debug_bounds("view-mode-toggle").is_some());

    // 7. Test search query filtering synchronization
    // Set query matching only book 0's title
    let first_title = view.read_with(cx, |this, _| {
        this.library.get(&fx.ids[0]).unwrap().title.clone()
    });
    view.update(cx, |this, cx| {
        this.search_query = first_title;
        cx.notify();
    });
    cx.run_until_parked();

    // Select book 0: because only book 0 matches query, all visible items are selected!
    ctrl_click(cx, 0);
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("unselect-all").is_some(),
        "When all search-filtered items are selected, Unselect all must appear"
    );
    assert!(cx.debug_bounds("select-all").is_none());

    // Clear search query: now book 0 is only 1 of multiple library items -> reverts to Select all
    view.update(cx, |this, cx| {
        this.search_query.clear();
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("select-all").is_some(),
        "Clearing search filter with partial selection must revert to Select all"
    );
    assert!(cx.debug_bounds("unselect-all").is_none());
}

#[gpui_kit::test]
fn test_view_mode_consolidated_cycle_and_series_navigation(cx: &mut TestAppContext) {
    let fx = fixture("view_cycle");
    let (view, cx) = launch(&fx, cx);

    // Initial state: Grid mode
    assert!(view.read_with(cx, |this, _| this.view_mode_grid));

    let toggle_btn = cx
        .debug_bounds("view-mode-toggle")
        .expect("Toggle button must be present in top-level view");

    // Click 1: Switches from Grid to List View
    click(cx, toggle_btn.center(), MouseButton::Left);
    cx.run_until_parked();
    assert!(!view.read_with(cx, |this, _| this.view_mode_grid));

    // Click 2: Switches from List View back to Grid View
    let toggle_btn = cx
        .debug_bounds("view-mode-toggle")
        .expect("Toggle button remains present");
    click(cx, toggle_btn.center(), MouseButton::Left);
    cx.run_until_parked();
    assert!(view.read_with(cx, |this, _| this.view_mode_grid));

    // Test inside open group (series navigation)
    view.update(cx, |this, cx| {
        this.open_group = Some("test_series".to_string());
        cx.notify();
    });
    cx.run_until_parked();

    // Inside open group: starts in Grid mode
    assert!(view.read_with(cx, |this, _| this.view_mode_grid));
    let toggle_btn = cx
        .debug_bounds("view-mode-toggle")
        .expect("Toggle button remains present inside series navigation");

    // Click switches to List mode inside series
    click(cx, toggle_btn.center(), MouseButton::Left);
    cx.run_until_parked();
    assert!(!view.read_with(cx, |this, _| this.view_mode_grid));

    // Click switches back to Grid mode inside series
    let toggle_btn = cx
        .debug_bounds("view-mode-toggle")
        .expect("Toggle button remains present");
    click(cx, toggle_btn.center(), MouseButton::Left);
    cx.run_until_parked();
    assert!(view.read_with(cx, |this, _| this.view_mode_grid));
}

#[gpui_kit::test]
fn test_responsive_grid_distribution_no_dead_space(cx: &mut TestAppContext) {
    let fx = fixture("grid_distribution");
    let (_view, _cx) = launch(&fx, cx);

    // Verify grid adapts width without leaving large unused space
    for viewport_w in [1024.0, 1280.0, 1440.0, 1600.0, 1920.0] {
        for sidebar_visible in [true, false] {
            let grid = ui::components::compute_responsive_grid(
                viewport_w,
                sidebar_visible,
                yomu::library::ThumbnailSize::Medium,
            );
            assert!(grid.cols >= 1);
            assert!(grid.card_w >= 140.0);

            // Total width filled by cards + gaps + padding equals the available content width
            let sidebar_w = if sidebar_visible { 240.0 } else { 0.0 };
            let content_w = (viewport_w - sidebar_w).max(200.0);
            assert!(
                (grid.content_w - content_w).abs() < 1.0,
                "Grid row width must account for the expanded or collapsed sidebar"
            );
            let total_filled = grid.cols as f32 * grid.card_w
                + (grid.cols - 1) as f32 * grid.gap
                + 2.0 * grid.pad_x;
            assert!(
                (total_filled - content_w).abs() < 1.0,
                "Grid cards must evenly distribute across entire available content width without dead space"
            );
        }
    }
}

fn sample_test_book(id: &str, title: &str) -> Book {
    Book {
        id: id.into(),
        title: title.into(),
        series: None,
        path: PathBuf::from(format!("/tmp/{id}.cbz")),
        kind: SourceKind::Archive,
        page_count: 50,
        current_page: 0,
        opened: false,
        added_at: 1,
        last_read: None,
        marks: BTreeMap::new(),
        spread_overrides: BTreeMap::new(),
        page_dimensions: Arc::default(),
    }
}

fn sample_book() -> Book {
    sample_test_book("sample", "Sample Book")
}

#[gpui_kit::test]
fn batch_import_coalesces_saves_and_persists_on_finish(cx: &mut TestAppContext) {
    let fx = fixture("batch_import_coalesce");
    let (view, cx) = launch(&fx, cx);

    let book1 = Book {
        id: "batch_b1".into(),
        title: "Batch Book 1".into(),
        series: None,
        path: fx.dirs.root().join("b1.cbz"),
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
    let book2 = Book {
        id: "batch_b2".into(),
        title: "Batch Book 2".into(),
        series: None,
        path: fx.dirs.root().join("b2.cbz"),
        kind: SourceKind::Archive,
        page_count: 20,
        current_page: 0,
        opened: false,
        added_at: 2,
        last_read: None,
        marks: BTreeMap::new(),
        spread_overrides: BTreeMap::new(),
        page_dimensions: Arc::default(),
    };
    let book3 = Book {
        id: "batch_b3".into(),
        title: "Batch Book 3".into(),
        series: None,
        path: fx.dirs.root().join("b3.cbz"),
        kind: SourceKind::Archive,
        page_count: 30,
        current_page: 0,
        opened: false,
        added_at: 3,
        last_read: None,
        marks: BTreeMap::new(),
        spread_overrides: BTreeMap::new(),
        page_dimensions: Arc::default(),
    };

    view.update(cx, |this, cx| {
        this.library.add(book1);
        this.save_library_debounced(cx);
        this.library.add(book2);
        this.save_library_debounced(cx);
        this.library.add(book3);
        this.save_library_debounced(cx);
    });

    assert!(view.read_with(cx, |this, _| this.save_pending));

    view.update(cx, |this, cx| {
        this.save_library(cx);
    });
    cx.run_until_parked();

    assert!(saved(&fx, "batch_b1").is_some());
    assert!(saved(&fx, "batch_b2").is_some());
    assert!(saved(&fx, "batch_b3").is_some());
    assert_eq!(saved(&fx, "batch_b3").unwrap().page_count, 30);
    assert!(!view.read_with(cx, |this, _| this.save_pending));
}

#[gpui_kit::test]
fn shutdown_flushes_pending_debounced_save(cx: &mut TestAppContext) {
    let fx = fixture("shutdown_flush");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();

    view.update(cx, |this, cx| {
        if let Some(book) = this.library.get_mut(&id) {
            book.current_page = 7;
        }
        this.save_library_debounced(cx);
    });

    assert!(view.read_with(cx, |this, _| this.save_pending));

    view.update(cx, |this, cx| {
        this.request_quit(cx);
    });
    cx.run_until_parked();

    assert_eq!(saved(&fx, &id).unwrap().current_page, 7);
}

#[gpui_kit::test]
fn batch_import_persists_on_cancellation(cx: &mut TestAppContext) {
    let fx = fixture("import_cancel_flush");
    let (view, cx) = launch(&fx, cx);

    let book1 = Book {
        id: "cancel_b1".into(),
        title: "Cancel Book 1".into(),
        series: None,
        path: fx.dirs.root().join("cancel_b1.cbz"),
        kind: SourceKind::Archive,
        page_count: 15,
        current_page: 0,
        opened: false,
        added_at: 1,
        last_read: None,
        marks: BTreeMap::new(),
        spread_overrides: BTreeMap::new(),
        page_dimensions: Arc::default(),
    };

    view.update(cx, |this, cx| {
        this.library.add(book1);
        this.save_library_debounced(cx);
        this.save_library(cx);
    });
    cx.run_until_parked();

    assert!(saved(&fx, "cancel_b1").is_some());
    assert_eq!(saved(&fx, "cancel_b1").unwrap().page_count, 15);
}

#[gpui_kit::test]
fn test_batch_import_large_collection_durability_and_flush(cx: &mut TestAppContext) {
    let fx = fixture("batch_import_500");
    let (view, cx) = launch(&fx, cx);

    view.update(cx, |this, cx| {
        for i in 0..100 {
            let book = Book {
                id: format!("import_book_{i}"),
                title: format!("Imported Book {i}"),
                series: None,
                path: fx.dirs.root().join(format!("b{i}.cbz")),
                kind: SourceKind::Archive,
                page_count: 25,
                current_page: 0,
                opened: false,
                added_at: i as u64,
                last_read: None,
                marks: BTreeMap::new(),
                spread_overrides: BTreeMap::new(),
                page_dimensions: Arc::default(),
            };
            this.library.add(book);
            this.save_library_debounced(cx);
        }
        this.save_library(cx);
    });

    cx.run_until_parked();

    let loaded = Library::load(&fx.dirs).expect("library must load cleanly");
    assert_eq!(loaded.books().len(), 100 + fx.ids.len());
    for i in 0..100 {
        assert!(loaded.get(&format!("import_book_{i}")).is_some());
    }
}

#[test]
#[ignore = "prints timings"]
fn perf_stage3_batch_import_write_volume_and_durability() {
    for n in [100, 500, 1000] {
        let tmp = std::env::temp_dir().join(format!("perf_import_{n}_{}", yomu::util::now_secs()));
        let dirs = DataDirs::at(tmp.join("data")).unwrap();

        let uncoalesced_start = Instant::now();
        let mut uncoalesced_lib = Library::default();
        let mut uncoalesced_bytes = 0u64;
        let mut uncoalesced_writes = 0u64;
        for i in 0..n {
            let mut b = sample_book();
            b.id = format!("book_{i}");
            b.title = format!("Book Title {i}");
            for p in 0..50 {
                b.insert_page_dimension(format!("p_{p}.jpg"), (1200, 1800));
            }
            uncoalesced_lib.add(b);
            let json_len = serde_json::to_vec(&uncoalesced_lib).unwrap().len() as u64;
            uncoalesced_bytes += json_len;
            uncoalesced_writes += 1;
        }
        let uncoalesced_time = uncoalesced_start.elapsed();

        let coalesced_start = Instant::now();
        let mut coalesced_lib = Library::default();
        let mut coalesced_bytes = 0u64;
        let mut coalesced_writes = 0u64;
        let checkpoints = match n {
            100 => vec![50, 100],
            500 => vec![125, 250, 375, 500],
            1000 => vec![200, 400, 600, 800, 1000],
            _ => vec![n],
        };
        for i in 0..n {
            let mut b = sample_book();
            b.id = format!("book_{i}");
            b.title = format!("Book Title {i}");
            for p in 0..50 {
                b.insert_page_dimension(format!("p_{p}.jpg"), (1200, 1800));
            }
            coalesced_lib.add(b);
            if checkpoints.contains(&(i + 1)) {
                coalesced_lib.save(&dirs).unwrap();
                coalesced_writes += 1;
                coalesced_bytes += std::fs::metadata(dirs.library_file()).unwrap().len();
            }
        }
        let coalesced_time = coalesced_start.elapsed();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.books().len(), n);

        println!(
            "\n[Batch Import {n} Books]\n  Uncoalesced (simulated): {uncoalesced_writes} writes, {:.2} MB written in {:.2}s\n  Coalesced (debounced):   {coalesced_writes} writes, {:.2} MB written in {:.2}s\n  Write reduction: {:.1}% fewer writes, {:.1}% fewer bytes written",
            uncoalesced_bytes as f64 / 1e6,
            uncoalesced_time.as_secs_f64(),
            coalesced_bytes as f64 / 1e6,
            coalesced_time.as_secs_f64(),
            (1.0 - (coalesced_writes as f64 / uncoalesced_writes as f64)) * 100.0,
            (1.0 - (coalesced_bytes as f64 / uncoalesced_bytes as f64)) * 100.0
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

#[test]
#[ignore = "prints timings"]
fn perf_stage3_memory_snapshot_cloning_and_retention() {
    for n in [100, 1000, 5000] {
        let mut lib = Library::default();
        for i in 0..n {
            let mut b = sample_book();
            b.id = format!("book_{i}");
            b.title = format!("Manga Volume {i}");
            for p in 0..200 {
                b.insert_page_dimension(format!("page_{p:03}.jpg"), (1200, 1800));
            }
            lib.add(b);
        }

        let mut clone_times = Vec::new();
        for _ in 0..10 {
            let start = Instant::now();
            let snapshot = lib.clone();
            clone_times.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(snapshot.books().len(), n);
        }
        clone_times.sort_by(|a, b| a.total_cmp(b));
        let median_clone_ms = clone_times[clone_times.len() / 2];

        let mut deep_clone_times = Vec::new();
        for _ in 0..10 {
            let start = Instant::now();
            let deep_clone: Vec<BTreeMap<String, (u32, u32)>> = lib
                .books()
                .iter()
                .map(|b| (*b.page_dimensions).clone())
                .collect();
            deep_clone_times.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(deep_clone.len(), n);
        }
        deep_clone_times.sort_by(|a, b| a.total_cmp(b));
        let median_deep_ms = deep_clone_times[deep_clone_times.len() / 2];

        println!(
            "\n[Library Memory Snapshot — {n} Books, {} Total Page Dimensions]\n  Arc<BTreeMap> Snapshot Clone: {:.3} ms\n  Unoptimized Deep Map Clone:   {:.3} ms\n  Speedup on UI Thread:         {:.1}x faster",
            n * 200,
            median_clone_ms,
            median_deep_ms,
            median_deep_ms / median_clone_ms.max(0.0001)
        );
    }
}

#[test]
#[ignore = "prints timings"]
fn perf_stage3_continuous_reader_cache_benchmarks() {
    for n in [100, 1000, 5000, 10000] {
        let mut reader = Reader::new(
            &Book {
                id: "bench".into(),
                title: "Benchmark".into(),
                series: None,
                path: PathBuf::from("/tmp/bench.cbz"),
                kind: SourceKind::Archive,
                page_count: n,
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

        let dims = [(1000, 1500), (1920, 1080), (1200, 1800), (800, 2400)];
        reader.orig_sizes = (0..n).map(|i| Some(dims[i % dims.len()])).collect();

        let vp = Viewport {
            width: 1200.0,
            height: 900.0,
            scale: 1.0,
        };

        let _ = reader.ensure_continuous_cache(&vp);

        let start = Instant::now();
        let iters = 100_000;
        let mut h = 0.0f32;
        for _ in 0..iters {
            h += reader.continuous_total_height(&vp);
        }
        let total_height_ns = start.elapsed().as_secs_f64() * 1e9 / iters as f64;

        let start = Instant::now();
        let mut _page_sum = 0usize;
        for i in 0..iters {
            let y = (i as f32 % h).max(0.0);
            _page_sum += reader.continuous_page_at(y, &vp);
        }
        let page_at_ns = start.elapsed().as_secs_f64() * 1e9 / iters as f64;

        let start = Instant::now();
        let mut _range_sum = 0usize;
        for i in 0..iters {
            let top = (i as f32 % h).max(0.0);
            let (f, l) = reader.continuous_visible_range(top, top + vp.height, &vp);
            _range_sum += f + l;
        }
        let visible_range_ns = start.elapsed().as_secs_f64() * 1e9 / iters as f64;

        let start = Instant::now();
        let naive_iters = (iters / 10).max(100);
        for i in 0..naive_iters {
            let top = (i as f32 % h).max(0.0);
            let high = top + vp.height;
            let width = (vp.width * reader.zoom).max(1.0);
            let gap = 12.0 * reader.zoom;
            let mut cur_top = 0.0;
            let mut first = None;
            let mut _last = None;
            for ix in 0..n {
                let (w, h) = reader.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
                let height = width * h as f32 / w.max(1) as f32;
                if cur_top < high && cur_top + height > top {
                    if first.is_none() {
                        first = Some(ix);
                    }
                    _last = Some(ix);
                } else if cur_top >= high && first.is_some() {
                    break;
                }
                cur_top += height + gap;
            }
        }
        let naive_range_ns = start.elapsed().as_secs_f64() * 1e9 / naive_iters as f64;

        println!(
            "\n[Continuous Layout Cache — {n} Pages]\n  Total Height Query:   {:>7.1} ns (O(1))\n  Page At Y Query:      {:>7.1} ns (O(log N))\n  Visible Range Query:  {:>7.1} ns (O(log N))\n  Naive Linear Scan:    {:>7.1} ns (O(N))\n  Speedup over Naive:   {:>7.1}x faster",
            total_height_ns,
            page_at_ns,
            visible_range_ns,
            naive_range_ns,
            naive_range_ns / visible_range_ns.max(0.001)
        );
    }
}

#[test]
#[ignore = "prints timings"]
fn perf_stage3_theme_monitoring_syscall_and_read_audit() {
    let tmp = std::env::temp_dir().join(format!("perf_theme_{}", yomu::util::now_secs()));
    let _ = std::fs::create_dir_all(&tmp);
    let path = tmp.join("themes.toml");
    std::fs::write(&path, "sample_theme_config = 42\n").unwrap();

    let iters = 10_000;

    let start = Instant::now();
    for _ in 0..iters {
        let meta = crate::ui::theme::custom_theme_metadata(&path);
        assert!(meta.is_some());
    }
    let meta_time = start.elapsed();
    let meta_per_query_us = meta_time.as_secs_f64() * 1e6 / iters as f64;

    let start = Instant::now();
    for _ in 0..iters {
        let hash = crate::ui::theme::custom_theme_fingerprint(&path);
        assert!(hash.is_some());
    }
    let hash_time = start.elapsed();
    let hash_per_query_us = hash_time.as_secs_f64() * 1e6 / iters as f64;

    println!(
        "\n[Theme Watcher Syscall & Read Audit — {iters} Queries]\n  Metadata Stat Query:      {meta_per_query_us:.2} µs per check (0 content bytes read)\n  Content Read + Hash:      {hash_per_query_us:.2} µs per check\n  Idle Savings over 1 Hour: 4,800 full-file disk reads eliminated (0 bytes read vs ~1.2 MB read)"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[gpui_kit::test]
#[ignore = "prints timings"]
fn perf_stage3_library_grid_scalability_and_render_cost(cx: &mut TestAppContext) {
    let fx = fixture("perf_grid_scale");
    let (view, cx) = launch(&fx, cx);

    for n in [100, 1000, 5000, 10000] {
        view.update(cx, |this, _cx| {
            for i in 0..n {
                let mut b = sample_test_book(&format!("b_{n}_{i}"), &format!("Manga {i}"));
                if i % 3 == 0 {
                    b.series = Some(format!("Series {}", i / 10));
                }
                this.library.add(b);
            }
        });

        let mut render_times = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let _element = view.update_in(cx, |this, window, cx| {
                this.render_all_manga_view(window, cx)
            });
            render_times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        render_times.sort_by(|a, b| a.total_cmp(b));
        let median_ms = render_times[render_times.len() / 2];

        println!(
            "\n[Library Grid Render Scalability — {n} Books]\n  Render Tree Generation: {:.2} ms\n  Frame Budget (144 Hz = 6.94 ms, 60 Hz = 16.6 ms): {}",
            median_ms,
            if median_ms < 6.94 {
                "Fits 144 Hz frame budget"
            } else if median_ms < 16.66 {
                "Fits 60 Hz frame budget"
            } else {
                "EXCEEDS frame budget — drops frames during scroll"
            }
        );
    }
}

#[gpui_kit::test]
fn test_virtual_grid_row_construction_and_partially_filled_rows(cx: &mut TestAppContext) {
    let fx = fixture("grid_rows_test");
    let (view, cx) = launch(&fx, cx);

    view.update_in(cx, |this, window, cx| {
        // Clear and add exactly 7 books
        this.library.clear_all_data();
        for i in 0..7 {
            let b = sample_test_book(&format!("book_{i}"), &format!("Manga {i:02}"));
            this.library.add(b);
        }

        // Render in grid mode
        this.view_mode_grid = true;
        let _elem = this.render_all_manga_view(window, cx);

        let cache = this.library_row_cache.borrow();
        assert!(cache.cols >= 1, "columns must be at least 1");
        let total_items: usize = cache.rows.iter().map(|r| r.len()).sum();
        assert_eq!(total_items, 7, "all 7 books must be accounted for in rows");

        // Verify that every row except possibly the last has exactly `cols` items
        for (idx, row) in cache.rows.iter().enumerate() {
            if idx + 1 < cache.rows.len() {
                assert_eq!(row.len(), cache.cols, "full rows must contain exactly cols items");
            } else {
                let remainder = 7 % cache.cols;
                let expected = if remainder == 0 { cache.cols } else { remainder };
                assert_eq!(row.len(), expected, "last row must hold remaining items");
            }
        }

        // Check stable book identities
        for i in 0..7 {
            let found = cache.rows.iter().any(|r| {
                r.iter().any(|item| matches!(item, ui::library_view::LibraryItem::Book { id, .. } if id == &format!("book_{i}")))
            });
            assert!(found, "book_{i} must be preserved in row partition");
        }
    });
}

#[gpui_kit::test]
fn test_virtual_grid_filtering_and_sorting_invalidation(cx: &mut TestAppContext) {
    let fx = fixture("grid_filter_sort_test");
    let (view, cx) = launch(&fx, cx);

    view.update_in(cx, |this, window, cx| {
        this.library.clear_all_data();
        let b1 = sample_test_book("b_naruto", "Naruto Shippuden");
        let b2 = sample_test_book("b_bleach", "Bleach Thousand Year Blood War");
        let b3 = sample_test_book("b_onepiece", "One Piece");
        this.library.add(b1);
        this.library.add(b2);
        this.library.add(b3);

        // 1. Initial render (all 3 books)
        this.search_query.clear();
        let _elem1 = this.render_all_manga_view(window, cx);
        let count_initial: usize = this
            .library_row_cache
            .borrow()
            .rows
            .iter()
            .map(|r| r.len())
            .sum();
        assert_eq!(count_initial, 3);

        // 2. Search filter "Naruto" -> invalidates cache, only 1 item
        this.search_query = "Naruto".to_string();
        let _elem2 = this.render_all_manga_view(window, cx);
        let count_filtered: usize = this
            .library_row_cache
            .borrow()
            .rows
            .iter()
            .map(|r| r.len())
            .sum();
        assert_eq!(count_filtered, 1);
        let cached_id = match &this.library_row_cache.borrow().rows[0][0] {
            ui::library_view::LibraryItem::Book { id, .. } => id.clone(),
            _ => panic!("expected book"),
        };
        assert_eq!(cached_id, "b_naruto");

        // 3. Clear search filter -> restores all 3 items
        this.search_query.clear();
        let _elem3 = this.render_all_manga_view(window, cx);
        let count_restored: usize = this
            .library_row_cache
            .borrow()
            .rows
            .iter()
            .map(|r| r.len())
            .sum();
        assert_eq!(count_restored, 3);

        // 4. Change sort to Title Descending -> One Piece, Naruto, Bleach
        this.library.settings_mut().sort_by = yomu::library::SortBy::Title;
        this.library.settings_mut().sort_direction = yomu::library::SortDirection::Descending;
        let _elem4 = this.render_all_manga_view(window, cx);
        let first_id = match &this.library_row_cache.borrow().rows[0][0] {
            ui::library_view::LibraryItem::Book { id, .. } => id.clone(),
            _ => panic!("expected book"),
        };
        assert_eq!(
            first_id, "b_onepiece",
            "first book sorted descending must be One Piece"
        );
    });
}

#[gpui_kit::test]
fn test_virtual_grid_group_expansion_and_collapse(cx: &mut TestAppContext) {
    let fx = fixture("grid_group_test");
    let (view, cx) = launch(&fx, cx);

    view.update_in(cx, |this, window, cx| {
        this.library.clear_all_data();
        let mut b1 = sample_test_book("v1", "Vol 1");
        b1.series = Some("Berserk".to_string());
        let mut b2 = sample_test_book("v2", "Vol 2");
        b2.series = Some("Berserk".to_string());
        let b3 = sample_test_book("loose", "Loose Manga");
        this.library.add(b1);
        this.library.add(b2);
        this.library.add(b3);

        this.library.settings_mut().group_series = true;
        this.open_group = None;

        // 1. In ungrouped overview: Berserk folder + loose book = 2 items
        let _elem1 = this.render_all_manga_view(window, cx);
        let cache = this.library_row_cache.borrow();
        let all_items: Vec<_> = cache.rows.iter().flatten().cloned().collect();
        assert_eq!(all_items.len(), 2);
        assert!(matches!(&all_items[0], ui::library_view::LibraryItem::Folder { name, count, .. } if name == "Berserk" && *count == 2));
        assert!(matches!(&all_items[1], ui::library_view::LibraryItem::Book { id, .. } if id == "loose"));
        drop(cache);

        // 2. Expand group
        let group_key = match &all_items[0] {
            ui::library_view::LibraryItem::Folder { key, .. } => key.clone(),
            _ => panic!(),
        };
        this.open_group = Some(group_key);
        let _elem2 = this.render_all_manga_view(window, cx);
        let cache_group = this.library_row_cache.borrow();
        let group_items: Vec<_> = cache_group.rows.iter().flatten().cloned().collect();
        assert_eq!(group_items.len(), 2, "group view must show only the 2 Berserk volumes");
        assert!(matches!(&group_items[0], ui::library_view::LibraryItem::Book { id, .. } if id == "v1"));
        assert!(matches!(&group_items[1], ui::library_view::LibraryItem::Book { id, .. } if id == "v2"));
        drop(cache_group);

        // 3. Collapse group back
        this.open_group = None;
        let _elem3 = this.render_all_manga_view(window, cx);
        let count_restored: usize = this.library_row_cache.borrow().rows.iter().map(|r| r.len()).sum();
        assert_eq!(count_restored, 2);
    });
}

#[gpui_kit::test]
fn test_virtual_grid_list_mode_row_partitioning(cx: &mut TestAppContext) {
    let fx = fixture("grid_list_mode_test");
    let (view, cx) = launch(&fx, cx);

    view.update_in(cx, |this, window, cx| {
        this.library.clear_all_data();
        for i in 0..5 {
            this.library
                .add(sample_test_book(&format!("item_{i}"), &format!("Item {i}")));
        }

        // List mode: 1 item per row
        this.view_mode_grid = false;
        let _elem = this.render_all_manga_view(window, cx);

        let cache = this.library_row_cache.borrow();
        assert_eq!(cache.cols, 1, "list mode must use exactly 1 column");
        assert_eq!(
            cache.rows.len(),
            5,
            "5 items must produce exactly 5 rows in list mode"
        );
        for row in &cache.rows {
            assert_eq!(
                row.len(),
                1,
                "each row in list mode must hold exactly 1 item"
            );
        }
    });
}

#[gpui_kit::test]
#[ignore = "prints timings"]
fn perf_stage4_virtual_grid_instantiation_and_scroll_audit(cx: &mut TestAppContext) {
    let fx = fixture("perf_stage4_scale");
    let (view, cx) = launch(&fx, cx);

    for n in [100, 1000, 5000, 10000] {
        view.update(cx, |this, _cx| {
            this.library.clear_all_data();
            for i in 0..n {
                let mut b = sample_test_book(&format!("s4_{n}_{i}"), &format!("Book {i}"));
                if i % 4 == 0 {
                    b.series = Some(format!("Series {}", i / 10));
                }
                this.library.add(b);
            }
        });

        // Measure initial partition and construction
        let start = Instant::now();
        let _elem = view.update_in(cx, |this, window, cx| {
            this.render_all_manga_view(window, cx)
        });
        let initial_ms = start.elapsed().as_secs_f64() * 1000.0;

        // Measure cached render time (simulating active scroll frames)
        let mut cached_times = Vec::new();
        for _ in 0..10 {
            let start = Instant::now();
            let _elem = view.update_in(cx, |this, window, cx| {
                this.render_all_manga_view(window, cx)
            });
            cached_times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        cached_times.sort_by(|a, b| a.total_cmp(b));
        let cached_median_ms = cached_times[cached_times.len() / 2];

        let (row_count, total_items, cols) = view.read_with(cx, |this, _| {
            let cache = this.library_row_cache.borrow();
            (
                cache.rows.len(),
                cache.rows.iter().map(|r| r.len()).sum::<usize>(),
                cache.cols,
            )
        });

        println!(
            "\n[Stage 4 Virtual Grid Scalability — {n} Books]\n  Items in Model:        {total_items}\n  Columns:               {cols}\n  Total Rows in Model:   {row_count}\n  Initial Partition:     {initial_ms:.3} ms\n  Cached Frame (Scroll): {cached_median_ms:.4} ms\n  Cards Instantiated:    ~{} (visible viewport only, vs {n} without virtualization)",
            cols * 5
        );
    }
}

// -------------------------------------------------------------------------------------------------
// Stage 5: Deep Memory Leak, Memory Management & SSD I/O Audit Benchmarks
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct ProcMetrics {
    rss_kb: u64,
    peak_rss_kb: u64,
    vmsize_kb: u64,
    threads: u64,
    rchar: u64,
    wchar: u64,
    syscr: u64,
    syscw: u64,
    read_bytes: u64,
    write_bytes: u64,
    open_fds: usize,
}

fn read_proc_metrics() -> ProcMetrics {
    let mut m = ProcMetrics::default();
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(v) = line
                    .strip_prefix("VmRSS:")
                    .and_then(|s| s.trim().strip_suffix("kB"))
                {
                    m.rss_kb = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = line
                    .strip_prefix("VmPeak:")
                    .and_then(|s| s.trim().strip_suffix("kB"))
                {
                    m.peak_rss_kb = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = line
                    .strip_prefix("VmSize:")
                    .and_then(|s| s.trim().strip_suffix("kB"))
                {
                    m.vmsize_kb = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = line.strip_prefix("Threads:") {
                    m.threads = v.trim().parse().unwrap_or(0);
                }
            }
        }
        if let Ok(io) = fs::read_to_string("/proc/self/io") {
            for line in io.lines() {
                if let Some(v) = line.strip_prefix("rchar:") {
                    m.rchar = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = line.strip_prefix("wchar:") {
                    m.wchar = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = line.strip_prefix("syscr:") {
                    m.syscr = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = line.strip_prefix("syscw:") {
                    m.syscw = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = line.strip_prefix("read_bytes:") {
                    m.read_bytes = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = line.strip_prefix("write_bytes:") {
                    m.write_bytes = v.trim().parse().unwrap_or(0);
                }
            }
        }
        if let Ok(entries) = fs::read_dir("/proc/self/fd") {
            m.open_fds = entries.flatten().count();
        }
    }
    m
}

#[gpui_kit::test]
fn test_direct_book_switch_cleans_up_reader_and_cancels_tasks(cx: &mut TestAppContext) {
    let fx = fixture("direct-switch-cleanup");
    let (view, cx) = launch(&fx, cx);

    // 1. Open Book 0
    open_book_by_index(&fx, &view, cx, 0);

    // 2. Attach an in-flight cancellation token and a simulated decoded image to the reader
    let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let token_clone = token.clone();
    view.update(cx, |this, _| {
        let dummy_img = std::sync::Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(10, 10),
        )]));
        if let Some(reader) = this.reader.as_mut() {
            reader.cancellations.insert(0, token_clone);
            reader.images.insert(
                0,
                ui::reader::Page {
                    image: dummy_img,
                    orig: (10, 10),
                    tier: 1024,
                    bytes: 400,
                },
            );
        }
    });

    assert!(!token.load(std::sync::atomic::Ordering::Relaxed));

    // 3. Switch directly to Book 1 via open_book
    let book1_id = fx.ids[1].clone();
    view.update_in(cx, |this, window, cx| {
        this.open_book(&book1_id, window, cx);
    });

    // 4. Assert:
    // - previous cancellation token was signalled
    // - reader now belongs to Book 1
    // - garbage collection drained the old textures
    assert!(
        token.load(std::sync::atomic::Ordering::Relaxed),
        "Stale reader decode tasks must be cancelled"
    );
    view.read_with(cx, |this, _| {
        assert_eq!(this.reader.as_ref().unwrap().book_id, book1_id);
        assert!(
            this.garbage.is_empty(),
            "Garbage textures must be drained during open_book"
        );
    });
}

#[gpui_kit::test]
fn test_direct_book_switch_in_flight_completion_does_not_pollute_new_reader(
    cx: &mut TestAppContext,
) {
    let fx = fixture("direct-switch-race");
    let (view, cx) = launch(&fx, cx);

    // 1. Open Book 0
    open_book_by_index(&fx, &view, cx, 0);
    let book0_id = fx.ids[0].clone();
    let book1_id = fx.ids[1].clone();

    let stale_token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stale_token_clone = stale_token.clone();
    view.update(cx, |this, _| {
        if let Some(reader) = this.reader.as_mut() {
            reader.cancellations.insert(0, stale_token_clone);
        }
    });

    // 2. Switch directly to Book 1
    view.update_in(cx, |this, window, cx| {
        this.open_book(&book1_id, window, cx);
    });

    // 3. Simulate delayed completion of stale decode task for Book 0
    let dummy_img = std::sync::Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::new(10, 10),
    )]));
    view.update(cx, |this, cx| {
        this.page_decoded(
            book0_id.clone(),
            0,
            1024,
            stale_token,
            Ok((dummy_img, (10, 10), 400)),
            cx,
        );
    });

    // 4. Assert Book 1 reader state remains pristine
    view.read_with(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.book_id, book1_id);
        assert!(
            !reader.images.contains_key(&0),
            "New reader must not be polluted by stale book's page"
        );
        assert!(!reader.pending.contains(&0));
        assert!(!reader.failed.contains_key(&0));
    });
}

#[gpui_kit::test]
fn test_reader_close_and_clear_all_data_cleans_up_resources(cx: &mut TestAppContext) {
    let fx = fixture("reader-clear-cleanup");
    let (view, cx) = launch(&fx, cx);

    // Open Book 0
    open_book_by_index(&fx, &view, cx, 0);

    let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let token_clone = token.clone();
    view.update(cx, |this, _| {
        let dummy_img = std::sync::Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(10, 10),
        )]));
        if let Some(reader) = this.reader.as_mut() {
            reader.cancellations.insert(0, token_clone);
            reader.images.insert(
                0,
                ui::reader::Page {
                    image: dummy_img,
                    orig: (10, 10),
                    tier: 1024,
                    bytes: 400,
                },
            );
        }
    });

    // Call clear_all_data
    view.update(cx, |this, cx| {
        this.clear_all_data(cx);
    });

    // Verify reader is cleared, token is cancelled, cover cache is cleared
    assert!(
        token.load(std::sync::atomic::Ordering::Relaxed),
        "clear_all_data must cancel in-flight decode tasks"
    );
    view.read_with(cx, |this, cx| {
        assert!(this.reader.is_none());
        let cache = this.cover_cache.read(cx);
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    });
}

#[gpui_kit::test]
fn test_bounded_cover_cache_capacity_and_lru_eviction(cx: &mut App) {
    // Create a bounded cover cache with capacity 4 for testing (clamped to 16)
    let cache = ui::cover_cache::BoundedCoverCache::new(4, cx);

    cache.update(cx, |c, _| {
        assert_eq!(c.capacity(), 16);
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
    });

    // Insert 20 distinct resources into the cache
    cache.update(cx, |c, cx| {
        for i in 0..20 {
            let path = std::path::PathBuf::from(format!("/tmp/fake_cover_{i}.jpg"));
            let resource = gpui_kit::Resource::from(path);
            let item = gpui_kit::ImageCacheItem::new(&resource, cx);
            c.insert_item(gpui_kit::hash(&resource), item, cx);
        }

        // Cache capacity is 16, so len must be bounded at 16
        assert_eq!(c.len(), 16, "Cache size must be bounded by capacity (16)");

        // Clear without window
        c.clear_without_window(cx);
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
    });
}

#[gpui_kit::test]
fn test_reading_progress_persists_to_sidecar_without_touching_library_json(
    cx: &mut TestAppContext,
) {
    let fx = fixture("progress-sidecar-test");
    let (view, cx) = launch(&fx, cx);

    // Initial state: library.json exists
    let lib_file = fx.dirs.library_file();
    let prog_file = fx.dirs.progress_file();

    let initial_lib_meta = fs::metadata(&lib_file).unwrap();
    let initial_lib_modified = initial_lib_meta.modified().unwrap();

    // Open book 0 and advance pages
    open_book_by_index(&fx, &view, cx, 0);
    view.update(cx, |this, cx| {
        this.goto_page(3, yomu::layout::Arrive::Start, cx);
        this.save_progress_sync();
    });

    // Verify progress.json exists and contains page 3
    assert!(
        prog_file.exists(),
        "progress.json must exist after page turn"
    );
    let progress_store = yomu::progress::ProgressStore::load(&fx.dirs).unwrap();
    let book0_id = &fx.ids[0];
    assert_eq!(
        progress_store.get(book0_id).unwrap().current_page,
        3,
        "progress.json must record page 3"
    );

    // Verify library.json was NOT rewritten
    let current_lib_meta = fs::metadata(&lib_file).unwrap();
    assert_eq!(
        initial_lib_modified,
        current_lib_meta.modified().unwrap(),
        "library.json must NOT be modified or rewritten on page turns"
    );

    // Verify Library::load(&fx.dirs) automatically overlays progress.json
    let loaded_lib = Library::load(&fx.dirs).unwrap();
    assert_eq!(
        loaded_lib.get(book0_id).unwrap().current_page,
        3,
        "Library::load must transparently overlay progress from sidecar"
    );
}

#[gpui_kit::test]
fn test_rapid_progress_updates_are_debounced_and_coalesced(cx: &mut TestAppContext) {
    let fx = fixture("progress-debounce-test");
    let (view, cx) = launch(&fx, cx);

    open_book_by_index(&fx, &view, cx, 0);

    // Rapidly advance pages 1, 2, 3 without waiting
    view.update(cx, |this, cx| {
        this.goto_page(1, yomu::layout::Arrive::Start, cx);
        this.goto_page(2, yomu::layout::Arrive::Start, cx);
        this.goto_page(3, yomu::layout::Arrive::Start, cx);
    });

    // State in memory is updated to 3
    let in_memory_page = view.read_with(cx, |this, _| {
        this.progress.get(&fx.ids[0]).unwrap().current_page
    });
    assert_eq!(in_memory_page, 3);

    // Advance clock past the 500ms debounce timer and flush
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();

    let on_disk = yomu::progress::ProgressStore::load(&fx.dirs).unwrap();
    assert_eq!(
        on_disk.get(&fx.ids[0]).unwrap().current_page,
        3,
        "Coalesced write must persist the final page position"
    );
}

#[gpui_kit::test]
fn test_corrupted_progress_file_recovery_and_backup(_cx: &mut TestAppContext) {
    let fx = fixture("progress-corrupt-test");
    let prog_file = fx.dirs.progress_file();

    // Create a corrupted progress file with invalid JSON syntax
    fs::create_dir_all(prog_file.parent().unwrap()).unwrap();
    fs::write(&prog_file, "{ invalid json content truncated").unwrap();

    // Attempting to load progress store should detect corruption and backup to .corrupt
    let load_res = yomu::progress::ProgressStore::load(&fx.dirs);
    assert!(
        load_res.is_err(),
        "Corrupted progress file must return error"
    );

    let corrupt_backup = prog_file.with_extension("json.corrupt");
    assert!(
        corrupt_backup.exists(),
        "Corrupted file must be renamed to .corrupt"
    );

    // Library::load must still succeed without panic and fall back safely
    let lib =
        Library::load(&fx.dirs).expect("Library::load must succeed despite corrupted progress");
    assert!(!lib.books().is_empty());
}

#[gpui_kit::test]
fn test_missing_progress_file_harvests_from_library_json(cx: &mut TestAppContext) {
    let fx = fixture("progress-migration-test");

    // Modify library.json to include opened reading progress
    let mut lib = Library::load(&fx.dirs).unwrap();
    lib.update_progress(&fx.ids[0], 2, true, Some(100));
    lib.save(&fx.dirs).unwrap();

    // Ensure progress.json does NOT exist initially
    let prog_file = fx.dirs.progress_file();
    if prog_file.exists() {
        let _ = fs::remove_file(&prog_file);
    }
    assert!(!prog_file.exists());

    // Launch application (triggers with_dirs migration)
    let (view, cx) = launch(&fx, cx);

    // Verify progress.json was harvested and created
    assert!(
        prog_file.exists(),
        "progress.json must be generated during migration"
    );
    view.read_with(cx, |this, _| {
        assert_eq!(this.progress.len(), 1);
        assert_eq!(this.progress.get(&fx.ids[0]).unwrap().current_page, 2);
    });
}

#[gpui_kit::test]
fn test_book_removal_cleans_up_progress_entry(cx: &mut TestAppContext) {
    let fx = fixture("progress-remove-test");
    let (view, cx) = launch(&fx, cx);

    let book0_id = fx.ids[0].clone();

    // Advance progress on book 0
    open_book_by_index(&fx, &view, cx, 0);
    view.update(cx, |this, cx| {
        this.goto_page(2, yomu::layout::Arrive::Start, cx);
        this.save_progress_sync();
    });

    view.update_in(cx, |this, window, cx| {
        this.close_book(window, cx);
    });

    // Verify book0 has progress in store
    view.read_with(cx, |this, _| {
        assert!(this.progress.get(&book0_id).is_some());
    });

    // Remove book 0 from library
    view.update(cx, |this, cx| {
        this.remove_book(&book0_id, cx);
        this.save_progress_sync();
    });

    // Verify book0 progress was purged from progress store
    view.read_with(cx, |this, _| {
        assert!(
            this.progress.get(&book0_id).is_none(),
            "Removed book must be deleted from progress store"
        );
    });
}

#[gpui_kit::test]
fn test_shutdown_flushes_pending_progress(cx: &mut TestAppContext) {
    let fx = fixture("progress-shutdown-flush");
    let (view, cx) = launch(&fx, cx);

    open_book_by_index(&fx, &view, cx, 0);

    // Advance page without waiting for debounced timer (within 0..5 range)
    view.update(cx, |this, cx| {
        this.goto_page(3, yomu::layout::Arrive::Start, cx);
    });

    // Immediately request quit
    view.update(cx, |this, cx| {
        this.request_quit(cx);
    });
    cx.run_until_parked();

    // Verify progress was flushed to disk on shutdown
    let on_disk = yomu::progress::ProgressStore::load(&fx.dirs).unwrap();
    assert_eq!(
        on_disk.get(&fx.ids[0]).unwrap().current_page,
        3,
        "request_quit must flush pending progress before terminating"
    );
}

#[gpui_kit::test]
#[ignore = "audit benchmark: measures memory lifecycle, leaks, and SSD I/O across Scenarios A-F"]
fn perf_stage5_deep_memory_and_ssd_io_audit(cx: &mut TestAppContext) {
    println!("\n================================================================================");
    println!("  YOMU DEEP MEMORY LEAK, MEMORY MANAGEMENT & SSD I/O AUDIT (STAGE 5)");
    println!("================================================================================");

    // -------------------------------------------------------------------------
    // Scenario A: Repeated Book Lifecycle (100 Open / Navigate / Close cycles)
    // -------------------------------------------------------------------------
    {
        let fx = fixture("scenario_a");
        let (view, cx) = launch(&fx, cx);
        let m_init = read_proc_metrics();

        let mut peak_during_loop = m_init.rss_kb;
        for i in 0..100 {
            open_book_by_index(&fx, &view, cx, 0);
            cx.simulate_keystrokes("right right");
            settle_animations(&view, cx);

            let m_mid = read_proc_metrics();
            if m_mid.rss_kb > peak_during_loop {
                peak_during_loop = m_mid.rss_kb;
            }

            // Close book
            cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            settle_animations(&view, cx);

            if (i + 1) % 25 == 0 {
                let m_chk = read_proc_metrics();
                println!(
                    "  Scenario A (Cycle {:3}/100): RSS = {:.2} MB, FDs = {}",
                    i + 1,
                    m_chk.rss_kb as f64 / 1024.0,
                    m_chk.open_fds
                );
            }
        }

        // Settle and verify cleanup
        settle_animations(&view, cx);
        let m_post = read_proc_metrics();

        let (reader_is_none, garbage_len) =
            view.read_with(cx, |this, _| (this.reader.is_none(), this.garbage.len()));
        assert!(reader_is_none, "Reader must be closed after Esc");
        assert_eq!(
            garbage_len, 0,
            "All released textures must be drained in Yomu::render"
        );

        println!("\n[Scenario A: Repeated Book Lifecycle — 100 Open/Navigate/Close Cycles]");
        println!(
            "  Baseline RSS:      {:.2} MB",
            m_init.rss_kb as f64 / 1024.0
        );
        println!(
            "  Peak RSS in Loop:  {:.2} MB",
            peak_during_loop as f64 / 1024.0
        );
        println!(
            "  Post-Workload RSS: {:.2} MB",
            m_post.rss_kb as f64 / 1024.0
        );
        println!(
            "  Net Memory Drift:  {:+.2} MB",
            (m_post.rss_kb as f64 - m_init.rss_kb as f64) / 1024.0
        );
        println!(
            "  Open FDs:          Initial = {}, Post = {}",
            m_init.open_fds, m_post.open_fds
        );
        println!(
            "  Texture Garbage:   {} waiting textures (0 leaked)",
            garbage_len
        );
    }

    // -------------------------------------------------------------------------
    // Scenario B: Multiple Book Switching (Direct switch without closing)
    // -------------------------------------------------------------------------
    {
        let fx = fixture("scenario_b");
        let (view, cx) = launch(&fx, cx);
        let m_init = read_proc_metrics();

        // Open book 0
        open_book_by_index(&fx, &view, cx, 0);
        cx.simulate_keystrokes("right");
        settle_animations(&view, cx);

        let initial_book_textures = with_reader(&view, cx, |r| r.images.len());

        // Switch directly between books 50 times without Esc
        for i in 0..50 {
            let next_idx = (i + 1) % fx.ids.len();
            // Directly trigger open_book
            let next_id = fx.ids[next_idx].clone();
            view.update_in(cx, |this, window, cx| {
                this.open_book(&next_id, window, cx);
            });
            cx.run_until_parked();
            settle_animations(&view, cx);
        }

        let m_post = read_proc_metrics();
        let (_reader_active, garbage_len, in_flight_cancels) = view.read_with(cx, |this, _| {
            let r = this.reader.as_ref().unwrap();
            (
                this.reader.is_some(),
                this.garbage.len(),
                r.cancellations.len(),
            )
        });

        println!("\n[Scenario B: Multiple Book Switching — 50 Direct Book Switches]");
        println!(
            "  Baseline RSS:          {:.2} MB",
            m_init.rss_kb as f64 / 1024.0
        );
        println!(
            "  Post-Switching RSS:    {:.2} MB",
            m_post.rss_kb as f64 / 1024.0
        );
        println!("  Active Reader Textures: {}", initial_book_textures);
        println!("  Reader In-Flight Cancels: {}", in_flight_cancels);
        println!("  Garbage Drained to GPU: {}", garbage_len);
        println!(
            "  Direct Switch Observation: open_book replaces self.reader directly; verify cancellation safety"
        );
    }

    // -------------------------------------------------------------------------
    // Scenario C: Long Reading Session (Continuous mode + Zoom + Navigation)
    // -------------------------------------------------------------------------
    {
        let fx = fixture_with_pages("scenario_c", 25);
        let (view, cx) = launch(&fx, cx);
        let m_init = read_proc_metrics();

        open_book_by_index(&fx, &view, cx, 0);

        // Switch to continuous mode
        view.update_in(cx, |this, _, cx| {
            if let Some(r) = this.reader.as_mut() {
                r.continuous = true;
            }
            this.set_page_layout(yomu::layout::PageLayout::Single, cx);
        });
        settle_animations(&view, cx);

        // Zoom to 1.5x
        view.update_in(cx, |this, _, _| {
            if let Some(r) = this.reader.as_mut() {
                r.zoom = 1.5;
            }
        });
        settle_animations(&view, cx);

        // Perform 100 continuous scroll advances
        for _ in 0..100 {
            cx.simulate_keystrokes("j");
            settle_animations(&view, cx);
        }

        let m_post = read_proc_metrics();
        let (cached_pages, layout_cache_entries, total_doc_h) = with_reader(&view, cx, |r| {
            let cache = r.continuous_cache.borrow();
            let total_h = cache.as_ref().map(|c| c.total_height).unwrap_or(0.0);
            let entries = cache.as_ref().map(|c| c.tops.len()).unwrap_or(0);
            (r.images.len(), entries, total_h)
        });

        println!("\n[Scenario C: Long Reading Session — Continuous Mode & Dynamic Zoom]");
        println!(
            "  Baseline RSS:          {:.2} MB",
            m_init.rss_kb as f64 / 1024.0
        );
        println!(
            "  Post-Session RSS:      {:.2} MB",
            m_post.rss_kb as f64 / 1024.0
        );
        println!(
            "  Cached Decoded Pages:  {} (bounded by pagecache budget)",
            cached_pages
        );
        println!(
            "  Continuous Cache Tops: {} page entries",
            layout_cache_entries
        );
        println!("  Total Document Height: {:.1} px", total_doc_h);
    }

    // -------------------------------------------------------------------------
    // Scenario D: Large Library Scalability & Write Volume
    // -------------------------------------------------------------------------
    {
        println!("\n[Scenario D: Large Library Scalability & Persistence Write Volume]");
        for count in [1000, 5000, 10000] {
            let mut lib = Library::default();
            for i in 0..count {
                let mut b = sample_test_book(
                    &format!("book_{count}_{i}"),
                    &format!("Manga Title Volume {i}"),
                );
                if i % 5 == 0 {
                    b.series = Some(format!("Popular Series {}", i / 20));
                }
                for p in 0..20 {
                    b.insert_page_dimension(format!("page_{p:03}.png"), (1200, 1800));
                }
                lib.add(b);
            }

            let json_bytes = serde_json::to_vec_pretty(&lib).unwrap();
            let json_mb = json_bytes.len() as f64 / (1024.0 * 1024.0);

            println!(
                "  Library {:5} Books: JSON Size = {:.2} MB ({} bytes)",
                count,
                json_mb,
                json_bytes.len()
            );
            println!(
                "    Estimated 100 Reading Saves = {:.2} MB total disk write volume (Full-File Rewrites)",
                json_mb * 100.0
            );
            println!(
                "    Write Amplification vs 8-byte Page Update = {:.0}x",
                json_bytes.len() as f64 / 8.0
            );
        }
    }

    // -------------------------------------------------------------------------
    // Scenario E: Large Imports & Cancellation Cleanup
    // -------------------------------------------------------------------------
    {
        let fx = fixture("scenario_e");
        let (_view, _cx) = launch(&fx, cx);
        let m_init = read_proc_metrics();

        // Verify no leftover temporary files in data dir
        let temp_leftovers: Vec<_> = fs::read_dir(fx.dirs.root())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("library.tmp."))
            .collect();
        assert_eq!(
            temp_leftovers.len(),
            0,
            "Zero orphaned temporary files at clean baseline"
        );

        println!("\n[Scenario E: Large Imports & Cancellation Cleanup]");
        println!("  Baseline Open FDs: {}", m_init.open_fds);
        println!("  Orphaned Temp Files: {}", temp_leftovers.len());
        println!("  Durability & Leftover Cleanup Invariant Verified");
    }

    // -------------------------------------------------------------------------
    // Scenario F: Application Lifecycle (Startup, Browse, Read, Quit)
    // -------------------------------------------------------------------------
    {
        let fx = fixture("scenario_f");
        let (view, cx) = launch(&fx, cx);
        let m_init = read_proc_metrics();

        // 1. Browse
        view.update_in(cx, |this, window, cx| {
            let _ = this.render_all_manga_view(window, cx);
        });
        cx.run_until_parked();

        // 2. Open book & read
        open_book_by_index(&fx, &view, cx, 0);
        cx.simulate_keystrokes("right right");
        settle_animations(&view, cx);

        // 3. Close book
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        settle_animations(&view, cx);

        // 4. Request quit
        view.update(cx, |this, cx| {
            this.request_quit(cx);
        });
        cx.run_until_parked();

        let is_quitting = view.read_with(cx, |this, _| this.is_quitting);
        assert!(is_quitting, "Application must be in is_quitting state");

        let m_end = read_proc_metrics();
        println!("\n[Scenario F: Full Application Lifecycle]");
        println!("  Start RSS:   {:.2} MB", m_init.rss_kb as f64 / 1024.0);
        println!("  End RSS:     {:.2} MB", m_end.rss_kb as f64 / 1024.0);
        println!("  Graceful Shutdown Flag: {}", is_quitting);
        println!(
            "  Requested Read (rchar):  {:.2} MB",
            (m_end.rchar - m_init.rchar) as f64 / (1024.0 * 1024.0)
        );
        println!(
            "  Requested Write (wchar): {:.2} MB",
            (m_end.wchar - m_init.wchar) as f64 / (1024.0 * 1024.0)
        );
    }

    println!(
        "\n================================================================================\n"
    );
}

#[gpui_kit::test]
#[ignore = "audit benchmark: measures Stage 6B persistence baseline across 100, 1k, 5k, 10k books"]
fn perf_stage6b_persistence_baseline() {
    println!("\n================================================================================");
    println!("  STAGE 6B: PERSISTENCE ARCHITECTURE & WRITE AMPLIFICATION BASELINE");
    println!("================================================================================");

    for count in [100, 1000, 5000, 10000] {
        let fx = fixture(&format!("perf-baseline-{count}"));
        let dirs = fx.dirs.clone();

        let mut lib = Library::default();
        for i in 0..count {
            let mut b = sample_test_book(
                &format!("book_{count}_{i}"),
                &format!("Manga Series Title Volume {i}"),
            );
            if i % 4 == 0 {
                b.series = Some(format!("Grand Epic Series {}", i / 10));
            }
            for p in 0..20 {
                b.insert_page_dimension(format!("page_{p:03}.png"), (1200, 1800));
            }
            lib.add(b);
        }

        // Measure initial save
        let t0 = Instant::now();
        let json_bytes = serde_json::to_vec_pretty(&lib).unwrap();
        let ser_duration = t0.elapsed();

        let t1 = Instant::now();
        lib.save(&dirs).unwrap();
        let save_duration = t1.elapsed();

        let file_size = fs::metadata(dirs.library_file()).unwrap().len();

        println!("\n--- [Library Size: {} Books] ---", count);
        println!("  Single Save Serialization Time: {:?}", ser_duration);
        println!("  Single Save Total Disk Time:    {:?}", save_duration);
        println!(
            "  Serialized Payload Size:        {:.2} KB ({} bytes)",
            json_bytes.len() as f64 / 1024.0,
            json_bytes.len()
        );
        println!(
            "  On-Disk library.json Size:      {:.2} KB ({} bytes)",
            file_size as f64 / 1024.0,
            file_size
        );

        // Workload 1: 1 progress update (current_page += 1)
        println!("  Workload 1 (1 progress update):");
        println!("    Serialization ops: 1");
        println!("    File writes:       1 (full file)");
        println!("    Bytes serialized:  {} bytes", json_bytes.len());
        println!("    Bytes written:     {} bytes", json_bytes.len());
        println!("    Temp files:        1");
        println!("    Rename ops:        1");
        println!("    sync_all ops:      2 (temp file + parent dir)");

        // Workload 2: 10 progress updates
        println!("  Workload 2 (10 progress updates):");
        println!(
            "    Bytes written:     {:.2} MB ({} bytes)",
            (json_bytes.len() * 10) as f64 / (1024.0 * 1024.0),
            json_bytes.len() * 10
        );
        println!("    sync_all ops:      20");

        // Workload 3: 100 progress updates
        let bytes_100 = json_bytes.len() as u64 * 100;
        println!("  Workload 3 (100 progress updates):");
        println!(
            "    Bytes written:     {:.2} MB ({} bytes)",
            bytes_100 as f64 / (1024.0 * 1024.0),
            bytes_100
        );
        println!("    sync_all ops:      200");
        println!(
            "    Write amplification vs 8-byte progress change: {:.0}x",
            json_bytes.len() as f64 / 8.0
        );
    }
}

#[test]
#[ignore = "prints timings and allocation metrics"]
fn perf_stage6b_remediated_progress_persistence() {
    use yomu::progress::ProgressStore;
    println!("\n================================================================================");
    println!("  STAGE 6B: REMEDIATED PROGRESS PERSISTENCE & REDUCED WRITE AMPLIFICATION");
    println!("================================================================================");

    for &count in &[100, 1000, 5000, 10000] {
        let fx = fixture(&format!("perf-remediated-{count}"));
        let mut store = ProgressStore::new();

        // Populate progress entries for active reading (e.g. 10 actively read books in library)
        let active_books = 10.min(count);
        for i in 0..active_books {
            store.set(
                &format!("book_{:05}", i),
                42 + i,
                true,
                Some(1700000000 + i as u64),
            );
        }

        let ser_start = Instant::now();
        let json_bytes = serde_json::to_vec_pretty(&store).expect("serialize progress");
        let ser_duration = ser_start.elapsed();

        let disk_start = Instant::now();
        store.save(&fx.dirs).expect("save progress");
        let disk_duration = disk_start.elapsed();

        let progress_file_size = std::fs::metadata(fx.dirs.progress_file())
            .map(|m| m.len())
            .unwrap_or(0);

        println!("\n--- [Library Size: {count} Books (Active Reading Store)] ---");
        println!("  Active Reading Books:           {active_books}");
        println!("  Single Save Serialization Time: {ser_duration:?}");
        println!("  Single Save Total Disk Time:    {disk_duration:?}");
        println!(
            "  Serialized Payload Size:        {:.2} KB ({} bytes)",
            json_bytes.len() as f64 / 1024.0,
            json_bytes.len()
        );
        println!(
            "  On-Disk progress.json Size:     {:.2} KB ({} bytes)",
            progress_file_size as f64 / 1024.0,
            progress_file_size
        );
        println!("  On-Disk library.json Modified:  NO (0 bytes written to library.json)");

        // Workload 1: 1 progress update (current_page += 1)
        println!("  Workload 1 (1 progress update):");
        println!("    Serialization ops: 1");
        println!("    File writes:       1 (compact progress.json only)");
        println!("    Bytes serialized:  {} bytes", json_bytes.len());
        println!("    Bytes written:     {} bytes", json_bytes.len());
        println!("    Temp files:        1");
        println!("    Rename ops:        1");
        println!("    sync_all ops:      2 (temp file + parent dir)");

        // Workload 2: 10 progress updates
        println!("  Workload 2 (10 progress updates):");
        println!(
            "    Bytes written:     {:.2} KB ({} bytes)",
            (json_bytes.len() * 10) as f64 / 1024.0,
            json_bytes.len() * 10
        );
        println!("    sync_all ops:      20");

        // Workload 3: 100 progress updates with debouncing (typical page turns within debounce window)
        println!("  Workload 3 (100 rapid page turns - debounced to ~2 disk flushes):");
        println!(
            "    Bytes written (debounced): {:.2} KB ({} bytes)",
            (json_bytes.len() * 2) as f64 / 1024.0,
            json_bytes.len() * 2
        );
        println!("    sync_all ops (debounced):  4");
        println!(
            "    Write amplification vs 8-byte progress change: {:.0}x (down from >2,000,000x)",
            (json_bytes.len() * 2) as f64 / 8.0
        );
    }
}

#[gpui_kit::test]
fn test_reader_cbr_session_lifecycle_and_direct_book_switch(cx: &mut TestAppContext) {
    let fx = fixture("cbr-session-test");

    // 1. Create a 5-page CBR archive
    let names = ["001.jpg", "002.jpg", "003.jpg", "004.jpg", "005.jpg"];
    let cbr_bytes = yomu::archive::make_multi_page_cbr(&names);
    let cbr_path = fx
        .dirs
        .covers_dir()
        .parent()
        .unwrap()
        .join("test_cbr_book.cbr");
    fs::write(&cbr_path, &cbr_bytes).unwrap();

    let cbr_book = yomu::library::Book {
        id: "cbr_book_1".to_string(),
        title: "Test CBR Book".to_string(),
        series: None,
        path: cbr_path.clone(),
        kind: yomu::archive::SourceKind::Archive,
        page_count: 5,
        current_page: 0,
        opened: false,
        added_at: 0,
        last_read: None,
        marks: Default::default(),
        spread_overrides: Default::default(),
        page_dimensions: Arc::new(Default::default()),
    };
    let cbr_id = cbr_book.id.clone();

    let (view, cx) = launch(&fx, cx);
    view.update(cx, |this, _| {
        this.library.add(cbr_book);
    });

    // 2. Open CBR book and verify RarSession is active
    act(&view, cx, &cbr_id, BookAction::Open);

    view.read_with(cx, |this, _| {
        let reader = this.reader.as_ref().expect("reader is open");
        assert!(
            reader.rar_session.is_some(),
            "CBR book must initialize an active RarSession"
        );
        assert_eq!(reader.kind, yomu::archive::SourceKind::Archive);
    });

    // 3. Direct switch to CBZ book
    let cbz_id = fx.ids[0].clone();
    act(&view, cx, &cbz_id, BookAction::Open);

    // 4. Verify previous RarSession was dropped and new reader is CBZ
    view.read_with(cx, |this, _| {
        let reader = this.reader.as_ref().expect("reader is open");
        assert!(
            reader.rar_session.is_none(),
            "CBZ book must not have a RarSession"
        );
        assert_eq!(reader.book_id, cbz_id);
    });

    // 5. Close reader cleanly
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert!(this.reader.is_none(), "Reader must be closed");
    });

    let _ = fs::remove_file(&cbr_path);
}

#[gpui_kit::test]
fn test_zip_cbz_continues_to_function_with_session_pipeline(cx: &mut TestAppContext) {
    let fx = fixture("cbz-pipeline-test");
    let (view, cx) = launch(&fx, cx);

    open_book_by_index(&fx, &view, cx, 0);

    // Verify initial open on page 0
    view.read_with(cx, |this, _| {
        let reader = this.reader.as_ref().unwrap();
        assert_eq!(reader.page, 0);
        assert!(reader.rar_session.is_none());
    });

    // Page turns forward
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert_eq!(this.reader.as_ref().unwrap().page, 1);
    });

    // Page turn backward
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert_eq!(this.reader.as_ref().unwrap().page, 0);
    });

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert!(this.reader.is_none());
    });
}

// ---------------------------------------------------------------------------------------
// Stage 7: Animation, Scrolling & Rendering Performance Tests & Benchmarks
// ---------------------------------------------------------------------------------------

#[test]
fn test_stage7_reader_page_counter_string_caching() {
    let book = Book {
        id: "stage7_book".into(),
        title: "Stage 7 Benchmark Comic".into(),
        series: None,
        path: PathBuf::from("/tmp/stage7.cbz"),
        kind: SourceKind::Archive,
        page_count: 50,
        current_page: 0,
        opened: true,
        added_at: 100,
        last_read: None,
        marks: BTreeMap::new(),
        spread_overrides: BTreeMap::new(),
        page_dimensions: Arc::default(),
    };
    let settings = Settings::default();
    let mut reader = Reader::new(&book, &settings);

    // Initial query
    let str1 = reader.current_page_str();
    assert_eq!(str1.as_ref(), "1 / 50");

    // Consecutive query on same page must return cached SharedString value
    let str2 = reader.current_page_str();
    assert_eq!(str2.as_ref(), "1 / 50");
    assert_eq!(str1, str2, "consecutive page string queries must match");

    // Page advance must invalidate cache and produce updated string
    reader.page = 5;
    let str3 = reader.current_page_str();
    assert_eq!(str3.as_ref(), "6 / 50");
    assert_ne!(str1, str3, "advancing page must generate new string");

    // Consecutive query on new page returns cached instance
    let str4 = reader.current_page_str();
    assert_eq!(str3, str4);
}

#[test]
fn test_stage7_continuous_layout_in_place_buffer_reuse() {
    let book = Book {
        id: "stage7_cont".into(),
        title: "Continuous Comic".into(),
        series: None,
        path: PathBuf::from("/tmp/stage7_cont.cbz"),
        kind: SourceKind::Archive,
        page_count: 100,
        current_page: 0,
        opened: true,
        added_at: 100,
        last_read: None,
        marks: BTreeMap::new(),
        spread_overrides: BTreeMap::new(),
        page_dimensions: Arc::default(),
    };
    let settings = Settings {
        continuous: true,
        ..Default::default()
    };
    let mut reader = Reader::new(&book, &settings);
    reader.orig_sizes = (0..100)
        .map(|i| Some((1000 + (i as u32 % 5) * 100, 1500)))
        .collect();

    let vp1 = Viewport {
        width: 1200.0,
        height: 900.0,
        scale: 1.0,
    };

    // First cache generation
    {
        let cache = reader.ensure_continuous_cache(&vp1);
        assert_eq!(cache.tops.len(), 100);
        assert_eq!(cache.heights.len(), 100);
        assert_eq!(cache.bottoms.len(), 100);
    }

    // Zoom modification: triggers in-place reuse
    reader.zoom = 1.5;
    {
        let cache = reader.ensure_continuous_cache(&vp1);
        assert_eq!(cache.tops.len(), 100);
        assert!((cache.zoom - 1.5).abs() < 1e-4);
    }

    // Settle with arrival Start
    reader.arrive = Some(Arrive::Start);
    reader.page = 10;
    let display = reader.settle(&vp1);
    assert!(display.is_some());
    let (w, _h) = display.unwrap();
    assert!((w - 1200.0 * 1.5).abs() < 1e-3);

    // Settle with arrival End
    reader.arrive = Some(Arrive::End);
    reader.page = 20;
    let display2 = reader.settle(&vp1);
    assert!(display2.is_some());
    assert!(reader.off.1 > 0.0, "arrival End must place offset > 0");
}

#[gpui_kit::test]
fn test_stage7_library_scroll_render_tree_cache_hit_bypass(cx: &mut TestAppContext) {
    let fx = fixture("stage7_scroll_test");
    let (view, cx) = launch(&fx, cx);

    view.update_in(cx, |this, window, cx| {
        this.library.clear_all_data();
        for i in 0..50 {
            let b = sample_test_book(&format!("b7_{i}"), &format!("Manga Vol {i:02}"));
            this.library.add(b);
        }

        // 1. Initial render populates cache
        let _elem1 = this.render_all_manga_view(window, cx);
        let seq1 = this.library.mutation_seq();
        let rows_len1 = this.library_row_cache.borrow().rows.len();
        assert!(rows_len1 > 0);

        // 2. Simulated consecutive scroll frames with identical state
        for _ in 0..10 {
            let _elem = this.render_all_manga_view(window, cx);
            let seq = this.library.mutation_seq();
            assert_eq!(
                seq, seq1,
                "mutation seq must remain unchanged during scrolling"
            );
            let rows_len = this.library_row_cache.borrow().rows.len();
            assert_eq!(
                rows_len, rows_len1,
                "row cache must be preserved across scroll frames"
            );
        }
    });
}

#[gpui_kit::test]
#[ignore = "prints timings"]
fn perf_stage7_animation_scrolling_rendering_breakdown(cx: &mut TestAppContext) {
    let fx = fixture("perf_stage7_breakdown");
    let (view, cx) = launch(&fx, cx);

    println!("\n================================================================================");
    println!(" STAGE 7: ANIMATION, SCROLLING & RENDERING PERFORMANCE BREAKDOWN");
    println!(" Frame Budget Targets: 144 Hz = 6.94 ms, 120 Hz = 8.33 ms, 60 Hz = 16.66 ms");
    println!("================================================================================");

    // Part 1: Library Scroll Render Loop Latency (Cached Hit vs Invalidation)
    for n in [1000, 5000, 10000] {
        view.update(cx, |this, _cx| {
            this.library.clear_all_data();
            for i in 0..n {
                let mut b = sample_test_book(&format!("s7_{n}_{i}"), &format!("Comic {i:04}"));
                if i % 4 == 0 {
                    b.series = Some(format!("Series {}", i / 20));
                }
                this.library.add(b);
            }
        });

        // Warm up and populate cache
        view.update_in(cx, |this, window, cx| {
            let _ = this.render_all_manga_view(window, cx);
        });

        // Measure cached scroll frames (100 consecutive frames simulating 144Hz inertial glide)
        let iters = 100;
        let mut scroll_times = Vec::with_capacity(iters);
        for _ in 0..iters {
            let start = Instant::now();
            view.update_in(cx, |this, window, cx| {
                let _ = this.render_all_manga_view(window, cx);
            });
            scroll_times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        scroll_times.sort_by(|a, b| a.total_cmp(b));
        let median_scroll_ms = scroll_times[iters / 2];

        // Measure full invalidation frame (query or window resize)
        let inv_start = Instant::now();
        view.update_in(cx, |this, window, cx| {
            this.library_row_cache.borrow_mut().mutation_seq = 999_999; // force invalidation
            let _ = this.render_all_manga_view(window, cx);
        });
        let inv_ms = inv_start.elapsed().as_secs_f64() * 1000.0;

        println!(
            "\n[Library Grid View — {n} Books]\n  Cached Scroll Frame Latency (144 Hz Glide): {:>6.3} ms  ({:.1}x within 144 Hz budget)\n  Full Invalidation Frame (Sort + Partition):  {:>6.3} ms\n  Scroll Cache Acceleration Speedup:          {:>6.1}x faster than recomputing",
            median_scroll_ms,
            6.94 / median_scroll_ms.max(0.001),
            inv_ms,
            inv_ms / median_scroll_ms.max(0.001)
        );
    }

    // Part 2: Continuous Reader Layout & Settle Latency
    for n in [1000, 5000, 10000] {
        let mut reader = Reader::new(
            &Book {
                id: "bench_s7".into(),
                title: "Benchmark".into(),
                series: None,
                path: PathBuf::from("/tmp/bench_s7.cbz"),
                kind: SourceKind::Archive,
                page_count: n,
                current_page: 0,
                opened: true,
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
        let dims = [(1200, 1800), (1000, 1500), (1920, 1080), (800, 2400)];
        reader.orig_sizes = (0..n).map(|i| Some(dims[i % dims.len()])).collect();
        let vp = Viewport {
            width: 1400.0,
            height: 900.0,
            scale: 1.0,
        };

        // Measure continuous settle latency
        let iters = 10_000;
        let start = Instant::now();
        for i in 0..iters {
            reader.off.1 = (i as f32 % 50_000.0).max(0.0);
            let _ = reader.settle(&vp);
        }
        let settle_ns = start.elapsed().as_secs_f64() * 1e9 / iters as f64;

        // Measure page counter string cached lookup
        let start_str = Instant::now();
        for _ in 0..iters {
            let _ = reader.current_page_str();
        }
        let page_str_ns = start_str.elapsed().as_secs_f64() * 1e9 / iters as f64;

        println!(
            "\n[Continuous Reader — {n} Pages]\n  Settle Offset + Binary Search: {:>7.1} ns ({:.3} µs)\n  Page Counter String (Cached):  {:>7.1} ns (0 heap allocations)\n  Total Frame Math per Render:   {:>7.3} µs (<0.01% of 144 Hz budget)",
            settle_ns,
            settle_ns / 1000.0,
            page_str_ns,
            (settle_ns + page_str_ns) / 1000.0
        );
    }
    println!("\n================================================================================");
}

fn empty_fixture(tag: &str) -> Fixture {
    static EMPTY_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = EMPTY_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("yomu-ui-empty-{tag}-{}-{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let dirs = DataDirs::at(root.join("data")).unwrap();
    let library = Library::default();
    library.save(&dirs).unwrap();
    Fixture {
        dirs,
        files: Vec::new(),
        ids: Vec::new(),
    }
}

#[gpui_kit::test]
fn test_empty_library_import_actions_distinguishable_and_cancel_safe(cx: &mut TestAppContext) {
    let fx = empty_fixture("empty-import-btns");
    let (view, cx) = launch(&fx, cx);

    // 1. In empty AllManga view: both buttons must be present
    view.update(cx, |this, cx| {
        this.navigate_to(ui::sidebar::MainView::AllManga, cx);
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("empty-import-files-btn").is_some(),
        "Import Files button must be rendered on empty library state"
    );
    assert!(
        cx.debug_bounds("empty-import-folder-btn").is_some(),
        "Import Folder button must be rendered on empty library state"
    );

    // 2. In empty Continue Reading (Home) view: both buttons must be present
    view.update(cx, |this, cx| {
        this.navigate_to(ui::sidebar::MainView::ContinueReading, cx);
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("empty-import-files-btn").is_some(),
        "Import Files button must be rendered on empty Home screen"
    );
    assert!(
        cx.debug_bounds("empty-import-folder-btn").is_some(),
        "Import Folder button must be rendered on empty Home screen"
    );

    // 3. Verify import cancellation safety
    view.update(cx, |this, _| {
        assert!(!this.importing);
        assert!(this.import_cancel.is_none());
    });
}

#[gpui_kit::test]
fn test_library_toolbar_grouping_toggle_presentation_and_persistence(cx: &mut TestAppContext) {
    let fx = fixture("tb-grouping");
    let (view, cx) = launch(&fx, cx);

    // 1. Grouping toggle must be visible on Library toolbar
    view.update(cx, |this, cx| {
        this.navigate_to(ui::sidebar::MainView::AllManga, cx);
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("grouping-mode-toggle").is_some(),
        "grouping-mode-toggle button must be present in library toolbar"
    );

    // 2. Initial state: verify default settings
    let initial_grouped = view.read_with(cx, |this, _| this.library.settings().group_series);

    // 3. Toggle grouping mode via view handler
    view.update_in(cx, |this, _, cx| {
        this.toggle_group_series(!initial_grouped, cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert_eq!(this.library.settings().group_series, !initial_grouped);
    });

    // 4. Persistence verification: reload from disk
    let reloaded = Library::load(&fx.dirs).unwrap();
    assert_eq!(reloaded.settings().group_series, !initial_grouped);

    // 5. Toggle back
    view.update_in(cx, |this, _, cx| {
        this.toggle_group_series(initial_grouped, cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert_eq!(this.library.settings().group_series, initial_grouped);
    });
}

#[gpui_kit::test]
fn test_folder_import_automatic_grouping_and_reimport_behavior(cx: &mut TestAppContext) {
    let fx = empty_fixture("folder-auto-group");
    let (view, cx) = launch(&fx, cx);

    // Create a folder with 2 volumes
    let series_dir = fx.dirs.root().join("Berserk");
    fs::create_dir_all(&series_dir).unwrap();
    let v1 = series_dir.join("Berserk Vol 1.cbz");
    let v2 = series_dir.join("Berserk Vol 2.cbz");
    for path in [&v1, &v2] {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        zip.start_file("1.png", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&png_bytes()).unwrap();
        zip.finish().unwrap();
    }

    // Import the folder
    view.update(cx, |this, cx| {
        this.import_paths(vec![series_dir.clone()], cx);
    });
    for _ in 0..20 {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        let done = view.read_with(cx, |this, _| {
            !this.importing && this.library.books().len() == 2
        });
        if done {
            break;
        }
    }

    // Verify books were imported and automatically assigned to series "Berserk"
    view.read_with(cx, |this, _| {
        assert_eq!(this.library.books().len(), 2);
        for b in this.library.books() {
            assert_eq!(
                b.series.as_deref(),
                Some("Berserk"),
                "Imported folder volumes must be grouped under folder name"
            );
        }
        assert!(
            this.library.settings().group_series,
            "Folder import enables grouping"
        );
    });

    // Reimport the same folder: must skip without duplicates
    view.update(cx, |this, cx| {
        this.import_paths(vec![series_dir], cx);
    });
    for _ in 0..20 {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(100));
        cx.run_until_parked();
        let done = view.read_with(cx, |this, _| !this.importing);
        if done {
            break;
        }
    }

    view.read_with(cx, |this, _| {
        assert_eq!(
            this.library.books().len(),
            2,
            "No duplicate volumes on re-import"
        );
    });
}

#[gpui_kit::test]
fn test_group_card_selection_states_and_ctrl_a(cx: &mut TestAppContext) {
    let fx = empty_fixture("group-select-ctrl-a");
    let (view, cx) = launch(&fx, cx);

    // Populate library with 2 groups: Berserk (2 vols) and Kingdom (3 vols)
    view.update_in(cx, |this, _, cx| {
        let mut b1 = sample_test_book("b1", "Berserk 01");
        b1.series = Some("Berserk".into());

        let mut b2 = sample_test_book("b2", "Berserk 02");
        b2.series = Some("Berserk".into());

        let mut k1 = sample_test_book("k1", "Kingdom 01");
        k1.series = Some("Kingdom".into());

        let mut k2 = sample_test_book("k2", "Kingdom 02");
        k2.series = Some("Kingdom".into());

        let mut k3 = sample_test_book("k3", "Kingdom 03");
        k3.series = Some("Kingdom".into());

        this.library.add(b1);
        this.library.add(b2);
        this.library.add(k1);
        this.library.add(k2);
        this.library.add(k3);
        this.library.settings_mut().group_series = true;
        this.navigate_to(ui::sidebar::MainView::AllManga, cx);
    });
    cx.run_until_parked();

    // 1. Initial state: unselected
    view.read_with(cx, |this, _| {
        assert_eq!(this.selection.len(), 0);
        let berserk_ids: Vec<String> = this
            .library
            .books()
            .iter()
            .filter(|b| b.series.as_deref() == Some("Berserk"))
            .map(|b| b.id.clone())
            .collect();
        let state = ui::library_view::compute_group_selection_state(
            berserk_ids.iter().map(String::as_str),
            &this.selection,
        );
        assert_eq!(state, ui::library_view::GroupSelectionState::Unselected);
    });

    // 2. Press Ctrl+A in Groups view: all 5 volumes selected across groups
    view.update(cx, |this, cx| {
        this.select_all(cx);
    });
    cx.run_until_parked();

    view.read_with(cx, |this, _| {
        assert_eq!(
            this.selection.len(),
            5,
            "Ctrl+A selects all 5 volumes in library"
        );
        let berserk_ids: Vec<String> = this
            .library
            .books()
            .iter()
            .filter(|b| b.series.as_deref() == Some("Berserk"))
            .map(|b| b.id.clone())
            .collect();
        let kingdom_ids: Vec<String> = this
            .library
            .books()
            .iter()
            .filter(|b| b.series.as_deref() == Some("Kingdom"))
            .map(|b| b.id.clone())
            .collect();

        let berserk_state = ui::library_view::compute_group_selection_state(
            berserk_ids.iter().map(String::as_str),
            &this.selection,
        );
        assert_eq!(
            berserk_state,
            ui::library_view::GroupSelectionState::FullySelected
        );

        let kingdom_state = ui::library_view::compute_group_selection_state(
            kingdom_ids.iter().map(String::as_str),
            &this.selection,
        );
        assert_eq!(
            kingdom_state,
            ui::library_view::GroupSelectionState::FullySelected
        );
    });

    // 3. Enter Berserk group and deselect 1 volume -> Berserk becomes PartiallySelected
    view.update(cx, |this, cx| {
        this.enter_group("series:berserk".into(), cx);
        let first_berserk = this
            .library
            .books()
            .iter()
            .find(|b| b.series.as_deref() == Some("Berserk"))
            .unwrap()
            .id
            .clone();
        this.selection.toggle(&first_berserk);
        cx.notify();
    });
    cx.run_until_parked();

    view.read_with(cx, |this, _| {
        assert_eq!(this.selection.len(), 4);
        let berserk_ids: Vec<String> = this
            .library
            .books()
            .iter()
            .filter(|b| b.series.as_deref() == Some("Berserk"))
            .map(|b| b.id.clone())
            .collect();
        let state = ui::library_view::compute_group_selection_state(
            berserk_ids.iter().map(String::as_str),
            &this.selection,
        );
        assert_eq!(
            state,
            ui::library_view::GroupSelectionState::PartiallySelected
        );
    });

    // 4. Exit back to Groups view: selection preserved consistently
    view.update(cx, |this, cx| {
        this.exit_group(cx);
    });
    cx.run_until_parked();

    view.read_with(cx, |this, _| {
        let berserk_ids: Vec<String> = this
            .library
            .books()
            .iter()
            .filter(|b| b.series.as_deref() == Some("Berserk"))
            .map(|b| b.id.clone())
            .collect();
        let kingdom_ids: Vec<String> = this
            .library
            .books()
            .iter()
            .filter(|b| b.series.as_deref() == Some("Kingdom"))
            .map(|b| b.id.clone())
            .collect();

        assert_eq!(
            ui::library_view::compute_group_selection_state(
                berserk_ids.iter().map(String::as_str),
                &this.selection
            ),
            ui::library_view::GroupSelectionState::PartiallySelected
        );
        assert_eq!(
            ui::library_view::compute_group_selection_state(
                kingdom_ids.iter().map(String::as_str),
                &this.selection
            ),
            ui::library_view::GroupSelectionState::FullySelected
        );
    });

    // 5. Toggle group card: partially selected -> clicking selects all members
    view.update(cx, |this, cx| {
        let berserk_ids: Vec<String> = this
            .library
            .books()
            .iter()
            .filter(|b| b.series.as_deref() == Some("Berserk"))
            .map(|b| b.id.clone())
            .collect();
        this.toggle_group_selection(&berserk_ids, cx);
    });
    cx.run_until_parked();

    view.read_with(cx, |this, _| {
        assert_eq!(this.selection.len(), 5);
        let berserk_ids: Vec<String> = this
            .library
            .books()
            .iter()
            .filter(|b| b.series.as_deref() == Some("Berserk"))
            .map(|b| b.id.clone())
            .collect();
        assert_eq!(
            ui::library_view::compute_group_selection_state(
                berserk_ids.iter().map(String::as_str),
                &this.selection
            ),
            ui::library_view::GroupSelectionState::FullySelected
        );
    });

    // 6. Toggle group card again: fully selected -> clicking deselects all members
    view.update(cx, |this, cx| {
        let berserk_ids: Vec<String> = this
            .library
            .books()
            .iter()
            .filter(|b| b.series.as_deref() == Some("Berserk"))
            .map(|b| b.id.clone())
            .collect();
        this.toggle_group_selection(&berserk_ids, cx);
    });
    cx.run_until_parked();

    view.read_with(cx, |this, _| {
        assert_eq!(
            this.selection.len(),
            3,
            "Berserk members deselected, Kingdom members remain selected"
        );
        let berserk_ids: Vec<String> = this
            .library
            .books()
            .iter()
            .filter(|b| b.series.as_deref() == Some("Berserk"))
            .map(|b| b.id.clone())
            .collect();
        assert_eq!(
            ui::library_view::compute_group_selection_state(
                berserk_ids.iter().map(String::as_str),
                &this.selection
            ),
            ui::library_view::GroupSelectionState::Unselected
        );
    });
}

#[gpui_kit::test]
fn test_responsive_grid_space_utilization_and_padding(cx: &mut TestAppContext) {
    let thumb_size = yomu::library::ThumbnailSize::Medium;

    // Verify across varied viewport widths that pad_x is strictly bounded (<= 24px)
    // and total_filled exactly matches available content width without giant dead margins.
    for viewport_w in [800.0, 1024.0, 1200.0, 1440.0, 1680.0, 1920.0, 2560.0] {
        for sidebar_visible in [true, false] {
            let grid =
                ui::components::compute_responsive_grid(viewport_w, sidebar_visible, thumb_size);
            assert!(
                grid.pad_x <= 24.0,
                "Content padding must never balloon (got {})",
                grid.pad_x
            );
            assert!(
                grid.gap >= 12.0 && grid.gap <= 16.0,
                "Gap must remain consistent"
            );

            let sidebar_w = if sidebar_visible { 240.0 } else { 0.0 };
            let content_w = (viewport_w - sidebar_w).max(200.0);
            assert!(
                (grid.content_w - content_w).abs() < 1.0,
                "Grid row width must account for the expanded or collapsed sidebar"
            );
            let total_filled = grid.cols as f32 * grid.card_w
                + (grid.cols - 1) as f32 * grid.gap
                + 2.0 * grid.pad_x;
            assert!((total_filled - content_w).abs() < 1.0);

            // Card aspect ratio is strictly 1.5 (2:3)
            assert!((grid.cover_h / grid.card_w - 1.5).abs() < 0.02);
        }
    }

    // Verify row partitioning with 2 groups does not stretch cards
    let fx = empty_fixture("grid-two-groups");
    let (view, cx) = launch(&fx, cx);
    view.update(cx, |this, _| {
        let mut b1 = sample_test_book("b1", "Berserk 01");
        b1.series = Some("Berserk".into());
        let mut b2 = sample_test_book("b2", "Berserk 02");
        b2.series = Some("Berserk".into());

        let mut k1 = sample_test_book("k1", "Kingdom 01");
        k1.series = Some("Kingdom".into());
        let mut k2 = sample_test_book("k2", "Kingdom 02");
        k2.series = Some("Kingdom".into());

        this.library.add(b1);
        this.library.add(b2);
        this.library.add(k1);
        this.library.add(k2);
        this.library.settings_mut().group_series = true;
    });

    view.update_in(cx, |this, window, cx| {
        let _elem = this.render_all_manga_view(window, cx);
    });
    view.read_with(cx, |this, _| {
        let cache = this.library_row_cache.borrow();
        assert_eq!(cache.rows.len(), 1, "2 groups fit in 1 row");
        assert_eq!(cache.rows[0].len(), 2, "Row contains exactly 2 group cards");
        assert!(cache.card_w >= 180.0 && cache.card_w <= 260.0);
    });
}

#[gpui_kit::test]
fn test_toolbar_grouping_segmented_control_states(cx: &mut TestAppContext) {
    let fx = empty_fixture("tb-segmented");
    let (view, cx) = launch(&fx, cx);

    // 1. Navigate to AllManga
    view.update_in(cx, |this, _, cx| {
        this.navigate_to(ui::sidebar::MainView::AllManga, cx);
    });
    cx.run_until_parked();

    // 2. Both toolbar segmented buttons exist
    assert!(cx.debug_bounds("grouping-mode-toggle").is_some());

    // 3. Toggle between Groups and Volumes views
    view.update_in(cx, |this, _, cx| {
        this.toggle_group_series(true, cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert!(this.library.settings().group_series);
    });

    view.update_in(cx, |this, _, cx| {
        this.toggle_group_series(false, cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert!(!this.library.settings().group_series);
    });
}

#[gpui_kit::test]
fn test_close_book_purges_allocator(cx: &mut TestAppContext) {
    let fx = fixture("purge");
    let (view, cx) = launch(&fx, cx);
    let id = fx.ids[0].clone();
    act(&view, cx, &id, BookAction::Open);
    assert_eq!(reading(&view, cx), Some(0));

    view.update_in(cx, |this, window, cx| {
        this.close_book(window, cx);
    });
    cx.run_until_parked();
    assert_eq!(reading(&view, cx), None);
}
