# Agent Note: Delete the retired egui desktop shell

Status: implemented

English | [中文](2026-11-06-delete-retired-egui-desktop-shell.zh.md)

## Problem

`apps/desktop-tauri/native-app` shipped one gpui binary plus a second, transitional egui binary (`dsh-desktop-egui-legacy`) behind the `legacy-egui` feature. Both rendered the same V2 screens from shared protocol and preference modules, so every UI change had to be made twice. That duplication produced real drift: features added to the gpui shell reached the egui shell late or with different wording, the egui shell broke while nobody shipped it, and its snapshot baselines had to be re-recorded for changes that only mattered to gpui.

## Decision

The native app has exactly one binary, `dsh-desktop-native-app` (`src/gpui_main.rs` plus `src/shell/`). The egui shell, its feature, and everything that existed only to build or test it are deleted: `src/main.rs`, `src/ui_harness.rs` with `src/ui_harness/audit.rs`, `src/settings_panel.rs`, `src/transcript_view.rs`, `tests/snapshots/*.png`, the `legacy-egui` feature, the optional `eframe` dependency, the `egui_kittest` dev-dependency, and every `#[cfg(feature = "legacy-egui")]` block in the shared modules. `fonts.rs` now exposes only `contains_cjk`; the font catalog and egui font-stack helpers are gone, as are the egui-only helpers in `document_preview`, `explorer`, `question_card`, `session_list`, `terminal_ansi`, `layout`, and `ui_preferences`. The `dsh-desktop-egui-legacy` target and all `DSH_NATIVE_UI_*` probe wiring are removed from the packaged-journey scripts.

Shared modules stay renderer-neutral. `KeyChord`, `Rgba8`, `AnsiColor`, `Transcript`, preferences, and the typed clients keep the contracts they had, so gpui remains one consumer of a framework-free core.

## Alternatives considered

- **Keep the egui binary buildable but frozen.** Rejected: it still had to compile against every shared-module change, which is exactly the maintenance that produced the drift, and nobody shipped it.
- **Keep full parity between both shells.** Rejected: it doubles the cost of every UI slice for a renderer that no release uses.
- **Keep only the egui harness while deleting the binary.** Rejected: the harness drives the egui renderer, so it cannot outlive it, and keeping eframe linked only for tests reintroduces the dependency the port removed.

## Consequences

One renderer, one set of screens, and roughly twelve thousand lines of second implementation gone. The shared core no longer carries cfg-gated presentation code.

The cost is test evidence. The `egui_kittest` harness was the only headless UI harness in the lane: it provided AccessKit interaction tests, screenshot baselines, and the L2 packaged journeys for Marketplace, Office preview, PTY panels, the profile default model, and the signed update modal. `src/harness.rs` replaces part of it: it opens the real `DesktopApp` in gpui's test window, reads the geometry the renderer produced through `debug_bounds`, asserts the V2 bar heights, dock widths and thresholds, the Changes toggle, and page-versus-conversation surfaces, and compares rectangle reports against `tests/snapshots/shell-1366x768.txt` and `shell-1920x1080.txt`. `scripts/native-journey-probe.sh shell` runs the same harness against the packaged Host: the shell connects, creates a Session through its own RPC path, and confirms it is listed.

Two capabilities stay missing. gpui 0.2.2 ships no accessibility tree at all, so AccessKit-style assertions cannot be written against this renderer; the harness asserts rendered geometry and shell state instead. Nothing rasterizes a frame, so screenshot review still has no automated baseline. The prompt → tool → approval journey through a packaged Host is likewise absent, and `office-component-probe.sh`, `model-default-probe.sh`, and `update-flow-probe.mjs` keep only their Host-level checks.
