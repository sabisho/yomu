# Application Orchestration — Agent Guide (`src/app/` & `src/main.rs`)

This guide supplements the root [AGENTS.md](../../AGENTS.md) with specific instructions for the **Application Orchestration Layer** (`src/app/` and `src/main.rs`).

---

## 1. Architectural Boundary & Invariants

The application orchestration layer constitutes Tier 3 of Yomu's three-tier architecture. It coordinates application lifecycle, state routing, async background scheduling, and input dispatch:

- **State Ownership:** Root application state is centralized in `struct Yomu` (`src/app/state.rs`) and shared across orchestration submodules (`state.rs`, `reader_controller.rs`, `keybindings.rs`, `persistence.rs`, `actions.rs`, `desktop.rs`, `views/`).
- **Separation from Pure Views:** Orchestration logic coordinates state transitions and builds `Props` to render views in `src/ui/`. Presentation rendering must never mutate application state directly.
- **Asynchronous Scheduling:** All CPU-intensive operations (archive scanning, image decoding, SIMD scaling, disk writes) must be offloaded to `cx.background_executor()`.

---

## 2. Subsystem Invariants & Concurrency Rules

### 1. Persistence & Concurrency Coordination (`app/persistence.rs`)
- **Non-Blocking UI Dispatch:** All UI-triggered saves must use asynchronous background dispatch (`save_library` for discrete actions, `save_library_debounced` with a 500ms trailing timer for page turns and scrolling).
- **Zero UI-Thread Mutex Contention:** Never block the UI thread waiting on persistence mutexes. Use lock-free monotonic sequencing (`last_saved_seq: Arc<AtomicU64>`) and serialize disk writes on the background executor via `write_lock: Arc<Mutex<()>>`.
- **4-Tier Graceful Shutdown Isolation:** Window close and quit actions must route through `request_quit(cx)`. Shutdown enforces:
  1. Full-window native GPUI pointer occlusion barrier (`HitboxBehavior::BlockMouse`).
  2. Root action guards blocking keyboard and navigation input during flush.
  3. Domain mutation gates rejecting post-shutdown background completions.
  4. Non-blocking zero-I/O `Drop for Yomu` with error recovery dialog and `ForceQuit` fallback.
- **Cooperative Import Cancellation:** Long-running background imports pass an `Arc<AtomicBool>` cancellation token checkpointed across directory traversal, archive reading, and cover generation. Aborted imports must never commit late books or display UI toasts after shutdown.

### 2. Reader Lifecycle & Decode Queue (`app/reader_controller.rs`)
- **Prefetch Task Cancellation Safety:** Rapid page turns cancel outdated prefetch tasks by replacing the cancellation token (`cancel_token: Arc<AtomicBool>`). The decode callback (`page_decoded`) verifies token identity via `Arc::ptr_eq`.
- **Never Mark Aborted Tasks as Failed:** A cancelled prefetch task is an expected lifecycle event, not an error. Aborted tasks must **never** be inserted into `reader.failed`. Cancelled pages must remain immediately requestable when the user navigates back.
- **Eviction Synchronization:** Maintain adaptive memory budget bounds via `sync_reader`. Recycle entries through `reader.entries_scratch` to eliminate heap allocations per frame.

### 3. Keybindings & Action Dispatch (`app/keybindings.rs` & `app/actions.rs`)
- **Modal & Overlay Precedence:** Active modal overlays (dialogs, help sheet, settings) intercept keyboard events before reader or library shortcuts.
- **Vim Mode Delegation:** When Vim mode is enabled, reading motions delegate parsing to `src/vim.rs` (`vim::Vim::feed`), executing the returned `Action` enum via `Yomu::apply`.
- **Context Menus & Confirmations:** Context menu actions delegate to `app/actions.rs`. Destructive actions (book removal with progress or marks) must prompt for confirmation, while safe removals proceed cleanly.

### 4. GPU Texture Garbage Collection (`src/main.rs`)
- Texture buffers dropped during page turns or book closes are enqueued into `Yomu.garbage`.
- The root `Render::render` implementation must drain `garbage` and explicitly release GPU textures via `window.drop_image(image)` before drawing the frame.
