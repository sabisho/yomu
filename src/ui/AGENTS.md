# UI Presentation Layer — Agent Guide (`src/ui/`)

This guide supplements the root [AGENTS.md](../../AGENTS.md) with specific instructions for the **UI Presentation Layer** (`src/ui/`).

---

## 1. Architectural Boundary & Invariants

The UI presentation layer constitutes Tier 2 of Yomu's three-tier architecture. It provides reusable visual components and presentation screens built on **GPUI Kit 0.8.1**:

- **Presentation Decoupling (Pure Props Pattern):** UI view functions must receive domain models and callbacks via dedicated `Props` structs (e.g., `ReaderProps`, `MangaDetailsProps`). They must not hold direct `Entity<Yomu>` references.
- **Event Callback Architecture:** User actions emit standard callbacks (`on_click`, `on_action`, `on_change`) that accept `(&mut Window, &mut App)` or typed arguments, delegating business logic and state mutation to the application orchestration layer (`src/app/`).
- **Render Loop Discipline:** Render functions must remain strictly pure visual builders. Never execute state-machine side-effects, entity recreation, or synchronous disk I/O inside `Render::render`.
- **Zero UI-Thread Synchronous I/O:** Never call `std::fs::read_to_string`, `fs::metadata`, or `/proc/meminfo` within render methods or scroll handlers. Any filesystem verification must be cached or performed asynchronously.

---

## 2. Official Components & Toolkit Rules

- **Native Components First:** Always use official GPUI Kit components (`Button`, `ButtonGroup`, `Input`, `DropdownMenu`, `Dialog`, `Kbd`, `ScrollableElement`, `Spinner`) over manual `div()` constructions.
- **No Web-Habit Styling:** Do not introduce CSS strings, HTML tags, webview widgets, or ad-hoc styling wrappers. Use typed GPUI Kit layout and style builders (`.flex()`, `.gap()`, `.rounded()`, `.bg()`, `.text_color()`).
- **Semantic Theme Tokens:** Bind colors exclusively through semantic tokens derived from the active theme (`UiColors`). Never use arbitrary hardcoded hex color values in component rendering.
- **Stable Element Identification:** Assign domain-derived, stable `ElementId`s to interactive elements and list rows (e.g., `ElementId::NamedInteger("book-card", book.id_hash)`), never transient loop indices.

---

## 3. Motion & Animation Standards

- **Restrained & Purposeful:** Motion is used exclusively for spatial continuity (tab navigation, sidebar toggling, modal entrance). High-frequency surfaces (library card grids, search result lists, decoded comic page canvas) remain completely static.
- **Official GPUI Kit Transitions:** Use `EffectTransition` (`gpui_kit::base::animation::EffectTransition`) with `ease_out_cubic`:
  - View / Tab switches: 140 ms opacity drift ($0.85 \to 1.0$ or $0.90 \to 1.0$).
  - Modal / Overlay entrances: 120 ms micro vertical slide ($-6\text{px} \to 0\text{px}$) and subtle fade ($0.80 \to 1.0$).
  - Sidebar collapse: native built-in `SidebarCollapsible::Offcanvas` (200 ms).
- **Permanent Animation Policy:** Application runtime enforces `cx.set_reduce_motion(false)`. Legacy configuration fields (`disable_animations`) deserialize cleanly without disabling motion.

---

## 4. Cross-Platform Guidelines

- **macOS:** Provide AppKit native traffic lights clearance (`pl(px(80.))`) in toolbars and sidebars. Map platform shortcuts with `⌘` command key indicators.
- **Linux:** Support Wayland-native first with X11 fallback. Respect client-side window decorations (`Decorations::Client`). Ensure kinetic inertia glide functions smoothly in `ui::scroll::PlatformScrollController`.
- **Windows:** Map shortcuts using `Ctrl` key glyphs. Ensure file path separators are normalized cleanly across platform boundaries.
