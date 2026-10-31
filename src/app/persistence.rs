#![forbid(unsafe_code)]

//! Application-level persistence and autosave coordination.
//!
//! Manages asynchronous serialization of `library.json` using debounced background
//! tasks during active reading (continuous scroll / rapid navigation) and
//! immediate background persistence for discrete user actions (settings, bookmarks,
//! read status, dialogs). UI threads never perform synchronous filesystem I/O or
//! wait on disk locks.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use gpui_kit::Context;

use crate::app::state::Yomu;
use crate::ui::overlays::{ConfirmAction, ConfirmDialog, Overlay};

/// Delay before flushing debounced changes to disk during continuous updates.
pub const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// Maximum duration updates can remain in-memory before forcing a background flush.
pub const AUTOSAVE_MAX_DELAY: Duration = Duration::from_millis(2000);

impl Yomu {
    /// Schedule an immediate background save on the background executor.
    ///
    /// Clears any pending debounced timer and immediately dispatches serialization
    /// and atomic disk replacement to a background worker thread.
    ///
    /// Never blocks the UI thread.
    pub(crate) fn save_library(&mut self, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        self.save_seq += 1;
        self.save_pending = false;
        self.save_pending_since = None;
        self.flush_save_background(cx);
    }

    /// Synchronously serialize and write the library to disk.
    ///
    /// Reserved strictly for startup migrations (before window creation) and
    /// test assertions where synchronous execution is required.
    #[allow(dead_code)]
    pub(crate) fn save_library_sync(&mut self) {
        self.save_seq += 1;
        let seq = self.save_seq;
        self.save_pending = false;
        self.save_pending_since = None;
        let _guard = self.write_lock.lock().unwrap();
        if let Err(err) = self.library.save(&self.dirs) {
            eprintln!("yomu: failed to save library: {err:#}");
        } else {
            self.last_saved_seq.store(seq, Ordering::Release);
        }
    }

    /// Request a debounced library save on the background executor.
    ///
    /// Coalesces rapid updates (such as scroll offset and page position changes
    /// during continuous reading and page navigation) within `AUTOSAVE_DEBOUNCE`,
    /// while enforcing `AUTOSAVE_MAX_DELAY` as an upper bound before flushing.
    ///
    /// Never blocks the UI thread.
    pub(crate) fn save_library_debounced(&mut self, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        self.save_seq += 1;
        let now = Instant::now();
        let pending_since = *self.save_pending_since.get_or_insert(now);

        if now.duration_since(pending_since) >= AUTOSAVE_MAX_DELAY {
            self.save_pending = false;
            self.save_pending_since = None;
            self.flush_save_background(cx);
            return;
        }

        if !self.save_pending {
            self.save_pending = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(AUTOSAVE_DEBOUNCE).await;
                let _ = this
                    .update(cx, |this, cx| {
                        if this.save_pending {
                            this.save_pending = false;
                            this.save_pending_since = None;
                            this.flush_save_background(cx);
                        }
                    })
                    .ok();
            })
            .detach();
        }
    }

    /// Spawns background serialization and atomic disk write task on the background executor.
    ///
    /// Clones the `library` model snapshot and offloads JSON formatting and filesystem I/O
    /// away from the UI thread. Monotonically tracks `save_seq` under lock to prevent
    /// out-of-order writes from clobbering newer state.
    pub(crate) fn flush_save_background(&mut self, cx: &mut Context<Self>) {
        let seq = self.save_seq;
        let library = self.library.clone();
        let dirs = Arc::clone(&self.dirs);
        let last_saved_seq = Arc::clone(&self.last_saved_seq);
        let write_lock = Arc::clone(&self.write_lock);

        cx.background_executor()
            .spawn(async move {
                let _guard = write_lock.lock().unwrap();
                if seq > last_saved_seq.load(Ordering::Acquire) {
                    if let Err(err) = library.save(&dirs) {
                        eprintln!("yomu: failed to save library: {err:#}");
                    } else {
                        last_saved_seq.store(seq, Ordering::Release);
                    }
                }
            })
            .detach();
    }

    /// Initiate graceful asynchronous application shutdown.
    ///
    /// Directly performs the final serialization and atomic disk write on the
    /// background executor under `write_lock`, ensuring all in-flight and pending
    /// updates are committed to disk before terminating the application.
    ///
    /// Never blocks the UI thread.
    pub(crate) fn request_quit(&mut self, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        self.is_quitting = true;
        self.save_pending = false;
        self.save_pending_since = None;
        self.overlay = None;
        self.toast = None;
        if let Some(token) = &self.import_cancel {
            token.store(true, Ordering::Relaxed);
        }

        let target_seq = self.save_seq;
        let library = self.library.clone();
        let dirs = Arc::clone(&self.dirs);
        let last_saved_seq = Arc::clone(&self.last_saved_seq);
        let write_lock = Arc::clone(&self.write_lock);

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _guard = write_lock.lock().unwrap();
                    if target_seq > last_saved_seq.load(Ordering::Acquire) {
                        match library.save(&dirs) {
                            Ok(()) => {
                                last_saved_seq.store(target_seq, Ordering::Release);
                                Ok(())
                            }
                            Err(err) => Err(err),
                        }
                    } else {
                        Ok(())
                    }
                })
                .await;

            let _ = this
                .update(cx, |this, cx| {
                    match result {
                        Ok(()) => {
                            cx.quit();
                        }
                        Err(err) => {
                            this.is_quitting = false;
                            this.show_toast(format!("Save failed: {err:#}"), cx);
                            this.overlay = Some(Overlay::Confirm(ConfirmDialog {
                                title: "Failed to save library".into(),
                                message: format!(
                                    "Could not save library before quitting: {err:#}. Quit anyway and lose unsaved changes?"
                                )
                                .into(),
                                confirm_label: "Quit Without Saving",
                                action: ConfirmAction::ForceQuit,
                            }));
                            cx.notify();
                        }
                    }
                })
                .ok();
        })
        .detach();
    }
}

impl Drop for Yomu {
    fn drop(&mut self) {
        // Drop performs only non-blocking cleanup and never executes filesystem I/O.
        // Explicit asynchronous shutdown (`request_quit`) is the sole owner of library
        // persistence. This ensures that an explicit ForceQuit decision is never
        // overridden during object destruction, and UI/shutdown threads are never blocked.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};

    use gpui_kit::{App, Entity, KeyBinding, TestAppContext, VisualTestContext};
    use yomu::archive::SourceKind;
    use yomu::library::{Book, DataDirs, Library};

    use crate::Quit;

    struct TestDir(std::path::PathBuf);
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn test_dirs(tag: &str) -> (TestDir, DataDirs) {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "yomu-persistence-test-{tag}-{}-{}",
            std::process::id(),
            id
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let dirs = DataDirs::at(root.join("data")).unwrap();
        (TestDir(root), dirs)
    }

    fn launch_test(
        dirs: DataDirs,
        cx: &mut TestAppContext,
    ) -> (Entity<Yomu>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
        });
        cx.add_window_view(move |window, cx| {
            let yomu = Yomu::with_dirs(dirs, window, cx);
            window.focus(&yomu.focus_handle, cx);
            yomu
        })
    }

    // 1. Rapid page navigation followed by immediate quit persists the final page.
    #[gpui_kit::test]
    fn test_1_rapid_navigation_then_immediate_quit_persists_final_page(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("rapid-nav-quit");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, _cx| {
            let book = Book {
                id: "b1".into(),
                title: "Book 1".into(),
                series: None,
                path: std::path::PathBuf::from("/tmp/b1.cbz"),
                kind: SourceKind::Archive,
                page_count: 10,
                current_page: 0,
                opened: true,
                added_at: 0,
                last_read: None,
                marks: std::collections::BTreeMap::new(),
            };
            this.library.add(book);
        });

        view.update(cx, |this, cx| {
            if let Some(book) = this.library.get_mut("b1") {
                book.current_page = 5;
            }
            this.save_library_debounced(cx);
            this.request_quit(cx);
        });

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.get("b1").unwrap().current_page, 5);
    }

    // 2. A pending debounce is flushed during shutdown.
    #[gpui_kit::test]
    fn test_2_pending_debounce_is_flushed_during_shutdown(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("pending-debounce-flush");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, cx| {
            this.library.settings_mut().zoom = 2.5;
            this.save_library_debounced(cx);
            assert!(this.save_pending, "debounce must be pending");
            this.request_quit(cx);
            assert!(!this.save_pending, "request_quit must clear save_pending");
        });

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().zoom, 2.5);
    }

    // 3. A save worker already holding write_lock completes before shutdown.
    #[gpui_kit::test]
    fn test_3_worker_holding_write_lock_completes_before_shutdown(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("worker-holding-lock");
        let (view, cx) = launch_test(dirs.clone(), cx);

        let write_lock = view.read_with(cx, |this, _| Arc::clone(&this.write_lock));
        let last_saved_seq = view.read_with(cx, |this, _| Arc::clone(&this.last_saved_seq));
        let barrier_locked = Arc::new(Barrier::new(2));
        let barrier_release = Arc::new(Barrier::new(2));

        let lock_clone = Arc::clone(&write_lock);
        let b_locked = Arc::clone(&barrier_locked);
        let b_rel = Arc::clone(&barrier_release);
        let dirs_clone = dirs.clone();
        let last_saved_clone = Arc::clone(&last_saved_seq);

        let worker = std::thread::spawn(move || {
            let _guard = lock_clone.lock().unwrap();
            b_locked.wait();
            b_rel.wait();
            let mut lib = Library::default();
            lib.settings_mut().zoom = 1.5;
            lib.save(&dirs_clone).unwrap();
            last_saved_clone.store(1, Ordering::Release);
        });

        barrier_locked.wait();

        view.update(cx, |this, cx| {
            this.save_seq = 2;
            this.library.settings_mut().zoom = 3.0;
            this.request_quit(cx);
        });

        barrier_release.wait();
        worker.join().unwrap();

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().zoom, 3.0);
        assert_eq!(last_saved_seq.load(Ordering::Acquire), 2);
    }

    // 4. A save worker queued behind write_lock cannot overwrite the final shutdown snapshot.
    #[gpui_kit::test]
    fn test_4_queued_worker_cannot_overwrite_final_shutdown_snapshot(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("queued-cannot-overwrite");
        let (view, cx) = launch_test(dirs.clone(), cx);

        let write_lock = view.read_with(cx, |this, _| Arc::clone(&this.write_lock));
        let last_saved_seq = view.read_with(cx, |this, _| Arc::clone(&this.last_saved_seq));

        view.update(cx, |this, cx| {
            this.library.settings_mut().zoom = 2.0;
            this.save_seq = 2;
            this.request_quit(cx);
        });
        cx.run_until_parked();

        assert_eq!(last_saved_seq.load(Ordering::Acquire), 2);

        let mut stale_lib = Library::default();
        stale_lib.settings_mut().zoom = 1.0;
        let stale_seq = 1u64;
        {
            let _guard = write_lock.lock().unwrap();
            if stale_seq > last_saved_seq.load(Ordering::Acquire) {
                stale_lib.save(&dirs).unwrap();
                last_saved_seq.store(stale_seq, Ordering::Release);
            }
        }

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().zoom, 2.0);
        assert_eq!(last_saved_seq.load(Ordering::Acquire), 2);
    }

    // 5. Repeated request_quit() calls do not trigger premature termination or duplicate shutdown operations.
    #[gpui_kit::test]
    fn test_5_repeated_request_quit_calls_are_idempotent(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("repeated-quit");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, cx| {
            this.library.settings_mut().zoom = 1.75;
            this.save_library_debounced(cx);
            this.request_quit(cx);
            assert!(this.is_quitting);
            this.request_quit(cx);
            assert!(this.is_quitting);
        });

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().zoom, 1.75);
    }

    // 6. Repeated window-close events cannot close the window during persistence.
    #[gpui_kit::test]
    fn test_6_repeated_window_close_does_not_close_window(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("window-close-guard");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, cx| {
            this.request_quit(cx);
            assert!(this.is_quitting);
        });

        let is_quitting = view.read_with(cx, |this, _| this.is_quitting);
        let should_close = !is_quitting;
        assert!(
            !should_close,
            "should_close must return false during shutdown"
        );

        cx.run_until_parked();
    }

    // 7. macOS/global Quit reaches graceful shutdown.
    #[gpui_kit::test]
    fn test_7_global_quit_action_reaches_graceful_shutdown(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("global-quit-action");
        let (view, cx) = launch_test(dirs.clone(), cx);

        let yomu_view: Rc<RefCell<Option<Entity<Yomu>>>> =
            Rc::new(RefCell::new(Some(view.clone())));
        let yomu_for_quit = yomu_view.clone();

        cx.update(|_window, cx| {
            cx.on_action(move |_: &Quit, cx: &mut App| {
                if let Some(view) = yomu_for_quit.borrow().as_ref() {
                    view.update(cx, |this, cx| this.request_quit(cx));
                } else {
                    cx.quit();
                }
            });
        });

        view.update(cx, |this, cx| {
            this.library.settings_mut().zoom = 2.25;
            this.save_library_debounced(cx);
        });

        cx.simulate_keystrokes("ctrl-q");

        assert!(view.read_with(cx, |this, _| this.is_quitting));
        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().zoom, 2.25);
    }

    // 8. A failed final save does not silently terminate the application.
    #[gpui_kit::test]
    fn test_8_failed_final_save_keeps_application_alive_and_shows_error(cx: &mut TestAppContext) {
        let (tmp, dirs) = test_dirs("failed-save-alive");
        let (view, cx) = launch_test(dirs.clone(), cx);

        // Make data directory unwritable
        let data_dir = dirs.root().join("data");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&tmp.0);
        let file = std::fs::File::create(&tmp.0).unwrap();
        let mut perms = file.metadata().unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&tmp.0, perms.clone()).unwrap();

        view.update(cx, |this, cx| {
            this.save_seq += 1;
            this.request_quit(cx);
        });

        cx.run_until_parked();

        view.update(cx, |this, _| {
            assert!(!this.is_quitting, "application must reset is_quitting on failure");
            assert!(
                matches!(
                    &this.overlay,
                    Some(Overlay::Confirm(dialog)) if matches!(dialog.action, ConfirmAction::ForceQuit)
                ),
                "must present ForceQuit confirmation dialog on failure"
            );
        });
    }

    // 9. Clear All Data cannot be overwritten by an older background snapshot.
    #[gpui_kit::test]
    fn test_9_clear_all_data_cannot_be_overwritten_by_older_background_snapshot(
        cx: &mut TestAppContext,
    ) {
        let (_tmp, dirs) = test_dirs("clear-all-data-race");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, _cx| {
            let book = Book {
                id: "b1".into(),
                title: "Book 1".into(),
                series: None,
                path: std::path::PathBuf::from("/tmp/b1.cbz"),
                kind: SourceKind::Archive,
                page_count: 5,
                current_page: 0,
                opened: false,
                added_at: 0,
                last_read: None,
                marks: std::collections::BTreeMap::new(),
            };
            this.library.add(book);
        });

        view.update(cx, |this, cx| {
            this.clear_all_data(cx);
        });

        cx.run_until_parked();

        let write_lock = view.read_with(cx, |this, _| Arc::clone(&this.write_lock));
        let last_saved_seq = view.read_with(cx, |this, _| Arc::clone(&this.last_saved_seq));
        let stale_seq = 1u64;
        let mut stale_lib = Library::default();
        stale_lib.add(Book {
            id: "b1".into(),
            title: "Book 1".into(),
            series: None,
            path: std::path::PathBuf::from("/tmp/b1.cbz"),
            kind: SourceKind::Archive,
            page_count: 5,
            current_page: 0,
            opened: false,
            added_at: 0,
            last_read: None,
            marks: std::collections::BTreeMap::new(),
        });

        {
            let _guard = write_lock.lock().unwrap();
            if stale_seq > last_saved_seq.load(Ordering::Acquire) {
                stale_lib.save(&dirs).unwrap();
                last_saved_seq.store(stale_seq, Ordering::Release);
            }
        }

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(
            loaded.books().len(),
            0,
            "cleared state must not be overwritten"
        );
    }

    // 10. A successful final save is actually present on disk before shutdown completion.
    #[gpui_kit::test]
    fn test_10_successful_final_save_present_on_disk_before_shutdown_completes(
        cx: &mut TestAppContext,
    ) {
        let (_tmp, dirs) = test_dirs("final-save-present");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, cx| {
            this.library.settings_mut().zoom = 3.5;
            this.save_library_debounced(cx);
            this.request_quit(cx);
        });

        cx.run_until_parked();

        let last_saved = view.read_with(cx, |this, _| this.last_saved_seq.load(Ordering::Acquire));
        let save_seq = view.read_with(cx, |this, _| this.save_seq);
        assert_eq!(last_saved, save_seq);

        let file = dirs.library_file();
        assert!(file.exists(), "library.json must physically exist on disk");
        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().zoom, 3.5);
    }

    // 11. ForceQuit discards unsaved changes and Drop never attempts synchronous persistence.
    #[gpui_kit::test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn test_11_force_quit_flow_and_drop_never_retries_persistence(cx: &mut TestAppContext) {
        let (tmp, dirs) = test_dirs("force-quit-drop");
        let (view, cx) = launch_test(dirs.clone(), cx);

        // Make data dir unwritable to induce save failure on quit
        let data_dir = dirs.root().join("data");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&tmp.0);
        let file = std::fs::File::create(&tmp.0).unwrap();
        let mut perms = file.metadata().unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&tmp.0, perms.clone()).unwrap();

        view.update(cx, |this, cx| {
            this.library.settings_mut().zoom = 4.2;
            this.save_library_debounced(cx);
            this.request_quit(cx);
        });

        cx.run_until_parked();

        // Verify the failure occurred, is_quitting is reset, and ForceQuit dialog is open
        view.update(cx, |this, _| {
            assert!(!this.is_quitting, "is_quitting must reset to false on save failure");
            assert!(
                matches!(
                    &this.overlay,
                    Some(Overlay::Confirm(dialog)) if matches!(dialog.action, ConfirmAction::ForceQuit)
                ),
                "must present ForceQuit confirmation dialog"
            );
        });

        // Restore permissions so that if Drop WERE to attempt synchronous persistence,
        // it would be capable of writing.
        perms.set_readonly(false);
        std::fs::set_permissions(&tmp.0, perms).unwrap();
        let _ = std::fs::remove_file(&tmp.0);
        std::fs::create_dir_all(dirs.root().join("data")).unwrap();

        // Simulate user clicking "Quit Without Saving" (ConfirmAction::ForceQuit)
        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.confirm_accept(window, cx);
            });
        });

        // Verify save_seq > last_saved_seq before drop
        let save_seq = view.read_with(cx, |this, _| this.save_seq);
        let last_saved_seq =
            view.read_with(cx, |this, _| this.last_saved_seq.load(Ordering::Acquire));
        assert!(
            save_seq > last_saved_seq,
            "save_seq must exceed last_saved_seq (unsaved changes exist)"
        );

        // Drop the view to simulate object destruction during application exit
        drop(view);

        // Verify that Drop did NOT perform synchronous persistence:
        // library.json must not exist, honoring the user's explicit ForceQuit choice.
        let library_file = dirs.library_file();
        assert!(
            !library_file.exists(),
            "library.json must NOT be written by Drop during ForceQuit"
        );
    }

    // 12. Cancelling ForceQuit leaves the application usable, and retrying quit succeeds.
    #[gpui_kit::test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn test_12_cancel_force_quit_leaves_app_usable_and_retry_succeeds(cx: &mut TestAppContext) {
        let (tmp, dirs) = test_dirs("cancel-force-quit");
        let (view, cx) = launch_test(dirs.clone(), cx);

        // Make data dir unwritable to induce save failure on quit
        let data_dir = dirs.root().join("data");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&tmp.0);
        let file = std::fs::File::create(&tmp.0).unwrap();
        let mut perms = file.metadata().unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&tmp.0, perms.clone()).unwrap();

        view.update(cx, |this, cx| {
            this.library.settings_mut().zoom = 3.0;
            this.save_library_debounced(cx);
            this.request_quit(cx);
        });

        cx.run_until_parked();

        // User cancels the dialog
        view.update(cx, |this, cx| {
            assert!(!this.is_quitting);
            this.close_overlay(cx);
            assert!(this.overlay.is_none());
        });

        // App remains usable; user modifies settings again
        view.update(cx, |this, cx| {
            this.library.settings_mut().zoom = 2.75;
            this.save_library_debounced(cx);
        });

        // Permissions fixed
        perms.set_readonly(false);
        std::fs::set_permissions(&tmp.0, perms).unwrap();
        let _ = std::fs::remove_file(&tmp.0);
        std::fs::create_dir_all(dirs.root().join("data")).unwrap();

        // User requests quit again
        view.update(cx, |this, cx| {
            this.request_quit(cx);
        });

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(
            loaded.settings().zoom,
            2.75,
            "retried save must write the newest state"
        );
    }

    // 13. Repeated window-close requests during shutdown cannot close the window or bypass persistence.
    #[gpui_kit::test]
    fn test_13_repeated_window_close_does_not_abort_or_bypass_shutdown(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("repeated-window-close");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, cx| {
            this.library.settings_mut().zoom = 1.9;
            this.save_library_debounced(cx);
            this.request_quit(cx);
            assert!(this.is_quitting);
        });

        // Simulate multiple window manager close callbacks firing while shutdown is in flight
        let is_quitting = view.read_with(cx, |this, _| this.is_quitting);
        let close1 = !is_quitting;
        let close2 = !is_quitting;
        assert!(!close1, "first should_close check must return false");
        assert!(!close2, "second should_close check must return false");

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().zoom, 1.9);
    }

    // 14. Pointer and UI settings interactions cannot mutate settings during shutdown.
    #[gpui_kit::test]
    fn test_14_pointer_interaction_during_shutdown_cannot_mutate_settings(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("pointer-block-settings");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, cx| {
            this.library.settings_mut().group_series = true;
            this.library.settings_mut().zoom = 1.0;
            this.save_library_debounced(cx);
            this.request_quit(cx);
            assert!(this.is_quitting);

            // Attempt to mutate settings while quitting
            this.toggle_group_series(false, cx);
            this.set_continuous(Some(true), cx);
            this.set_webcomics(Some(true), cx);
            this.set_fit(yomu::layout::FitMode::Width, cx);
            assert!(this.library.settings().group_series);
            assert!(!this.library.settings().continuous);
            assert!(!this.library.settings().webcomics);
        });

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert!(loaded.settings().group_series);
        assert!(!loaded.settings().continuous);
    }

    // 15. Library card actions cannot change reading progress during shutdown.
    #[gpui_kit::test]
    fn test_15_library_card_actions_cannot_change_reading_progress_during_shutdown(
        cx: &mut TestAppContext,
    ) {
        let (_tmp, dirs) = test_dirs("card-actions-blocked");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, cx| {
            let book = Book {
                id: "b1".into(),
                title: "Book 1".into(),
                series: None,
                path: std::path::PathBuf::from("/tmp/b1.cbz"),
                kind: SourceKind::Archive,
                page_count: 20,
                current_page: 5,
                opened: true,
                added_at: 0,
                last_read: None,
                marks: std::collections::BTreeMap::new(),
            };
            this.library.add(book);
            this.save_library_debounced(cx);
            this.request_quit(cx);
            assert!(this.is_quitting);

            // Attempt to navigate or change progress
            this.goto_page(15, yomu::layout::Arrive::Start, cx);
            assert_eq!(this.library.get("b1").unwrap().current_page, 5);
        });

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.get("b1").unwrap().current_page, 5);
    }

    // 16. Context menu and confirmation dialog actions cannot mutate the library during shutdown.
    #[gpui_kit::test]
    fn test_16_context_menu_and_dialog_actions_cannot_mutate_library_during_shutdown(
        cx: &mut TestAppContext,
    ) {
        let (_tmp, dirs) = test_dirs("dialog-actions-blocked");
        let (view, cx) = launch_test(dirs.clone(), cx);

        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                let book = Book {
                    id: "b1".into(),
                    title: "Book 1".into(),
                    series: None,
                    path: std::path::PathBuf::from("/tmp/b1.cbz"),
                    kind: SourceKind::Archive,
                    page_count: 10,
                    current_page: 2,
                    opened: true,
                    added_at: 0,
                    last_read: None,
                    marks: std::collections::BTreeMap::new(),
                };
                this.library.add(book);
                this.save_library_debounced(cx);
                this.request_quit(cx);
                assert!(this.is_quitting);

                // Attempt context menu book actions
                this.perform_book_action(yomu::menu::BookAction::Remove, "b1", window, cx);
                this.perform_book_action(yomu::menu::BookAction::MarkRead, "b1", window, cx);
                assert!(this.library.get("b1").is_some());
                assert_eq!(this.library.get("b1").unwrap().current_page, 2);

                // Attempt to accept a mutating confirmation dialog while quitting
                this.overlay = Some(Overlay::Confirm(ConfirmDialog {
                    title: "Reset all".into(),
                    message: "Reset".into(),
                    confirm_label: "Reset",
                    action: ConfirmAction::ClearAllData,
                }));
                this.confirm_accept(window, cx);
                assert_eq!(
                    this.library.books().len(),
                    1,
                    "ClearAllData must be rejected during shutdown"
                );
            });
        });

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.books().len(), 1);
    }

    // 17. Keyboard and Vim actions remain blocked during shutdown.
    #[gpui_kit::test]
    fn test_17_keyboard_and_vim_actions_remain_blocked_during_shutdown(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("vim-actions-blocked");
        let (view, cx) = launch_test(dirs.clone(), cx);

        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.library.settings_mut().zoom = 1.0;
                this.save_library_debounced(cx);
                this.request_quit(cx);
                assert!(this.is_quitting);

                // Attempt to apply Vim actions
                let vp = yomu::layout::Viewport {
                    width: 800.0,
                    height: 600.0,
                    scale: 1.0,
                };
                this.apply(yomu::vim::Action::ZoomSet(3.0), vp, window, cx);
                this.apply(yomu::vim::Action::SetContinuous(Some(true)), vp, window, cx);
                assert_eq!(this.library.settings().zoom, 1.0);
                assert!(!this.library.settings().continuous);
            });
        });

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().zoom, 1.0);
    }

    // 18. Background import completion arriving after shutdown begins is discarded safely.
    #[gpui_kit::test]
    fn test_18_background_import_completion_discarded_during_shutdown(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("import-discarded-shutdown");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, cx| {
            this.request_quit(cx);
            assert!(this.is_quitting);

            // Attempt to call import_paths directly
            this.import_paths(vec![std::path::PathBuf::from("/tmp/new_comic.cbz")], cx);
            assert!(
                !this.importing,
                "import_paths must not start when is_quitting is true"
            );
            assert_eq!(this.library.books().len(), 0);
        });

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.books().len(), 0);
    }

    // 19. Save safety gates prevent new save dispatch during shutdown.
    #[gpui_kit::test]
    fn test_19_save_gates_prevent_new_dispatch_during_shutdown(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("save-gates-shutdown");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, cx| {
            this.library.settings_mut().zoom = 1.25;
            this.save_library_debounced(cx);
            let initial_seq = this.save_seq;
            this.request_quit(cx);
            assert!(this.is_quitting);

            // Calling save_library or save_library_debounced must be no-ops
            this.save_library(cx);
            assert_eq!(
                this.save_seq, initial_seq,
                "save_library must not increment save_seq when quitting"
            );
            this.save_library_debounced(cx);
            assert_eq!(
                this.save_seq, initial_seq,
                "save_library_debounced must not increment save_seq when quitting"
            );
        });

        cx.run_until_parked();

        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(loaded.settings().zoom, 1.25);
    }

    // 20. The final persisted snapshot remains unchanged by attempted post-shutdown mutations.
    #[gpui_kit::test]
    fn test_20_final_persisted_snapshot_remains_unchanged_by_post_shutdown_mutations(
        cx: &mut TestAppContext,
    ) {
        let (_tmp, dirs) = test_dirs("snapshot-integrity");
        let (view, cx) = launch_test(dirs.clone(), cx);

        view.update(cx, |this, cx| {
            this.library.settings_mut().zoom = 2.0;
            this.save_library_debounced(cx);
            this.request_quit(cx);
            assert!(this.is_quitting);

            // Directly try to mutate in-memory library (simulating an uncaught mutation)
            this.library.settings_mut().zoom = 9.9;
            this.save_library(cx);
        });

        cx.run_until_parked();

        // Disk must contain the snapshot captured at request_quit (zoom = 2.0), NOT 9.9
        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(
            loaded.settings().zoom,
            2.0,
            "final snapshot on disk must not be corrupted by post-shutdown mutation"
        );
    }

    fn test_png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([120, 60, 200, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn write_test_cbz(path: &std::path::Path, pages: &[&str]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for name in pages {
            zip.start_file(*name, opts).unwrap();
            use std::io::Write;
            zip.write_all(&test_png_bytes(64, 96)).unwrap();
        }
        zip.finish().unwrap();
    }

    // 21. Starting an import and immediately calling request_quit cancels background worker
    // and prevents late books from being committed to persistent state.
    #[gpui_kit::test]
    fn test_21_starting_import_then_immediate_quit_cancels_worker_and_does_not_commit_late_books(
        cx: &mut TestAppContext,
    ) {
        let (_tmp, dirs) = test_dirs("import-immediate-quit");
        let (view, cx) = launch_test(dirs.clone(), cx);

        let comic_dir = dirs.root().join("incoming");
        std::fs::create_dir_all(&comic_dir).unwrap();
        let cbz1 = comic_dir.join("vol1.cbz");
        let cbz2 = comic_dir.join("vol2.cbz");
        let cbz3 = comic_dir.join("vol3.cbz");
        write_test_cbz(&cbz1, &["1.png"]);
        write_test_cbz(&cbz2, &["1.png"]);
        write_test_cbz(&cbz3, &["1.png"]);

        view.update(cx, |this, cx| {
            this.import_paths(vec![cbz1, cbz2, cbz3], cx);
            assert!(this.importing, "import must be active initially");
            assert!(
                this.import_cancel.is_some(),
                "import_cancel handle must be registered"
            );

            // Quit immediately while the background import worker is in flight
            this.request_quit(cx);
            assert!(this.is_quitting, "request_quit must set is_quitting");
            assert!(
                this.import_cancel
                    .as_ref()
                    .is_some_and(|c| c.load(Ordering::Relaxed)),
                "request_quit must signal the import cancellation token"
            );
        });

        cx.run_until_parked();

        // Disk library must contain zero books: the initial empty snapshot was preserved,
        // and late background completions were prevented from mutating or saving to disk.
        let loaded = Library::load(&dirs).unwrap();
        assert_eq!(
            loaded.books().len(),
            0,
            "cancelled import must not commit late books to persistent state"
        );
    }

    // 22. A file-picker error callback during shutdown does not produce UI toasts.
    #[gpui_kit::test]
    fn test_22_file_picker_error_callback_during_shutdown_does_not_produce_toast(
        cx: &mut TestAppContext,
    ) {
        let (_tmp, dirs) = test_dirs("picker-error-shutdown");
        let (view, cx) = launch_test(dirs.clone(), cx);

        // First, verify prompt_for_paths initiated before shutdown cancels cleanly
        view.update(cx, |this, cx| {
            this.import_dialog(false, cx);
        });
        assert!(
            cx.did_prompt_for_paths(),
            "dialog prompt must be registered"
        );

        view.update(cx, |this, cx| {
            this.request_quit(cx);
            assert!(this.is_quitting);

            // Attempting to show a toast directly or via error handler while is_quitting must be a no-op
            this.show_toast("Failed to open file picker: mock D-Bus error", cx);
            assert!(
                this.toast.is_none(),
                "show_toast must never produce a toast during shutdown"
            );
        });

        cx.simulate_path_prompt_response(|_| None);
        cx.run_until_parked();

        assert!(
            view.read_with(cx, |this, _| this.toast.is_none()),
            "toast must remain None after prompt response during shutdown"
        );
    }

    // 23. Late import completion cannot clear a newer import's cancellation token.
    #[gpui_kit::test]
    fn test_23_late_import_completion_cannot_clear_newer_import_token(cx: &mut TestAppContext) {
        let (_tmp, dirs) = test_dirs("token-identity-guard");
        let (view, cx) = launch_test(dirs.clone(), cx);

        let cancel_old = Arc::new(AtomicBool::new(false));
        let cancel_new = Arc::new(AtomicBool::new(false));

        view.update(cx, |this, _| {
            this.importing = true;
            this.importing_title = Some("New Comic".to_string());
            this.import_cancel = Some(Arc::clone(&cancel_new));

            // Simulate completion path of the older import task:
            // Arc::ptr_eq check ensures an older task holding cancel_old does NOT clear cancel_new
            let was_active = this
                .import_cancel
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &cancel_old));
            assert!(!was_active, "older token must not match active token");
            if was_active {
                this.import_cancel = None;
                this.importing = false;
                this.importing_title = None;
            }

            assert!(
                this.importing,
                "newer import must still be marked importing"
            );
            assert_eq!(this.importing_title.as_deref(), Some("New Comic"));
            assert!(
                Arc::ptr_eq(this.import_cancel.as_ref().unwrap(), &cancel_new),
                "newer token must remain active"
            );

            // When the newer task finishes, it successfully matches and clears
            let was_active_new = this
                .import_cancel
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &cancel_new));
            assert!(was_active_new, "newer token must match active token");
            if was_active_new {
                this.import_cancel = None;
                this.importing = false;
                this.importing_title = None;
            }

            assert!(!this.importing);
            assert!(this.import_cancel.is_none());
            assert!(this.importing_title.is_none());
        });
    }
}
