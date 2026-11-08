# Zed UI reference — influence without cloning

Verified against current public Zed documentation on **2026-11-08**; Zed product layout evolves, so **DSH V2 remains the layout authority**.

| DSH surface | Useful Zed reference | Apply to DSH | Do not copy |
|---|---|---|---|
| Project/Explorer | https://zed.dev/docs/project-panel | Dense tree, expandable folders, keyboard up/down/left/right, safe previews, selected vs focused row; context menu with scope | A second project sidebar or Activity Bar |
| Agent timeline | https://zed.dev/docs/ai/agent-panel | Inline tool activity with progressive disclosure, legible streaming, review changes, follow active tool | Arbitrary Agentic layout or editor-first premise |
| Git/Changes | https://zed.dev/docs/git | Flat/tree change status, diff/commit workflow, conflicts and staged distinction, dock navigation | Duplicate Git and Changes surfaces |
| Commands | https://zed.dev/docs/command-palette | Single command registry, shortcuts shown, keyboard fuzzy search, Esc return-focus | Make hamburger itself the palette |
| Panels/docks | https://zed.dev/docs/development/glossary | Left/right/bottom docks, persisted sizes, toggle/restore, pane semantics | Zed's current default dock arrangement as a forced DSH redesign |
| Current Zed Agentic layout | https://zed.dev/docs/ai/parallel-agents | Thread status, cross-project awareness, clear handoff | Permanent Threads sidebar: DSH has Session switch at Explorer foot |
| Appearance & Keymap | https://zed.dev/docs/appearance | Separate ui/buffer/terminal typography; keyboard presets and accessible display states | Assume pinned GPUI automatically supports Zed settings UI |

## DSH's product differentiation

Zed is a full code editor with agent capability. DSH is an **agent supervisor** with Explorer/Changes/PTY support; therefore the center is a conversation plus tool/approval timeline. Avoid a code editing surface in the center unless the user explicitly enters file preview. Keep the primary reading area large and the composer stable.

## Practical Zed-inspired polish (P0/P1, after verifying code)

- Delineate **selected**, **hovered** and **keyboard-focused** rows; keep tree row height consistent with density.
- Jump from changed file to safe preview without permanently crushing Agent panel.
- Tool activity: collapse successful runs, expand failures and approvals; preserve stdout/stderr, exit code and duration where Host already provides them.
- Git review: differentiate staged/unstaged and file/hunk scope; protective confirm for discard/checkout, plus stale-guard.
- Show no repo, missing workspace, disconnected Host, interrupted run and pending approval as distinct states.
- Menus, palette and settings must not fork the command/action implementation.
