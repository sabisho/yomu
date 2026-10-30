# Product Roadmap

This document outlines prioritized future work, planned features, and tracked technical debt for Yomu. Items are prioritized based on user impact and architectural cleanliness.

---

## 🎯 High Priority

### 1. Dual-Page Spread Reading Mode
- **Goal:** Display two portrait pages side-by-side to replicate physical manga reading.
- **Behavior:**
  - Automatic single-page fallback for wide landscape pages (spread covers).
  - Respect reading direction (Right-to-Left pairs right page first; Left-to-Right pairs left page first).
  - Toggle via reader key `d` and `:spread [on|off]` command, with persistence in `Settings`.

### 2. Library Sorting Options
- **Goal:** Allow users to sort library cards and list rows beyond the default import order.
- **Criteria:**
  - Sort by: Title (alphabetical/natural), Last Read (recency), Date Added (import timestamp), Page Count, Reading Progress.
  - Direction: Ascending / Descending toggle.
  - Persist active sort preference in `library.json` without altering source file order.

---

## 🛠️ Medium Priority

### 3. CB7 / 7z Archive Support
- **Goal:** Support `.cb7` and `.7z` comic archives alongside CBZ, CBR, ZIP, and RAR.
- **Engineering Requirements:**
  - Use a safe, pure-Rust or well-vetted 7z archive extractor.
  - Integrate with `archive.rs`'s streaming interface without blocking the UI thread.

### 4. Official Upstream Momentum Scrolling Migration
- **Goal:** Transition from `scroll.rs`'s custom kinetic decay to official upstream GPUI momentum APIs once published.
- **Condition:**
  - Track upstream GPUI / GPUI Kit releases for Linux/Windows touchpad momentum events.
  - Preserve macOS AppKit native momentum pass-through.

---

## 🔍 Low Priority / Exploratory

### 5. Multi-Window and Secondary Monitor Reading
- Support detaching the reader into an independent window on a secondary display while keeping the library open.

### 6. Book Metadata Editing & Tagging
- Support editing title, series, volume number, and custom tags directly within the Manga Details view (`ui::details`).

---

## 📋 Completed Features (Archive)

- ✅ **Framework Migration to GPUI Kit 0.8.0**
- ✅ **macOS HIG Two-Pane Redesign** (Persistent 240px sidebar, unified toolbar, responsive content views)
- ✅ **Incremental Modularization** (Consolidated library rendering, overlays, command box, help modal, history view)
- ✅ **Kinetic Momentum Scrolling** (`PlatformScrollController` with touch catch, stillness fallback, and reduced motion override)
- ✅ **Floating Command Prompt** (Contextual completion for reader commands, fits, and theme switcher)
- ✅ **Automatic Directory and Series Grouping** (Directory-derived folders with manual series override)
- ✅ **Safe Desktop Integration** (Explicit CLI installation flags `--install-desktop` / `--install-icons`)
