# Yomu — Desktop UI/UX Design System Specification

## 1. Executive Summary & Design Principles

This specification defines the desktop visual identity, design tokens, spatial metrics, and component behaviors for **Yomu**, an editorial, keyboard-first desktop manga and comic reader built in safe Rust 2024 on **GPUI Kit 0.8.1** (`gpui-pre = 0.3.10` runtime) under `#![forbid(unsafe_code)]`.

### Core Design Philosophy
1. **Editorial & Understated:** Warm, low-fatigue Gruvbox aesthetic tailored for extended reading sessions under both low and bright ambient lighting.
2. **Keyboard-First & High-Density:** Respects Vim navigation paradigms (`h/j/k/l`, `g/G`, `:spread`, `:single`, `/`), compact information density, and rapid scanning without unnecessary decorative bloat.
3. **Zero Distraction Reader:** The reading viewport gives 100% priority to high-resolution comic artwork. Chrome, toolbars, and lateral page turners remain unobtrusive or autohiding.
4. **Native Desktop Rigor:** Built exclusively with native desktop conventions (persistent 240px sidebar, context-aware 52px toolbar, virtualized lists, native modal dialogs, platform-aware window controls).
5. **Verified Vector Icons:** Strict use of clean, resolution-independent vector icons mapped to official `gpui_kit::assets::IconName` (Lucide icon set). Zero platform emoji tofu or character encoding artifacts.

---

## 2. Color System (Gruvbox Palette)

Yomu uses the canonical Gruvbox palette extracted directly from [`src/ui/builtin_themes.toml`](file:///home/sabisho/Projects/yomu/src/ui/builtin_themes.toml). All surfaces adhere to comfortable, WCAG AA-compliant contrast ratios. No Catppuccin or external palettes are introduced.

### Gruvbox Dark (Default Mode)
| Semantic Token | Hex Code | HSL / Opacity | UI Role / Application |
|---|---|---|---|
| `background` | `#282828` | Normal Dark 0 | Main window canvas, library grid background |
| `surface.hard` | `#1d2021` | Hard Dark 0 | Persistent 240px sidebar, window titlebar, popovers |
| `surface.secondary` | `#3c3836` | Dark 1 | Cards, inputs, button secondary, inactive tabs |
| `surface.hover` | `#504945` | Dark 2 | Card hover state, button hover, active table rows |
| `border` | `#504945` | Dark 2 | 1px structural dividing lines and panel borders |
| `border.subtle` | `#3c3836` | Dark 1 | Soft inner card borders, subtle separators |
| `foreground` | `#ebdbb2` | Light 1 | Primary titles, headings, high-contrast readable text |
| `muted.foreground` | `#a89984` | Gray 246 | Secondary metadata, authors, chapter counts, captions |
| `dim.foreground` | `#7c6f64` | Dark 4 | Disabled states, empty placeholder hints, hotkey badges |
| `primary` (Accent) | `#fe8019` | Bright Orange | Primary CTA buttons, reading progress bars, active pills |
| `primary.hover` | `#fabd2f` | Bright Yellow | Primary button hover and interactive focus rings |
| `primary.muted` | `#d65d0e` | Neutral Orange | Subdued progress indicators, collection badges |
| `danger` | `#fb4934` | Bright Red | Destructive item removal alerts, critical errors |
| `success` | `#b8bb26` | Bright Green | Completed series badges, valid archive indicators |
| `info` / `link` | `#83a598` | Bright Blue | Archive format pills (`CBZ`, `ZIP`), interactive links |
| `reader.canvas` | `#161616` | Deep Neutral | OLED/deep background canvas behind comic pages |

### Gruvbox Light (Alternate Mode)
| Semantic Token | Hex Code | UI Role |
|---|---|---|
| `background` | `#fbf1c7` | Main window background, library canvas |
| `surface.hard` | `#f9f5d7` | Sidebar, titlebar, popover backdrop |
| `surface.secondary` | `#ebdbb2` | Card backgrounds, inputs, secondary buttons |
| `border` | `#d5c4a1` | 1px borders, table dividers |
| `foreground` | `#3c3836` | Primary high-contrast dark text |
| `muted.foreground` | `#7c6f64` | Secondary metadata and labels |
| `primary` | `#af3a03` | Primary action buttons and progress fill |
| `danger` | `#cc241d` | Destructive confirmations |

---

## 3. Typography Hierarchy

Typographic sizing uses the system sans font (`Inter` on Wayland/Linux, `SF Pro` on macOS, `Segoe UI` on Windows) paired with monospace for shortcuts and Vim commandline tokens.

| Scale Token | Font Size | Line Height | Weight | Tracking | Primary Usage |
|---|---|---|---|---|---|
| `Display` | 28px | 34px | Bold (700) | -0.02em | Hero headers, empty state hero titles |
| `Heading 1` | 22px | 28px | Bold (700) | -0.01em | Continue Reading hero title, main view titles |
| `Heading 2` | 18px | 24px | SemiBold (600) | -0.005em | Section titles (Queue, Collections, Preferences) |
| `Heading 3` | 15px | 20px | SemiBold (600) | 0.0em | Modal dialog titles, card titles |
| `Body Large` | 13px | 18px | Regular (400) | 0.0em | Library card titles, table row primary text |
| `Body Standard`| 12px | 16px | Regular (400) | 0.0em | Toolbar actions, sidebar navigation, form inputs |
| `Caption` | 11px | 14px | Regular (400) | +0.01em | Metadata, timestamps, page counts, author labels |
| `Overline` | 10px | 12px | Bold (700) | +0.06em | Category labels (`LIBRARY`, `FOLDERS & SOURCES`, `UP NEXT`) |
| `Mono / Kbd` | 11px | 14px | Regular (400) | 0.0em | Keybinding badges (`/`, `⌘,`, `Enter`, `j`, `k`) |

---

## 4. Spacing, Sizing & Layout Scales

All dimensions adhere to an 8-point harmonic desktop grid with 4-point micro-increments.

### Control Sizing Scale
- `CONTROL_HEIGHT_SEGMENT`: **24px** — Segmented control options, compact filter toggles.
- `CONTROL_HEIGHT_TOOLBAR`: **28px** — Search inputs, sort dropdowns, toolbar icon buttons.
- `CONTROL_HEIGHT_ACTION`: **32px** — Standard action buttons, sidebar nav rows, form selects.
- `CONTROL_HEIGHT_PRIMARY`: **36px** — Prominent CTAs, hero Continue Reading, large imports.
- `TOOLBAR_HEIGHT`: **52px** — Fixed height for top application toolbar & floating reader header.
- `SIDEBAR_WIDTH`: **240px** — Persistent desktop sidebar (collapses to **64px** icon-rail in compact mode).

### Corner Radius Scale
- `RADIUS_MICRO`: **2px** — Micro indicators, progress fills.
- `RADIUS_SM`: **4px** — Tags, format badges, shortcut badges, thumbnail corners.
- `RADIUS_MD`: **6px** — Standard buttons, inputs, segmented containers.
- `RADIUS_LG`: **8px** — Book cards, content panels, dropdown popovers.
- `RADIUS_XL`: **12px** — Modal dialogs, floating selection bar, hero continue reading card.
- `RADIUS_PILL`: **9999px** — Progress badges, page counter pills, filter chips.

---

## 5. Manga & Comic Artwork Presentation

- **Cover Aspect Ratio:** Standard portrait ratio of 1:1.2 to 1:1.414 (standard tankōbon / B6 trim).
- **Artwork Prominence:** Covers occupy the upper ~85% of each card with 8px top/left/right corner radius and clipped overflow.
- **Card Metadata:** Placed cleanly in a dedicated 52px area beneath the cover art. Titles and authors render with comfortable contrast and zero text truncation.
- **Reading Spread Pairing:** Dual-page spread layout renders Page 25 on Right and Page 24 on Left under Right-to-Left (Manga) progression, respecting authentic Japanese manga reading flow with zero spine gap distortion.
- **Representative Artwork:** High-resolution monochrome manga illustrations with screentones, dynamic paneling, speedlines, and Japanese sound effects.

---

## 6. Iconography System (Lucide Vector Icons)

All icons map directly to `gpui_kit::assets::IconName` and are rendered using pure SVG vector paths:

| UI Purpose | FigCraft Vector Icon | Native `gpui_kit::assets::IconName` |
|---|---|---|
| All Manga | `layout_grid` | `IconName::LayoutGrid` |
| Continue Reading / Reader | `book_open` | `IconName::BookOpen` |
| Collections / Folders | `folder` | `IconName::Folder` |
| Reading History | `clock` | `IconName::Clock` |
| Downloads / Imports | `download` | `IconName::Download` |
| Settings & Preferences | `settings` | `IconName::Settings` |
| Appearance / Theme | `palette` | `IconName::Palette` |
| Keyboard Shortcuts | `keyboard` | `IconName::Keyboard` |
| Advanced & Cache | `database` | `IconName::Database` |
| Library Search | `search` | `IconName::Search` |
| Add / Import Action | `plus` | `IconName::Plus` |
| Play / Resume Reading | `play` | `IconName::Play` |
| Completed Series | `check` | `IconName::Check` |
| Favorites Collection | `heart` | `IconName::Heart` |
| Fullscreen Viewport | `maximize` | `IconName::Maximize` |
| Lateral Page Turns | `chevron_left` / `chevron_right` | `IconName::ChevronLeft` / `IconName::ChevronRight` |
| Destructive Removal | `trash` | `IconName::Trash` |
