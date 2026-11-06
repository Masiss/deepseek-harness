# Native desktop GUI prototype

English | [中文](README.zh.md)

This project contains a standalone interactive design prototype for the DeepSeek Harness native desktop interface. It is a visual behavior reference, not the shipped application or a Host client.

## Start here

- [`docs/architecture.md`](docs/architecture.md) explains the project structure, state ownership, runtime flows, responsive behavior, and limits.
- [`docs/interactions.md`](docs/interactions.md) catalogs each screen, interaction, and simulated outcome.
- [`index.html`](index.html) is the complete prototype entry file; its markup, styles, scripts, and fixtures are in one file.

## Preview

Open `index.html` directly in a browser. The file has no package install or build step and does not call the Desktop Host.

## Relationship to the application

The shipped application is in [`../src/`](../src/) and uses Rust + gpui (`../src/gpui_main.rs`, `../src/shell/`); the earlier egui shell in `../src/main.rs` is retired behind the `legacy-egui` feature. This prototype is not included in Cargo builds or AppImages. Prototype-specific documentation stays under this directory; repository-wide docs cover the shipped product and Desktop lane.

The frozen visual and behavioral target for the gpui port is [`../DSH_UI_Handoff_v1.0/`](../DSH_UI_Handoff_v1.0/), which holds the approved HTML reference, the functional spec, the runtime capability map, and the implementation backlog. This prototype stays the place to explore interaction ideas; the handoff directory is the one that gates implementation.
