# Themed UI and Command Prompt Plan

This document records the product direction, implementation plan, and GPUI Kit research for the UI redesign request. Read it before changing the app UI or replacing GPUI dependencies. It is intended to keep later implementation work grounded in this repository and avoid repeating the initial research.

## User direction

- Use the `gpui-kit_inspiration.jpg` image in the project root as a reference for component style: dark surfaces, fine borders, compact menus, tabbed settings, clear switches, and focused confirmation dialogs. It is a style reference, not a final settings layout or a full app mockup.
- For the library screen specifically, the user has since clarified the reference layout: center the search field, remove the Yomu title and total book count, keep the top bar transparent with no colored strip, and put a small hamburger menu at the top right for settings/import/library actions.
- Do not define a separate yomu color palette. Let the user select GPUI Kit themes, and read UI colors from the active Kit theme throughout the application.
- Offer GPUI Kit's built-in themes and a user-editable `themes.toml` file with hex color values and other supported theme options.
- Design the settings tabs around the real app preferences. Include settings for persistent modes and preferences that currently can only be changed with commands or key bindings. The tabs are open to expansion; the screenshot's Appearance/Reader tabs are not a fixed requirement.
- Replace the black bottom command strip with a floating command box with contextual completions and descriptions. The interaction should feel like a lightweight Noice/command-prompt panel: type after `:`, inspect suggestions, complete or navigate them, run with Enter, and dismiss with Escape.

## Current project facts and implementation status

Implementation is complete across all layers. **Completed: Layer 1, framework migration; Layer 2, built-in theme plumbing; Layer 3, custom theme file support; Layer 4, tabbed Settings; Layer 5, floating command prompt; Layer 6, app-wide visual polish; Layer 7, opacity-aware glass surfaces; Layer 8, progressive imports and directory folder views; Layer 9, search dismissal and menu icon polish; Layer 10, semantic theme consistency and Noctalia palette template; Layer 11, automatic theme reload; Layer 12, full native GPUI Kit migration and Noctalia M3 alignment (V 1.0 UI Redesign).** The app uses the pinned GPUI Kit 0.7.3 runtime. Its UI palette comes from active Kit semantic color tokens. Ad-hoc custom UI widgets were systematically eliminated in favor of native GPUI Kit components:
- `TabBar::new(...).underline()` for settings tabs with sliding indicator and dynamic sizing.
- `DropdownMenu` with native `Popover` for fit/theme selections with automatic outside-click dismissal.
- `Switch` for boolean preferences.
- `Input` with `InputState` for series naming and library search (including native text editing, undo/redo, cursor positioning, search prefix icon, and cleanable button).
- `Progress` for reading progress bars.
- `Tag` for volume counts and shortcut section counts.
- `Table`, `TableHeader`, `TableBody`, `TableRow`, `TableHead`, and `TableCell` for the keyboard shortcuts catalog, with action descriptions on the left and shortcut badges on the right.
- Content-driven dynamic menu sizing (`min_w`, `max_w`, `w_auto`) replacing static pixel widths.
- High-contrast Noctalia M3 active selection pairing solid `colors.accent` background with `colors.accent_foreground` text.
- Full window opacity (`WindowBackgroundAppearance::Opaque`) eliminating cross-platform translucency and Wayland popup glitches.

**Status:** UI Redesign V 1.0 is complete and all 135 unit and UI tests pass cleanly.

- The app depends on `gpui-kit = "=0.7.3"` and declares Rust 1.92 in `Cargo.toml`.
- The latest GPUI Kit docs checked for this plan are version 0.7.3. Its installation guide describes Kit as the entry point that brings in a matching GPUI snapshot (`gpui-pre = 0.3.9` in the checked guide), and notes the current locked Linux dependency graph requires Rust 1.92. Do not add Kit beside the existing GPUI dependency and assume the types interoperate. First confirm a coherent Kit/runtime version, then migrate the GPUI entry point and update the project's minimum Rust and Nix/devenv toolchains if needed.
- GPUI Kit documents more than 20 built-in themes and a `ThemeRegistry` for looking up and applying them. Theme entries include a name and light/dark mode, along with colors and optional font, radius, and shadow settings.
- The documented `ThemeRegistry` directory watcher reads JSON theme files. Kit's theme schema and examples describe JSON theme sets. Kit does not natively promise a `themes.toml` loader; yomu parses TOML and converts validated entries to Kit `ThemeConfig` values.
- The app can meet the TOML requirement with an application-side adapter: read and parse `themes.toml`, validate the supported Yomu format, convert its theme set to the JSON representation expected by the pinned Kit version, then register/apply it using that version's public API. Keep the TOML contract documented and include a sample. Do not claim Kit itself parses TOML.
- Verify the exact `ThemeRegistry`, `Theme`, initialization, and theme-update APIs against the GPUI Kit version selected in the compatibility spike. Kit's APIs and version pins can change.
- Existing UI and behavior are concentrated in `src/main.rs`; parsing and command actions are in `src/vim.rs`; reusable menu policy is in `src/menu.rs`; persisted preferences are in `src/library.rs`'s `Settings`.
- The current persisted settings include reading direction (`rtl`), fit mode (`fit`), animations (`animate`), normal continuous scrolling (`continuous`), gapless webcomics override (`webcomics`), series grouping (`group_series`), window opacity, and the Kit theme name.
- Custom themes live at `$XDG_CONFIG_HOME/yomu/themes.toml` (normally `~/.config/yomu/themes.toml`). If absent, yomu creates it from `themes.example.toml`. The explicit reload action validates into a temporary map first; on failure the previous custom registry and active theme remain unchanged.
- `render_settings` is a custom tabbed panel; `render_menu` and `render_confirm` are custom overlays. The reader renders an active `Vim` command draft in a themed floating panel. `Vim` remains the parser and owns command-line text; a UI suggestion catalog supplies labels, descriptions, and completion candidates without replacing parser behavior.
- The project is Linux/Wayland-targeted today. Preserve its file dialog, image resource, reduced-motion, keyboard, and compositor behavior during a framework migration.

## Research references

These are the official references checked while writing this brief. Recheck APIs only when the selected Kit version differs or its pinned documentation says an interface changed.

- [GPUI Kit installation and version alignment](https://gpui-kit.com/docs/installation/) — current documented Kit version, matching GPUI snapshot, Rust toolchain notes, and dependency usage.
- [GPUI Kit theme guide](https://gpui-kit.com/docs/theme/) — active theme access, built-in theme registry, and theme directory registration example.
- [GPUI Kit theme schema](https://github.com/longbridge/gpui-kit/blob/main/.theme-schema.json) — supported theme metadata, light/dark modes, color tokens, font fields, radii, and shadows.
- [Theme registry implementation reference](https://docs.rs/gpui-component/latest/src/gpui_component/theme/registry.rs.html) — registry behavior and its JSON theme loading path. Confirm the same behavior in the selected Kit release.
- [GPUI Kit tabs](https://gpui-kit.com/docs/components/tabs) and [switches](https://gpui-kit.com/component/switch/) — useful for settings navigation and immediate boolean settings.
- [GPUI Kit menus](https://gpui-kit.com/component/menu/) and [dialogs](https://gpui-kit.com/component/dialog/) — useful for book context actions and confirmation overlays.
- [GPUI Kit design guidance](https://gpui-kit.com/docs/design-guides/) — component composition, native interaction, and accessibility guidance.

## Proposed settings structure

Keep each tab populated with real app preferences and reference material. Do not add fake or nonfunctional controls to fill space.

| Tab | Initial contents |
|---|---|
| **Appearance** | Built-in/custom theme cycling; window background opacity; custom theme file location and reload/error status. Theme colors come from Kit's active theme. A searchable picker is a possible future improvement, not present behavior. |
| **Library** | Series grouping preference; future library sorting preferences only after sorting is implemented. Preserve the regular ungrouped grid mode. |
| **Reader** | Reading direction; fit mode; animation preference; reading layout selection for single page, normal continuous scrolling, and webcomics. Keep the existing semantics where webcomics forces continuous layout and turning it off restores the saved normal mode. |
| **Keyboard & Commands** | Static shortcut/command reference, including available `:` commands and arguments. Search/filtering is not implemented. Do not imply key remapping exists unless it is separately implemented. |

Use the settings screen to expose persistent choices currently reachable only by reader commands, including direction (`:rtl` / `:ltr`), fit (`:fit`), animation (`:animate`), continuous mode (`:continuous`), and webcomics mode (`:webcomics`). Commands and settings must call the same state mutation paths so they cannot drift. Transient actions such as showing the info HUD do not need to become persisted preferences just because they have a command.

### Theme selection and custom TOML

- Persist the chosen theme name in the existing settings file unless migration work establishes a better app-wide configuration owner. Older libraries must default to a safe Kit theme.
- Keep the user's custom theme file under the XDG config directory, at `$XDG_CONFIG_HOME/yomu/themes.toml` (normally `~/.config/yomu/themes.toml`). Do not put editable configuration in the app's install directory or the user data directory.
- Define a Yomu TOML theme-set format with a top-level set name and one or more `[[themes]]`, each with a name, `mode = "light" | "dark"`, and a `[themes.colors]` table. Keys should map to the pinned Kit theme schema; hex colors should accept documented `#RRGGBB` and, if the pinned API supports alpha, `#RRGGBBAA` values. Optional Kit font, radius, and shadow fields may be exposed after confirming their exact representation.
- Include a commented sample and document token names. Prefer partial overrides that inherit Kit defaults if the pinned `ThemeConfig` supports them. Validate names, mode, token names, and hex values. On parse/validation failure, retain the active theme and show a useful message.
- Start with an explicit **Reload custom themes** action in Appearance settings. Add live file watching only if it works reliably with the selected Kit release and can report errors without repeatedly replacing a good active theme.
- Theme selection should apply immediately and update all windows/components. Use Kit's supported theme-update/change path so its component and base theme projections stay in sync; do not mutate theme globals directly without synchronizing them.

## Floating command box

Replace the black bottom status strip used for `Vim::status()` with a themed overlay anchored near the bottom center of the reader. Keep the page visible behind the box. A suitable first layout is a compact input row and a suggestion panel directly above it, with a subtle border/surface from the active Kit theme.

Implement command descriptions and completion metadata from one command catalog shared with parsing where practical. Suggestions should filter as input changes, show a short description and valid arguments, and account for current mode/state. Examples include `:fit` → `page`, `width`, `height`, `original`; `:continuous` → `on`/`off`; `:webcomics` → `on`/`off`; and `:zoom` → reset or a percentage. Unknown commands should remain editable and produce the existing useful error feedback on Enter.

Initial interactions:

- `:` opens the prompt; focus goes directly to text entry.
- Typing updates matching command/argument suggestions.
- Up/Down and Tab/Shift+Tab move the selected suggestion; Tab may insert the selected completion while preserving further argument entry.
- Enter runs the command or selected completion as appropriate.
- Escape closes the prompt and cancels its draft without running it.
- Backspace edits the draft; existing command-line key behavior and reader shortcuts outside the prompt remain intact.

Use the pinned Kit's themed input and accessible list/menu primitives where they fit. A small custom composition is acceptable if the command prompt's placement and behavior do not fit a general command palette component. The current parser remains the source of command semantics; the prompt adds input presentation and completion behavior.

## Implementation phases

1. **Compatibility spike — complete:** verified a minimal Kit window and an isolated application compile against Kit 0.7.3 / `gpui-pre` 0.3.9. The supported build toolchain minimum is Rust 1.92. Existing xattr and stacksafe-macro patches no longer apply to the selected dependency graph and were removed from the active Cargo patch configuration; vendored source remains untouched.
2. **Framework migration — complete:** replaced direct GPUI dependency with the aligned Kit dependency, adapted application initialization and GPUI API changes, updated Cargo lock and documented minimum Rust version. Core modules remain independent of GPUI/Kit. The app's existing window still opens using the existing Yomu root view; Kit `init` runs before app UI use.
3. **Theme plumbing — complete:** Kit initializes before the app view, the UI palette comes from semantic Kit color tokens, and a persisted Settings control cycles built-in and custom themes. Yomu parses and validates the documented TOML format, checks hex colors and supported tokens, converts entries to Kit configs, provides a starter file, and reports reload errors without changing the current theme.
4. **Settings — complete:** tabbed shell with Appearance, Library, Reader, and Keyboard & Commands sections. Persistent preference controls call the same command mutation methods where applicable.
5. **Command prompt — complete:** render the editable Vim draft in a floating themed box; filter command and argument suggestions; support arrow selection, Tab/Shift+Tab completion, Enter execution, and Escape cancellation while preserving `:` command behavior.
6. **App-wide visual polish — complete:** use GPUI Kit buttons for app actions, give library cards and the library header a clearer visual hierarchy, and keep overlays on active semantic theme surfaces. Custom menus/dialogs remain in place to preserve yomu's keyboard selection, dismissal, and destructive action policies; migrate these to Kit compound components only when those components support the same behavior.
7. **Verification:** run formatting, `cargo check`, and the existing tests after migration; add focused interaction coverage for theme choice, TOML errors, settings/command parity, command completion, Enter/Escape, and preserved reader/library interactions. Visually inspect built-in light/dark themes and a custom theme on a real Wayland compositor.

## Invariants and acceptance criteria

- No hardcoded Yomu UI palette remains as the source of component colors; all UI surfaces and text use active Kit theme tokens. Intentional colors in comic artwork remain unchanged.
- Built-in theme choice applies live and survives restart. Existing preference fields and older `library.json` files continue loading safely.
- Custom `themes.toml` uses hex values, validates against the documented supported token set, can be reloaded from the settings UI, and a broken file does not blank/reset the current theme.
- Every implemented persistent reader/library preference has a settings control and uses the same implementation/persistence path as its command or key binding.
- The command box is not a full-width black bottom strip. It shows useful dynamic command/argument suggestions, supports completion/navigation, runs on Enter, and cancels on Escape.
- Existing open-book behavior, page order, continuous/webcomics behavior, progress, marks, caching, import, selection, and destructive confirmation semantics remain intact.
- UI behavior is checked both with the available simulated platform tests and visually on Wayland; report which was actually run.
