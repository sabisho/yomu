# Yomu UI Design System & macOS HIG Specifications

This document outlines the visual design language, design tokens, responsive layout principles, and macOS Human Interface Guidelines (HIG) compliance for Yomu's desktop manga reader.

---

## 1. Visual Hierarchy & Design Philosophy

Yomu's user interface is modeled directly after Apple's macOS Human Interface Guidelines and the approved multi-screen design board (`Screenshots/macOS_redign_design_board.png`):

- **Content-First Presentation**: Manga cover artwork and reading panels serve as the primary visual focus. Chrome is understated, translucent, and layered.
- **Two-Pane Desktop Architecture**: Replaces traditional three-pane or multi-nested designs with a unified, persistent navigation sidebar on the left and a responsive content viewport on the right.
- **Adaptive Depth & Translucency**: Distinct layered surfaces utilizing subtle opacity (`with_alpha`), fine 1px borders (`border_color`), and backdrop translucency for sidebars, floating overlays, and modal sheets.
- **macOS Native Window Ergonomics**: Window controls (close, minimize, zoom) integrated into the top of the sidebar with proportional padding (14px circle size, 8px spacing, matching macOS traffic lights).

---

## 2. Design Tokens

### 2.1 Color Palette & Theme Tokens

The application supports both system-adaptive macOS Light and Dark themes, alongside Catppuccin and Noctalia themes, managed via `UiColors`:

| Token | Dark Appearance | Light Appearance | Usage |
|---|---|---|---|
| `background` | `hsl(220, 18%, 10%)` | `hsl(0, 0%, 98%)` | Root window background canvas |
| `surface` | `hsl(220, 16%, 14%)` | `hsl(0, 0%, 100%)` | Cards, panels, sidebars, popups |
| `border` | `hsla(220, 12%, 25%, 0.5)`| `hsla(0, 0%, 82%, 0.6)` | Card outlines, dividers, separators |
| `foreground` | `hsl(220, 15%, 95%)` | `hsl(220, 15%, 15%)` | Primary titles, body text, high-contrast glyphs |
| `muted` | `hsl(220, 10%, 65%)` | `hsl(220, 10%, 45%)` | Secondary metadata, subtitles, stats |
| `accent` | `hsl(211, 100%, 50%)` | `hsl(211, 100%, 50%)` | Interactive highlights, active tabs, buttons |
| `accent_foreground`| `hsl(0, 0%, 100%)` | `hsl(0, 0%, 100%)` | Text rendered on accent backgrounds |

### 2.2 Typography Scale

Typography uses standard system font stacks (San Francisco on macOS, system sans-serif on Linux) with rigorous weight hierarchy:

- **Display Titles (Details View)**: 24px (`text_2xl`), Bold (`font_weight(FontWeight::BOLD)`), line height 32px.
- **Section Headers (Sidebar, Continue Reading)**: 16px (`text_base`), Bold / Semibold, line height 22px.
- **Card Titles & Menu Items**: 14px (`text_sm`), Medium (`font_weight(FontWeight::MEDIUM)`), line height 18px.
- **Metadata Subtitles, Badges & Labels**: 12px (`text_xs`), Regular or Semibold, line height 16px.

### 2.3 Corner Radii & Geometry

Corner radii adaptively follow user settings (`CornerRounding`) with standard defaults:
- `Medium` default: `8px` for buttons, cards, search input, and sidebar active indicators.
- `Large`: `12px` for hero banners and modal dialog cards.
- `Pill / Full`: `9999px` for status badges, tags, chevrons, and floating control pills.

---

## 3. Screen Architectures

### 3.1 Two-Pane Shell & Persistent Sidebar
```
+-------------------+---------------------------------------------------------+
| [●][●][●]         |  < >  [        🔍 Search Manga...        ⌘K ]  [+ Import] |
| YOMU              +---------------------------------------------------------+
|                   |                                                         |
| 📖 Continue Read  |  HERO MANGA BANNER                 UP NEXT QUEUE        |
| 📚 All Manga (84) |  [ Cover ] Title                   [Item 1]             |
| 📁 Collections (8)|            Progress: 75%           [Item 2]             |
| ⏱ History         |            [▶ Continue Reading]    [Item 3]             |
| ⬇ Downloads       |                                                         |
|                   |  RECENTLY ADDED                                         |
|                   |  [Cover]    [Cover]    [Cover]    [Cover]               |
|                   |  Ch. 12     Ch. 5      Ch. 1      Ch. 84                |
|                   |                                                         |
| [Settings]        +---------------------------------------------------------+
+-------------------+---------------------------------------------------------+
```

### 3.2 Immersive Reader Mode
- In Reader mode, the sidebar and standard toolbar are completely unmounted, dedicating 100% of viewport area to manga artwork.
- **Auto-Hiding Top Bar**: Reveals on hover or tap, displaying volume title, chapter/page count (`Ch. 12 · 45 / 180`), Page Fit selector, Reading Direction toggle (LTR/RTL), and Fullscreen button.
- **Edge Chevrons**: Semi-transparent floating circular buttons (`left_4` and `right_4`) for intuitive page flipping.
- **Bottom HUD Pill**: Subtle progress feedback showing current page percentage and reading mode status.
