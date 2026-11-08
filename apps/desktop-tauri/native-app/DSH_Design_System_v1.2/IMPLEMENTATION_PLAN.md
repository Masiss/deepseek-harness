# Implementation plan — themeable design system, V2 shell parity, Git review surface

Status: every slice is implemented on the working branch (local commits, not pushed): A1, A2, A3, B,
C, D, G0, G1, G2, G3, G4a, G4b, plus the icon/shortcut polish pass. Native screenshots at
1366×768, 1440×900 and 1920×1080 in both schemes are captured from the shipped binary. Audit date 2026-11-08, source `native-app` @ `56ff98e454`.
Authority: `DSH_UI_V2_Approved_Reference.html` (visuals) + `DSH_FEATURE_TRACEABILITY.md` + live Host.
This document supersedes nothing; `FOUNDATIONS.md` role names are used as the semantic token vocabulary.
The Git surface composition is settled in `prototype-git-v1.3.html` and described in
`GIT_SURFACE_V1.3.md`; this plan carries it into the native shell.

Owner direction that shapes this plan: **the design system palette is only the built-in theme set.**
Themes are user data: a user can install a theme file or a theme repository from GitHub, and the app
must render any valid pack without code changes.

## Problem

The native GPUI shell renders with one hardcoded warm-grey/orange palette, ignores the persisted
`ThemeMode`, and cannot accept a theme from outside the binary. Its shell chrome, composer and
transcript also diverge from the frozen V2 reference, and the Changes column has no staging groups,
hunk review, commit workflow or history. Measured evidence:

| Area | Current | Reference | Evidence |
|---|---|---|---|
| Light canvas / chrome | `#F1F2F3` body, `#FFFFFF` top | `#F6F8FB` / `#F4F7FB` | screenshot sample; `ui_preferences.rs:209-235` |
| Accent default | Harness orange `#E95618` | blue `#257FD3` light / `#4A99F7` dark | `ui_preferences.rs:62-100`, `shell/theme.rs:170` |
| ThemeMode | persisted but ignored; OS appearance wins | Light / Dark / System honored | `shell/app.rs:896`, `:4402-4409` |
| Theme sources | 4 enum packs + hand-edited `customTheme` JSON | user-installable packs | `ui_preferences.rs:118-246` |
| Explorer / Changes width | 390 / 380 | 258 / 485 measured at 1366 (300 / 565 proposed) | `layout.rs:10-19`, screenshots |
| Reading column | none (`px(14)` only) | 790 centered | `render/agent.rs:210-225` |
| Agent tab | absent without a Session; per-session tabs with × | always 34px `Agent` buffer tab | `render/mod.rs:162-238` |
| Top bar | text chips `☰`, `Search ⌕`, `Changes ●`, centered title | icon buttons, workspace picker, no centered title | `render/mod.rs:291-430` |
| Search binding | opens Session popover | opens palette | `render/mod.rs:390-403` |
| Explorer header | title row + "Read only · scoped…" | no title row | `render/explorer_panel.rs:222-250` |
| Status bar | five text tools with glyphs, duplicated workspace text | icon buttons + counts + status dot | `render/mod.rs:1690-1800` |
| Composer | 11 controls incl `＋ Attach`, Queue/Steer/Plan/Ask/History | attach + `@`, model + mode, send icon | `render/agent.rs:693-870` |
| Preferences | page replacing the Agent column | 910×680 overlay with section nav | `render/pages.rs:14-44` |
| Changes files | flat list, per-file stage/discard only, no grouping or bulk actions | Staged/Changes/Untracked groups, kind chips, stats, multi-select | `render/mod.rs:2253-2360`, prototype |
| Diff | raw patch line dump, no gutters or hunk actions | hunk cards, old/new gutters, per-hunk stage/discard | `render/mod.rs:2437-2530`, prototype |
| Commit / history | commit box only; no amend, push-publish, or history | commit bar with amend/sign-off/push, commit graph | `render/mod.rs:2359-2435`, prototype |

## Proposal

### 1. Theme is data, not an enum

Replace `ThemePack` + `ThemePalette` + `AccentScheme` colour literals with a theme-file model:

```json
{
  "schemaVersion": 1,
  "id": "dsh",
  "name": "DSH",
  "author": "DeepSeek",
  "license": "Apache-2.0",
  "source": "builtin",
  "colors": {
    "dark": {
      "canvas": "#0d1721", "chrome": "#0d1720", "surface": "#101b26",
      "surfaceDeep": "#0c1620", "surfaceRaised": "#14212d",
      "border": "#263544", "borderFaint": "#1c2a37",
      "text": "#d8e0e9", "textSecondary": "#8b9aac", "textTertiary": "#65788a",
      "accent": "#4a99f7", "accentText": "#0b1a2a", "selection": "#18324b",
      "success": "#53d49b", "danger": "#f17c80", "warning": "#e9b46b"
    },
    "light": { "...": "..." }
  },
  "accents": {
    "blue":   { "dark": {"accent": "#4a99f7", "selection": "#18324b"}, "light": {"accent": "#257fd3", "selection": "#e1eefb"} },
    "teal":   { "...": "..." },
    "violet": { "...": "..." }
  }
}
```

- Semantic roles only; no per-component colours. Metrics (density, radius scale, shell heights,
  panel bounds) stay in the app, not in theme files, so a theme cannot break layout.
- Light and dark are required for every pack; a missing side fails validation rather than silently
  reusing the other side.
- `accents` is optional. A pack without accents still renders with its own `accent`/`selection`.
- Built-in packs ship via `include_str!` (deterministic, no filesystem or network dependency):
  `dsh` (design-system tokens from `FOUNDATIONS.md`), `harness` (today's palette, kept so existing
  users do not get restyled), `high-contrast`, `solarized`.
- Module layout: `src/theme/{mod.rs, tokens.rs, pack.rs, store.rs, contrast.rs}`; `shell/theme.rs`
  becomes the resolved per-frame snapshot (`Theme`) built from a `ResolvedTheme`, keeping
  `sp()/fs()/gap()/control_height()/corner()` as the metric API.

### 2. Sources, precedence, discovery

| Priority | Source | Path |
|---|---|---|
| 1 | built-in packs | `include_str!` in binary |
| 2 | user packs | `<DSH_HOME>/themes/<id>.json` |
| 3 | fetched packs (cached) | `<DSH_HOME>/themes/cache/<id>.json` + `<id>.meta.json` |

- Duplicate `id` across sources: most specific source wins, and Preferences lists the origin.
- Selected id that cannot be resolved: render with `dsh` and show a persistent, dismissible banner
  with the concrete reason (file missing, invalid JSON pointer, failed contrast). Never panic,
  never silently substitute.
- `id` is validated as `^[a-z0-9][a-z0-9._-]{0,63}$` and used as the filename, so a pack cannot
  escape the theme directory.

### 3. Preferences schema v6

`ui_preferences.rs:13` is `SCHEMA_VERSION = 5`. v6 replaces the pack enum with a stable id and keeps
every v5 field readable:

```
themePack: "default" | "high-contrast" | "solarized" | "custom"   (v5)
  -> themeId: "dsh" | "high-contrast" | "solarized" | "custom"    (v6)
customTheme: { dark: {...}, light: {...} }  -> kept, stored as the editable `custom` pack
accentScheme: Harness|Blue|Teal|Violet      -> kept (accent stays a preference, not a pack)
```

- Migration maps `default` → `harness` (preserving today's look) and new installs default to `dsh`
  (design-system palette). This is deliberate: committed user preferences are not silently restyled.
- Unknown `themePack` from an older/hand-written file maps to `dsh` with a visible notice.
- Workspace overrides store an optional `themeId`; an override pointing at a removed pack falls back
  to the global selection with a notice instead of failing the whole preferences load.
- Save remains atomic (temp file + rename) and validated before write, as today.

### 4. Install and update flows

- **Add theme file…** — native file picker (`rfd` is already a dependency) → validate → copy into
  `<DSH_HOME>/themes/` → select it.
- **Export / duplicate** — any pack can be duplicated into an editable personal pack; the first
  editor writes the full semantic JSON so users have a complete starting point.
- **Import from GitHub** — user supplies either a raw `.json` URL or `owner/repo[@ref]` plus an
  optional path; the app resolves `raw.githubusercontent.com`, fetches once, validates, caches, and
  records provenance (`sourceUrl`, `ref`, `etag`, `fetchedAt`).
- **Updates are explicit**: Preferences shows "Update available" only after the user presses
  "Check for updates"; a remote pack is never refreshed behind the user's back. Offline or failed
  refresh keeps the cached copy and shows the failure.
- Constraints: HTTPS only, single JSON document, ≤ 64 KiB, ≤ 10 s timeout, no redirects to
  private/link-local addresses, no execution of anything from the pack. Provenance, license and
  author are shown in the list; absence of a field is shown as absent, never as "verified".
- Repository-wide manifests (`themes.json` listing several packs) are a later extension; v1 imports
  one document per action.

### 5. Validation and contrast gate

`pack.rs` validates structure for every pack: schema version, id pattern, required roles,
`#rrggbb`/`#rrggbbaa` colour syntax, and both colour schemes present. Unknown keys are rejected with
the exact JSON pointer, per the repository rule that misconfiguration fails loud.

Contrast is measured for every pack but only **blocks untrusted input**. Trusted packs are `dsh`,
`harness`, `high-contrast`, `solarized` and the migrated v5 `customTheme`; their measured failures are
a deterministic warning list, because a shipped first-party palette (or a user's existing custom file)
must render even though `solarized` is below 4.5:1 and `harness` light `textSecondary` measures 4.44:1.
User-installed files and GitHub imports are rejected when `text` or `textSecondary` is below 4.5:1
against `canvas`, `surface`, `surfaceDeep` and `surfaceRaised`, when `accent` is below 3:1 against
those four backgrounds, or when `accentText` on `accent` is below 4.5:1.

`border` has no contrast floor. A 1px separator is not a WCAG non-text boundary, and every realistic
hairline measures 1.1–2.1:1; it appears in the warning report only.

The resolver returns `{ pack_id, trusted, errors, warnings }` so a later surface can show provenance
and contrast status. Gate tests use synthetic packs for reject/accept, and assert the trusted packs'
warning lists, so a palette change is visible in the diff.

### 6. Shell parity changes (unchanged intent, now token-driven)

- Top bar (`render/mod.rs:291`): icon buttons for menu/search/changes, workspace picker button with
  chevron; remove the centered session title. Search opens the palette (`CommandId::ShowPalette`),
  workspace opens a new workspace picker surface; the Session picker stays at the Explorer foot.
- `render/mod.rs:162`: always render the 34px buffer strip with the `Agent` tab and 2px accent
  underline; per-session tabs remain as an additional buffer group, styled per V2 (no custom ×
  glyph; close affordance on hover/context).
- `render/mod.rs:1690` status bar: icon buttons with counts (terminal, problems, status dot), branch
  text with chevron on the left, no duplicated workspace label.
- `render/explorer_panel.rs:222`: drop the title row and the "Read only · scoped…" line (scope moves
  to a tooltip / empty-state copy); tree rows snap to density heights 23/27/32.
- `layout.rs`: `EXPLORER_DEFAULT 300` (210–390), `CHANGES_DEFAULT 565` (340–720), center minimum 420,
  reading column 790, composer 826. Geometry snapshots in `tests/snapshots/` are regenerated
  deliberately in the same change.
- `render/agent.rs`: transcript gets a 790px centered reading column, 29px avatars, speaker + time,
  markdown-ish blocks (heading/bullet/code), and a grouped tool-activity section with duration;
  composer collapses to attach + `@`, model selector, mode selector, send/stop icon at 826 max width.
- `render/pages.rs`: Preferences becomes a centered overlay window with a left section nav, leaving
  the shell (Explorer/Agent/Changes) visible behind it.

### 7. Git review surface (settled by the prototype)

The Changes column becomes a review surface rather than a file list plus patch dump.
`prototype-git-v1.3.html` is the visual and interaction contract for this slice; it is sample data
and executes no Git. `GIT_SURFACE_V1.3.md` lists every control, its demo transition, and the Host
verb matrix, and records the 73 interaction checks and screenshots that settled the composition.

- **Files**: `Staged / Changes / Untracked` groups, change-kind chip, `+N −M` per file (or `binary` /
  `untracked`), hover actions (stage, unstage, discard, delete untracked), selection checkboxes with
  a bulk bar, text filter.
- **Diff**: per-file header with stats and an Unstaged/Staged switch; hunk cards with old/new
  gutters; per-hunk stage, unstage and discard; binary and untracked states; horizontal scroll;
  Reviewed state keyed by path plus content signature, so it clears when the file changes.
- **Commit bar**: staged summary, message, recent messages, Amend (only when HEAD is local and
  something is staged), Sign-off, Commit, Commit & Push; disabled state names its reason.
- **History**: swimlane commit graph with stable lanes, merge curves and lane joins, refs chips,
  HEAD ring, 40px rows, `↑`/`↓` selection, commit detail with patch and parent navigation,
  cherry-pick/revert entries.
- **Operations**: in-progress merge/rebase banner with conflict chips, ours/theirs/base choice,
  Continue/Abort; every destructive action confirms with the path or hunk and carries
  `expectedStatus`.
- **Status bar**: branch menu (checkout, create, merge, rebase, stash) and sync menu (fetch, pull,
  push, push-and-set-upstream) with a push confirmation.

Host work this surface requires (the UI must not invent any of it):

| Capability | Verb | Change |
|---|---|---|
| ahead/behind, upstream, per-file stats, rename source | `git/status` | additive response fields |
| hunk stage/unstage/discard | `git/apply` | new; patch for one path, target `index`/`worktree`, checked before applying |
| commit amend, sign-off | `git/commit` | new request options |
| publish branch | `git/push` | `setUpstream` option |
| history graph, commit detail | `git/log`, `git/show` | new; `oid`, `parents`, `refs`, author, time, patch, paging |
| stash | `git/stash` | new; list/save/pop/apply/drop |
| conflict sides | `git/conflict`, `git/resolve` | new; read ours/theirs/base, resolve one path |
| delete untracked path | `git/discardUntracked` | new; explicitly refused today, needs a stricter confirmation |

Client work: `client-core/src/git.rs` types for every new field and verb, patch parsing and lane
layout on the client, and the GPUI components for the sections above. Lane layout stays client-side;
the Host never returns `--graph` ASCII.



### 8. Slices and order

| Slice | Content | Gate |
|---|---|---|
| **A1 ✅** | Theme model + built-in packs (`dsh`, `harness`, `high-contrast`, `solarized`) + resolved snapshot + `ThemeMode` fix + v6 migration + contrast tests | `cargo test --release`; light+dark native screenshots; prefs v5 file still loads and looks unchanged |
| **A2 ✅** | Personal pack store: add file, duplicate, export, editor writes full JSON, precedence + fallback notices | tests for precedence/lookup/fallback; edit a role and see it render |
| **A3 ✅** | GitHub import: fetch, validate, cache, provenance, explicit update check, offline behavior | tests with a local fixture server/fixture bytes; failure messages name the pointer |
| **B** ✅ | Shell chrome parity (top bar, workspace picker, Agent tab, status bar, Explorer header, metrics) | keyboard/focus/Esc tests; screenshots 1366×768 + 1920×1080 |
| **G0 ✅** | Host Git contract: `git/status` additions, `git/apply`, `git/commit` options, `git/push` upstream, `git/log`, `git/show`; `client-core` types | workspace-controller tests per verb, error codes, patch-fail and stale-status cases |
| **G1** ✅ | Files + diff: groups, kind chips, stats, bulk actions, hunk cards, per-hunk stage/unstage/discard, Reviewed, commit bar | unit tests for grouping/stats; interaction tests for stage/discard/commit; native screenshots both schemes |
| **G2** ✅ | History: graph layout, refs/HEAD, selection and detail, parent navigation, revert/cherry-pick wiring | layout unit tests for linear, merge, join and 3-lane histories; keyboard tests; screenshots |
| **G3** ✅ | Branch and remote workflow: ahead/behind, branch menu, fetch/pull/push, publish upstream | auth-failure, no-upstream and diverged-state tests; push confirmation |
| **G4** ✅ | Conflicts and stash: operation banner, ours/theirs/base, Continue/Abort, stash list/save/pop/drop, untracked delete | mid-operation state tests; destructive confirmation with `expectedStatus`; no optimistic completion |
| **C** ✅ | Composer + transcript + approvals | IME/Enter tests; before/after screenshots |
| **D** ✅ | Preferences overlay (Changes header/tabs now land in G1/G2) | destructive-confirmation and focus tests |

A1 must precede B, C, D, and G1: every surface resolves colours through the token snapshot.
G0 runs in parallel with A1 and blocks G1. C and D have no dependency on the Git slices.


## Alternatives considered

**Keep `ThemePack` as an enum and add one more variant.** Rejected: it cannot accept a user or GitHub
pack without recompiling, which is the stated requirement, and every new palette would grow the
preference schema.

**Let theme files carry metrics and component styling.** Rejected: a pack could then break hit
targets or the 30/34/26 shell, and the design system forbids per-component styling at call sites.
Colour roles only.

**Allow arbitrary remote URLs with automatic background updates.** Rejected: silent restyling, SSRF
surface and no user-visible provenance. Imports are explicit, HTTPS-only, size-capped, and updates
are user-triggered.

**Map `default` → `dsh` during migration.** Rejected: it would restyle every existing installation
without consent; `default` migrates to `harness`, and `dsh` is the new-install default.

**Parse `git log --graph` ASCII instead of laying out lanes.** Rejected: the decorated text is a
rendering, not data; column arithmetic breaks on padding, unicode widths and `--graph` changes.
The client lays out lanes from `parents` and `refs`, and the Host returns plain records.

**Keep whole-file staging only.** Rejected: it is the main review bottleneck the prototype exposes —
an agent edit and a human edit in one file cannot be separated. `git/apply` with a checked patch is
the smallest Host addition that fixes it without an interactive `git add -p`.

**Show History as a flat list.** Rejected: merge and rebase are operations the surface already
exposes, and a flat list hides which commit came from which branch. The graph keeps lane indexes
stable and costs one 38px gutter at the supported pane widths.

**Store themes in Host settings.** Rejected: appearance is client-local today, must work while the
Host is disconnected, and the Host has no theme contract.

## Acceptance criteria

- Selecting Dark renders dark on a light desktop; System follows the desktop; the Preferences preview
  and the rendered shell agree in all three modes.
- A pack added as a file under `<DSH_HOME>/themes/` appears in Preferences and renders without a
  rebuild, in both schemes and all four accent choices.
- A pack imported from a GitHub raw URL renders, shows its source/author/licence state, survives
  restart offline, and reports an explicit error (with JSON pointer) for an invalid document.
- An invalid or contrast-failing pack never renders partially styled: the shell keeps the last valid
  theme and shows the failure reason.
- `cargo test --release` passes; the v5 preference fixtures still load; geometry snapshots reflect
  the documented metrics.
- Native screenshots at 1366×768 and 1920×1080 exist for both schemes before any visual "done" claim.
- Git: a partially staged path appears under both Staged and Changes; staging one hunk leaves the
  other hunk unstaged; a stale diff refuses the hunk action and asks for a reload; commit clears only
  staged content; Amend is unavailable for an already-pushed HEAD; publish sets upstream; the graph
  renders a merge and a lane join without an orphan rail; untracked delete requires the stricter
  confirmation.
- Git: no UI state claims success before the Host acknowledges it; every mutation is disabled while
  one is in flight for the same repository.

## Risks

- **Contrast presets vs. reality:** the design-system light palette is close enough that `textSecondary`
  can fall below 4.5:1 on `surfaceRaised`; the gate may reject the proposed tokens and force a small
  palette adjustment — that must be resolved in `dsh`, not by weakening the gate.
- **Migration regressions:** v5 → v6 touches a durable user file; a botched mapping restyles or
  resets preferences. Mitigation: round-trip tests across every v5 shape and a backup copy before the
  first v6 write.
- **Remote supply chain:** a theme can make approved UI unreadable or impersonate another pack's
  identity. Mitigation: id namespace rules, provenance display, contrast gate, no auto-update.
- **Screenshot gates in this environment:** GPUI 0.2.2 panics on a Wayland compositor without a seat
  and cannot receive injected input here, so menu/interaction evidence still needs a real graphical
  session; dark-mode captures are impossible until the `ThemeMode` fix lands.
- **Scope creep:** B–D and G1–G4 change many call sites. Each slice must land with its own screenshots
  and keep the unimplemented remainder explicitly listed as `PARTIAL`.
- **Patch application correctness:** hunk actions send a patch that must apply cleanly; CRLF,
  whitespace-only changes and context drift are the likely first failures. Mitigation: `git apply
  --check` before applying, refuse with the Host's own message, and keep the dirty flag visible.
- **Stale review state:** a diff read before a concurrent edit can discard the wrong hunk. Mitigation:
  `expectedStatus` on every destructive path plus a content hash on the confirmed hunk.
- **Graph lane correctness:** merges, lane joins and octopus parents can render an orphan rail or a
  crossing dot. Mitigation: layout is a pure function with unit tests for linear, merge, join and
  three-lane histories; the renderer takes the same layout output.
- **Narrow pane budget:** at 485px the Changes column must hold graph, subject, refs and oid without
  collision. Mitigation: fixed graph gutter, truncating subject, refs capped to the row; verify at
  both supported widths before landing.
- **Host scope creep:** `git/log` paging, stash and conflict resolution are new Host surfaces. Land
  G0 per verb behind its own tests; never add a UI control before its verb exists.
