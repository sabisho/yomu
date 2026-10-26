<div align="center">

<img src="assets/yomu-banner.png" alt="Yomu — a minimal manga reader" width="300" />

# yomu

**A minimal, keyboard-first manga reader built entirely in Rust.**

GPU-accelerated rendering with [GPUI Kit](https://gpui-kit.com/) and direct Wayland integration — no webview, Tauri, GTK, or Qt.

[![Rust](https://img.shields.io/badge/Rust-1.92%2B-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Wayland-informational?logo=linux&logoColor=white)](https://wayland.freedesktop.org/)
[![UI](https://img.shields.io/badge/UI-GPUI%20Kit-7c3aed)](https://gpui-kit.com/)
[![Build](https://img.shields.io/badge/Build-Cargo%20%7C%20Nix-5277C3?logo=nixos&logoColor=white)](https://nixos.org/)

</div>

---

## ✨ Highlights

- 📚 **Personal library** — cover grid, reading-progress bars, centered search, and directory/series grouping.
- 📂 **Flexible imports** — open CBZ/CBR/ZIP/RAR archives or image folders; add files by browsing or drag and drop.
- 🖼️ **Distraction-free reader** — clean single-page view with a slim progress line and controls that stay out of the way.
- 🖱️ **Fast library actions** — context menus, multi-select, bulk removal, read/unread status, and progress reset.
- 🔎 **Powerful navigation** — Vim-style motions, counts, marks, a command prompt, and familiar keyboard shortcuts.
- 🔍 **Flexible viewing** — fit page/width/height, original size, zoom, drag-to-pan, continuous scrolling, and gapless webcomics mode.
- 🌀 **Smooth input** — kinetic touchpad/touchscreen scrolling, pinch-to-zoom at gesture anchor, and frame-rate-independent page transitions.
- 💾 **Persistent progress** — page position and marks are saved and restored when you return to a book.
- ⚡ **Responsive decoding** — pages are decoded and scaled on background threads, with nearby pages prefetched.
- 🎨 **Personalized appearance** — built-in and custom themes, opacity controls, and reduced-motion support.

## 📑 Contents

- [Build and run](#build-and-run-nixos)
- [Install on NixOS](#installing-yomu-on-nixos)
- [Keyboard shortcuts](#keys)
- [Selecting several books](#selecting-several-books)
- [Searching and grouping](#searching-and-grouping-the-library)
- [Context menu](#context-menu)
- [Settings and themes](#settings)
- [Performance](#page-turns-scrolling-and-performance)
- [Data storage](#where-data-lives)
- [Dependencies and development](#dependencies)
- [Roadmap / not done yet](#not-done-yet)

---

## Build and run (NixOS)

With [devenv](https://devenv.sh) (`devenv.nix` and `devenv.yaml` are in the repo):

```sh
devenv shell               # Rust toolchain + every native library gpui needs
cargo run --release
```

- `devenv test` runs the whole test suite; with direnv, `direnv allow` loads the shell automatically
  (the `.envrc` is already there).
- `nix develop` still works if you prefer it (same toolchain, same libraries), and `nix run .` builds and runs
  the packaged, wrapped binary. Flakes only see files that git tracks, so run `git init && git add .` first.

NixOS prerequisites, if you do not have them already:

```nix
hardware.graphics.enable = true;          # Vulkan drivers (/run/opengl-driver)
xdg.portal.enable = true;                 # the Add files / Add folder dialogs use the portal
xdg.portal.extraPortals = [ pkgs.xdg-desktop-portal-gtk ];   # or the GNOME portal
```

## Installing yomu on NixOS

The package definition is `nix/package.nix`; the flake exposes it as `packages.<system>.default` and as an overlay.
It is built with nixpkgs' own Rust (the project needs 1.92 or newer; use a current nixos-unstable),
wraps the binary so it finds Wayland and Vulkan, and installs a desktop entry.

**With a flake-based system configuration (recommended).** Add yomu as an input of your system flake:

```nix
inputs.yomu = {
  url = "git+file:///home/you/Projects/yomu";   # or github:you/yomu once it is pushed
  inputs.nixpkgs.follows = "nixpkgs";
};
```

then, in one of your NixOS modules (`inputs` must be passed through `specialArgs`):

```nix
{ inputs, pkgs, ... }: {
  nixpkgs.overlays = [ inputs.yomu.overlays.default ];
  environment.systemPackages = [ pkgs.yomu ];      # or home.packages with Home Manager
}
```

Without the overlay, use the package directly:
`environment.systemPackages = [ inputs.yomu.packages.${pkgs.stdenv.hostPlatform.system}.default ];`

Commit (or at least `git add`) your changes in the yomu repository, then `nix flake update yomu` in the system flake
and rebuild. Try it first without installing: `nix run git+file:///home/you/Projects/yomu`.

**Without flakes**, point `callPackage` at the file (an absolute path, so this does not work in a pure flake
evaluation):

```nix
environment.systemPackages = [ (pkgs.callPackage /home/you/Projects/yomu/nix/package.nix { }) ];
```

## Keys

Vim keybindings (motions like `j`/`k`/`gg`/`G`, marks, counts, and `:`) are opt-in via Settings → Reader or the `--vim` / `--vim-mode` command-line flag. When Vim mode is off (the default), standard desktop navigation shortcuts (arrows, PageUp/PageDown, Space, Home/End, `+`/`-`, Ctrl+wheel) operate cleanly without interception. When Vim mode is enabled, Vim motions take precedence for reading actions while preserving standard text editing in input fields. Press `?` in the reader for a cheat sheet.

**Library**

| Key | Action |
|-----|--------|
| Click a book | Open it where you left off |
| Right-click a book | Context menu (see below) |
| `Ctrl`+click, `Shift`+click | Select books (see below) |
| `Ctrl+A`, `Delete`, `Esc` | Select all, remove the selection, clear the selection |
| `Ctrl+O` / `Ctrl+Shift+O` | Add files / Add folder |
| `/`, click the search field, or `Ctrl+F` | Search titles, source paths, and series |
| `Ctrl+,`, `:settings`, or library hamburger → **Settings** | Settings |

**Moving**

| Key | Action |
|-----|--------|
| `j` `k` `h` `l`, arrows | Scroll. At the edge of the page they turn the page, so on a page that fits the window they simply turn pages |
| `Space` / `Shift+Space`, `Ctrl+F` / `Ctrl+B` | Scroll a screen down / up |
| `Ctrl+D` / `Ctrl+U` | Scroll half a screen |
| `J` `K`, `PageDown` `PageUp` | Next / previous page |
| `gg`, `G`, `12G`, `:12`, `Home`, `End` | First page, last page, page 12 |
| `ma`, `'a`, `''` | Set mark `a`, jump to it, jump back to where you were |
| Click left / right third of the window | Turn the page toward that side |
| Mouse wheel / Touchpad | Scroll; touchpad two-finger flicks launch natural inertial momentum scrolling |
| Touchpad / Touchscreen Pinch | Smoothly zoom in or out anchored at the pointer/pinch focal center |
| Drag | Pan a zoomed page or continuous strip; drag flicks launch inertial momentum |

Counts work on motions: `5j` scrolls five steps, `3J` skips three pages. Marks are saved with the book.

**View**

| Key | Action |
|-----|--------|
| `a` or `zp` | Fit page (the whole page is visible) |
| `s` or `zw` | Fit width |
| `zh` | Fit height |
| `z1` | Original size (one image pixel per screen pixel) |
| `Tab` | Cycle the fit modes |
| `+` / `=`, `zi` | Zoom in |
| `-`, `zo` | Zoom out |
| `0`, `z0` | Reset zoom to the fit size |
| `Ctrl` + wheel, Pinch | Zoom at the pointer / pinch center |
| `zt` `zz` `zb` | Scroll to the top / middle / bottom of the page |
| `v` | Toggle continuous vertical scrolling |
| `w` | Toggle gapless webcomics mode (forces continuous scrolling) |
| `i` | Show page number, fit mode and zoom |
| `f`, `F11` | Fullscreen |
| `?` | Help |
| `Esc`, `q` | Back to the library (`Esc` first cancels a half-typed command) |
| `Ctrl+Q` | Quit |

**Command prompt** (press `:` in the library or reader)

The floating prompt shows matching commands and short descriptions. Use `↑`/`↓` to select, `Tab` or
`Shift+Tab` to complete, `Enter` to run the draft, and `Esc` to cancel. Unknown commands remain editable
and show the parser's error message when run.

| Command | Action |
|---------|--------|
| `:12` | Go to page 12 |
| `:fit width`, `:fit page`, `:fit height`, `:fit original` | Set the fit mode |
| `:zoom 150` | Zoom to 150% of the fit size (`:zoom` resets) |
| `:continuous on`, `:continuous off` | Stack pages vertically or return to single-page reading |
| `:webcomics on`, `:webcomics off` | Force gapless pages or restore the normal reading mode |
| `:remember-zoom on`, `:remember-zoom off` | Keep custom zoom and pinch scale across all books |
| `:theme [name]` | Set color theme by name, or cycle to the next theme |
| `:q` | Back to the library |
| `:qa` | Quit |

By default pages run left to right: `Right` and `l` go forward, `Left` and `h` go back. The fit mode and
animation preference are remembered between books.

## Selecting several books

- **Ctrl+click** toggles a book. **Shift+click** selects the range from the last book you clicked.
- Once anything is selected, plain clicks toggle books instead of opening them, so you can just keep clicking.
  Deselect the last one (or press `Esc` / **Cancel**) and clicking opens books again.
- While books are selected the toolbar shows **Select all**, **Remove** and **Cancel** at the left.
  `Ctrl+A` selects everything and `Delete` removes the selection.
- Right-click a selected book for a bulk menu (**Clear Selection**, **Set Series for Selected Books…**, **Remove N Books from Library**).
  Right-clicking a book that is not selected drops the selection and shows that book's normal menu.
- Removal only forgets the books; the comic files are never deleted. It asks first only if some of the selected
  books have progress or marks that would be lost, and it saves once for the whole batch.

## Searching and grouping the library

The library toolbar search is centered and has no placeholder text; its caret blinks while active. Click the field,
press `/`, or press `Ctrl+F`, then type. Search matches a case-insensitive
substring in the book title, source path, or assigned series. The filter is temporary and does not change library data;
selection and bulk actions continue to refer to the full library. Press `Enter` to stop typing while keeping the
filter, or `Esc` to clear the query (press `Esc` again to leave search).

Directory grouping is on by default. Volumes from the same source directory appear as a folder card using the
first volume's cover; click it to open a second view of its books. The hamburger menu or **Settings → Library**
can switch to individual volumes. Folder grouping is saved in `library.json`. Right-click a book and choose
**Set Series…** / **Change Series…** to assign it to a named group, or select several books and choose
**Set Series for Selected Books…**. A manual series assignment takes precedence over the source directory;
clearing it returns the book to its directory group. Series names are trimmed, repeated whitespace is collapsed,
and names that differ only by capitalization share one folder.

Imports finish one volume at a time in the background. Each completed volume is added to the library and saved
immediately, so its title and cover appear while the remaining folder is still being scanned or decoded. New
books and folder cards retain scan/import order.

## Context menu

Right-click a book. Only the actions that make sense for its state are shown:

| Book state | Menu |
|------------|------|
| Not started | Start Reading · Mark as Read · Open File Location · Properties · Set Series… · Remove from Library |
| In progress | Resume Reading · Start from Beginning · Mark as Read · Reset Progress… · Open File Location · Properties · Set Series… · Remove from Library |
| Finished | Start from Beginning · Mark as Unread · Open File Location · Properties · Set Series… · Remove from Library |

- Navigate with `Up`/`Down` (or `j`/`k`, `Tab`), `Home`/`End`, `Enter` to run, `Esc` to close. Clicking outside closes it
  without clicking the book underneath. The menu slides back inside the window near the edges.
- **Remove from Library** never touches the comic file; it only forgets the book, its progress and its marks.
  It asks first only if there is progress or marks to lose. **Reset Progress** always asks (marks are kept).
- If the file has gone missing, the actions that need it are greyed out; Mark as Read/Unread, Properties and
  Remove still work.
- **Open File Location** uses the desktop's file manager through `xdg-desktop-portal`, falling back to `xdg-open`
  on the containing folder. It needs the same portal setup as the file dialogs.
- Dates in **Properties** are UTC.

## Settings

`Ctrl+,` (or `:settings` in the reader, or **Settings** in the library hamburger menu) opens a tabbed panel:

- **Theme**, cycle through the built-in GPUI Kit themes with the arrows. The app applies the theme immediately,
  uses its semantic colors throughout the library, reader and dialogs, and remembers it between launches.
- **Custom themes**, edit `$XDG_CONFIG_HOME/yomu/themes.toml` (usually `~/.config/yomu/themes.toml`) and choose
  **Reload custom themes**. On first launch yomu creates this file from [themes.example.toml](themes.example.toml).
  Changes to the file are detected automatically; the reload button remains available. A parse or validation error
  is shown in Settings and leaves the currently applied theme in place.
- **Noctalia colors**, install [noctalia/yomu-theme.toml.template](noctalia/yomu-theme.toml.template) as a Noctalia user template,
  with its output set to `$XDG_CONFIG_HOME/yomu/themes.toml`. The generated **Noctalia Dynamic** theme maps Noctalia's
  primary, surface, text, outline, and error roles to yomu's controls, icons, progress indicators, and surfaces. Select
  it in yomu Settings; yomu applies Noctalia's regenerated file automatically.
- **Reading mode**, single page or continuous vertical scrolling. The choice is remembered between launches;
  use `v`, the Settings panel, or `:continuous [on|off]` in the reader. In continuous mode, `j`/`k`, wheel and touchpad
  scroll through stacked pages, while `J`/`K` and page-number commands jump between pages.
- **Webcomics mode** shows pages as one connected vertical strip with no gap between adjacent pages. Toggle it
  with `w`, the Settings panel, or `:webcomics [on|off]`. It forces continuous scrolling while active, then restores
  the saved single-page or continuous reading mode when turned off.
- **Library** switches between automatic folder grouping and individual volumes.
- **Reader** controls page fit, animations, single-page or continuous scrolling, and webcomics mode.
- **Keyboard & Commands** displays a structured, scrollable table of all reader shortcuts and `:` commands with key badges and concise action descriptions.
- **Reset to Defaults** asks first and restores all preferences; it is greyed out when everything is already at the defaults.

Stored preferences are read leniently: a missing or invalid value falls back to its default (out-of-range numbers
are clamped) instead of affecting your library.

The custom theme file uses `[[themes]]` entries with `name`, `mode = "dark" | "light"`, and a `[themes.colors]`
table. Color keys use GPUI Kit theme names and accept `#RRGGBB` or `#RRGGBBAA` values. Supported options also include
`font_size`, `font_family`, `mono_font_size`, `mono_font_family`, `radius`, `radius_lg`, and `shadow`. The starter
file lists the supported color tokens and shows their TOML quoting for dotted keys.

For Noctalia v5+, add this to `~/.config/noctalia/templates.toml` (adjust the input path if you store the template elsewhere):

```toml
[theme.templates.user.yomu]
input_path = "$XDG_CONFIG_HOME/yomu/noctalia-theme.toml.template"
output_path = "$XDG_CONFIG_HOME/yomu/themes.toml"
```

Copy the repository template to that input path. Noctalia renders the template on palette changes using its active
`mode` and color roles; yomu reads the generated TOML when you reload custom themes. See [Noctalia's app theming guide](https://docs.noctalia.dev/noctalia/theming/app-theming/)
and [template reference](https://docs.noctalia.dev/noctalia/theming/templates/) for user-template setup and available palette tokens.

### How opacity works, and its limits

yomu asks the compositor for a window with an alpha channel. The window background follows the opacity slider,
and raised widget surfaces (cards, fields, dialogs, buttons, and the reader command/suggestion panel) use 10 percentage
points more opacity so they remain distinct panes over the background, capped at 100%. Text and comic images stay
fully opaque. At exactly 100% the window switches back to a fully opaque surface, which lets the compositor skip
blending.

- Needs a compositing window system. Every Wayland compositor qualifies; on X11 you need a compositor running.
- Blur is supplied by the compositor, not yomu. The reader command and suggestion panel is drawn inside yomu's main
  window and follows the same 10-point raised-surface opacity rule as Settings. The book context menu is a native
  `xdg_popup`. Niri needs a window effect for the command panel and a popup effect for the context menu; the popup
  setting alone does not blur in-window panels. Add both rules:

  ```kdl
  window-rule {
      match app-id="^yomu$"
      background-effect {
          blur true
      }
      popups {
          background-effect {
              blur true
          }
      }
  }
  ```
- I could not look at a real compositor while building this, so the actual translucent look (including the colour
  of semi-transparent areas) is the one thing not verified by the tests.

## Page turns, scrolling and performance

- **Page turn slide.** Turning a page slides the old page out and the new one in along the reading direction:
  forward moves the content left in a left-to-right book and right in a right-to-left one, backward the reverse.
  Both pages move by one window width, so the change is visible even when zoomed in. It takes 160 ms
  (90 ms when it interrupts a running slide, e.g. holding a key). There is never a queue: a turn that arrives
  mid-slide replaces the running one, and the page that was sliding in continues from exactly where it is.
  Your position (and the saved progress) change the instant you turn; the slide is only a visual.
- **No slide** when opening a comic, switching comics, resizing the window, zooming or changing the fit mode, and
  none while the next page is still decoding (the old page stays on screen instead of a blank).
- **Smooth scrolling.** `j`/`k`/`h`/`l`, Space and the mouse wheel glide to their target; repeated presses
  accumulate. Touchpads follow your fingers one to one.
- **Reduced motion.** `:animate off` (or `:animate` to toggle) turns the slide and the glide off and is
  remembered. They are also off when `YOMU_REDUCE_MOTION=1` is set or the GNOME "Animations" switch is off.
- **Frame pacing.** Animations are driven by the time between frames, never a frame count, so they take the same
  time at 60, 144 or 240 Hz, and a frame is only requested while something is moving; an idle window draws nothing.
  The compositor's frame callbacks set the pace.
- **Memory.** At most the current page, three ahead and two behind stay decoded (one behind and three ahead are
  prefetched), under a 320 MB cap; textures are freed as soon as they are no longer drawn. Pages are decoded and
  scaled on background threads at the resolution the current zoom needs, with a SIMD resampler.

Measured here (a slow single-core VM with no GPU, so only the CPU side): a 3000x4500 JPEG decodes and scales in
300 to 500 ms at every resolution tier (the old resampler needed about 1.3 s at the 1536 to 2048 pixel tiers and
about 2 s at the 3072 to 4096 pixel tiers), and a frame of the slide costs 0.26 ms of CPU on average (95th
percentile 0.38 ms, unoptimised build) against a 6.9 ms budget at 144 Hz. `cargo test perf_ -- --ignored --nocapture`
reproduces the numbers.

## Where data lives

`$XDG_DATA_HOME/yomu/` (usually `~/.local/share/yomu/`), or `$YOMU_DATA_DIR` if set:

- `library.json` holds the books and reading progress
- `covers/` holds generated cover thumbnails

Importing never copies or modifies your comic files.

## Dependencies

The UI uses GPUI Kit 0.7.3, which brings its matching GPUI snapshot and requires Rust 1.92 or newer. The old
`vendor/xattr` and `vendor/stacksafe-macro` compatibility copies are retained in the repository, but are no longer
patched into the active dependency graph.

## Development

For coding-agent instructions, architecture, persistence invariants, and change recipes, see
[AGENTS.md](AGENTS.md), [docs/architecture.md](docs/architecture.md),
[docs/development.md](docs/development.md), [docs/ui-redesign-plan.md](docs/ui-redesign-plan.md),
and [ROADMAP.md](ROADMAP.md).

```sh
cargo test       # core logic plus UI tests that drive the real views with simulated mouse and keyboard input
```

The code is split so that everything except the window lives in the library crate:

- `src/archive.rs` lists and reads pages from CBZ, CBR, ZIP, and RAR archives and image folders
- `src/imaging.rs` decodes and resizes pages, and builds cover thumbnails
- `src/layout.rs` is the fit-mode, zoom and pan math
- `src/vim.rs` turns keys into actions: counts, `gg`/`zw`/`ma` sequences and the `:` command line
- `src/menu.rs` decides which context-menu actions apply to a book, plus confirmations and the Properties rows
- `src/selection.rs` holds the multi-select rules (toggle, range, select all)
- `nix/package.nix` is the Nix package; `devenv.nix` / `devenv.yaml` and `flake.nix` are the dev environments
- `src/library.rs` is the library model, JSON persistence, and the import pipeline
- `src/main.rs` is the GPUI Kit UI

## Not done yet

- CB7 (needs a 7z archive reader)
- Double-page spreads
- Sorting options
- A clear Noctalia Launcher-style hover and active-selection color across menus and settings, including pointer hover
  and keyboard navigation. Use the theme's primary/accent color so the current item is easy to identify.
- Inertial scrolling (very low priority roadmap item)
