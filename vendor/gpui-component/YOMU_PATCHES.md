# Yomu patches to `gpui-component` 0.9.0

The source in this directory is based on the official crates.io `gpui-component` 0.9.0
archive. A read-only recursive comparison against that archive found only the two source
files listed below differ. The path patch is declared in the root `Cargo.toml`.

## Required local additions

### `src/scroll/scrollable.rs`

- `Scrollable::track_scroll(&ScrollHandle)` lets the caller use the same handle for the
  scrollable viewport, its `Scrollbar`, and Yomu's programmatic scroll/momentum controller.
  Upstream 0.9.0 always obtains a handle from the scrollable element ID and has no public
  external-handle setter.
- `Scrollable::scroll_wheel(bool)` forwards to `ScrollBounce::scroll_wheel`. Yomu sets it
  to `false` in its library, details, continue-reading, and settings scroll views so the
  Linux/Windows `PlatformScrollController` receives the raw scroll events. Upstream 0.9.0
  does not expose this control on `Scrollable`.

Do not drop either method until an upstream public API preserves shared-handle ownership
and wheel-event propagation. Verify scrollbar dragging, programmatic scrolling, and
Linux/Windows touchpad behavior before changing these call sites.

## Upgrade comparison

1. Fetch the exact upstream crate archive for the target version.
2. Extract it outside the repository and run `diff -qr <upstream> vendor/gpui-component`.
3. Review every changed file and keep application-specific changes limited to the required
   hunks above. Recheck the callers listed in `src/app/views/library.rs` and
   `src/ui/{continue_reading,details,settings_view}.rs`.
4. Run the menu, scroll, reader, and settings UI tests plus the project validation matrix.

The former explicit `Selectable` re-export in `src/lib.rs` was removed during this audit:
upstream's `pub use styled::*` already publicly re-exports `Selectable`. The former
`PopupMenu` debug-selector removal was tested and rejected: the existing menu test
`right_click_opens_the_menu_and_never_opens_the_book` failed without it because GPUI's
`debug_bounds` query does not use the element ID as a debug selector. The selector remains
required and local.
