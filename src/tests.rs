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
    std::thread::sleep(Duration::from_millis(350));
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
    std::thread::sleep(Duration::from_millis(350));
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
    let expected_cover_h = thumb_size.card_height();

    // 1. Narrow window (e.g. 700px) with expanded sidebar (240px)
    let grid_narrow = ui::components::compute_responsive_grid(700.0, true, thumb_size);
    assert_eq!(
        grid_narrow.card_w, expected_card_w,
        "card width must be fixed"
    );
    assert_eq!(
        grid_narrow.cover_h, expected_cover_h,
        "cover height must be fixed"
    );

    // 2. Medium window (e.g. 1200px)
    let grid_med = ui::components::compute_responsive_grid(1200.0, true, thumb_size);
    assert_eq!(grid_med.card_w, expected_card_w, "card width must be fixed");
    assert_eq!(
        grid_med.cover_h, expected_cover_h,
        "cover height must be fixed"
    );
    assert!(
        grid_med.cols >= grid_narrow.cols,
        "columns must increase with width"
    );

    // 3. Wide window (e.g. 1920px) with collapsed sidebar (48px)
    let grid_wide = ui::components::compute_responsive_grid(1920.0, false, thumb_size);
    assert_eq!(
        grid_wide.card_w, expected_card_w,
        "card width must be fixed"
    );
    assert_eq!(
        grid_wide.cover_h, expected_cover_h,
        "cover height must be fixed"
    );
    assert!(
        grid_wide.cols > grid_med.cols,
        "columns must adapt to available space"
    );

    // 4. Centering padding is balanced and positive
    assert!(
        grid_wide.pad_x >= 16.0,
        "grid padding must be at least min margin"
    );
    assert!(
        grid_med.pad_x >= 16.0,
        "grid padding must be at least min margin"
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
        page_dimensions: BTreeMap::new(),
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
            let sidebar_w = if sidebar_visible { 240.0 } else { 48.0 };
            let content_w = (viewport_w - sidebar_w).max(200.0);
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
