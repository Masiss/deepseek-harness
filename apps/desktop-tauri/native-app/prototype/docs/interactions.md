# Prototype interaction reference

This page records what the standalone HTML prototype lets a reviewer exercise and which state changes are only local demonstrations. It does not describe the shipped gpui app’s API contract.

## Screen inventory

| Screen or panel | Main interactions | Result and data source |
|---|---|---|
| Sessions and conversation | Search/select/rename/fork seeded Sessions; create a local Session; type and send; cancel; attach/remove files; choose a model and effort. | Session rows and transcript are DOM fixtures. Sending creates local transcript rows and a delayed canned reply; attachments contribute names only. |
| First-run onboarding | State-map cases for empty Sessions, Host disconnected, and no provider; create Session, retry, optionally open model picker. | State derives from `fx.sessions`, `fx.host`, and `fx.providers`; all transitions are local. |
| Workspace Explorer | Expand/collapse folders; select a file; navigate breadcrumbs; use keyboard tree navigation; refresh, retry, clear preview, or return to tree. | `FILES` and fixture line arrays provide excerpts and error cases; no repository or local-file read occurs. |
| Settings: Appearance | Change system/light/dark theme, Harness/Graphite/Slate preset, interface scale, code font, and density; Save or Revert. | Changes preview in the page and persist only in browser `localStorage`. |
| Settings: Layout | Show/hide Sessions and Explorer; adjust panel widths; Save or Revert. | Changes CSS grid inputs and local preference storage; no native window geometry is changed. |
| Settings: Shortcuts | Inspect sample bindings and click Edit. | Editing shows a prototype-only notice; there is no shortcut registration engine. |
| Settings: Host · Cordis | Edit a sample settings document; Save/Reload; trigger success, rejection, revision conflict, or unknown outcome; check/read back; rebase, discard, or copy. | A local revision fixture and selected scenario determine the display. No Host API is called and no settings file changes. |
| UI config draft | Edit JSON-with-comments, validate, Preview, Save locally, Reset, return to Settings. | Validation and preview update local preference state; Save only persists the sample preferences in `localStorage`. It does not write a file. |
| Document | Open the sample deliverable from the utility panel. | A static read-only sample and file details are shown. |
| Plugins | Refresh and open Details. | Notifications only; plugin/profile fixtures remain unchanged. |
| Skills | Search and select a sample skill. | Local filtering updates the detail rail. |
| Marketplace | Search sample entries and click Install. | The row changes to Installed locally; no package or profile operation occurs. |
| Goals | Edit a local goal; Save, Clear, or Refresh. | Text and notifications update only in the page. |
| Utility panel | Toggle Terminal, Jobs, Schedules, or Deliverables; close/clear; inspect a job; delete sample schedules; open a sample deliverable; enter a command. | All lists and outputs are fixtures. Terminal input is echoed with an explicit “command was not executed” message. |
| State map | Apply a named state, set the next simulated action outcome, play reconnect recovery/failure, reset fixtures, and preview a chosen state at wide/compact/narrow sizes. | The map mutates fixture state and embeds a fresh prototype document in a sandboxed `iframe`. |

## Fixture state model

The page starts with `fx = { sessions: 'has', host: 'connected', providers: 'ready', tree: 'ready', profile: 'set', hostSettings: 'loaded' }`. Its separate `scenario` object chooses the outcome of the next Host-settings save, unknown-outcome check, reconnect, profile-default write, and model-catalog retry.

| Area | States represented in the State map | Main transitions |
|---|---|---|
| Sessions | Seeded or none | Seeded → filter/select/create; none → onboarding; create restores a local empty Session. Only the Refactor settings loader row has a full sample transcript; other seeded rows show generic previews. |
| Host | Connected, disconnected, retrying, failed, recovered | Disconnect → timed retry → recovered or failed; failed state exposes retry. A running demo turn is interrupted locally when disconnect occurs. |
| Providers/catalog | Ready, loading, none, error | Retry consumes ready/error scenario; none remains no-provider. Composer is blocked when no model provider fixture is available. |
| Model profile default | Unset, initializing, set, saved, failure | Selecting a model changes local selection; profile action consumes success/failure scenario and displays local read-back. |
| Explorer tree | Ready, loading, empty, truncated, error, no workspace, no Session | Refresh consumes a timer; errors expose retry; file preview has its own state. |
| File preview | Excerpt, text, truncated, loading, missing, binary, unsupported, error, blocked, unavailable | Select/retry/clear changes fixture selection and rendering; outside-workspace path is shown as blocked and never opened. |
| Host settings | Loaded, loading, dirty, saving, checking, saved, failed, conflict, unknown | Save consumes success/conflict/failure/unknown; unknown outcome can be checked as applied or not applied. |

The state map’s next-action controls are `hostSave`, `unknownResolve`, `reconnect`, `profile`, and `catalog`. They are fixtures, not fault injection against a live service.

## Keyboard and focus behavior

- Enter sends the current prompt; Shift+Enter inserts a newline.
- Ctrl/Cmd+N creates a local Session; Ctrl/Cmd+E toggles the Explorer while the Session view is active.
- The model picker and state-map tabs support arrow navigation; Escape closes the model menu or mobile Explorer sheet.
- The Explorer tree uses Arrow Up/Down, Home/End, Arrow Left/Right, a roving tab stop, and a one-pane file view at narrower widths.
- Dialog close restores focus to the control that opened it or to the target fixture control.

## Responsive modes

| CSS viewport width | Intended layout |
|---|---|
| Above 1240 px | Navigation rail, Sessions, conversation, and Explorer dock. |
| 681–1240 px | Rail, Sessions, and conversation; Explorer is on demand and can replace the Sessions column. |
| 680 px and below | Rail and conversation; Sessions is hidden and Explorer opens as an overlay sheet with tree/file navigation. |

The State map demonstrates 1440×900, 1000×720, and 420×760 frames. UI scale changes the prototype frame, while CSS breakpoints use the browser viewport width.

## Simulated and persistent effects

| Operation | Observable effect | Persists or reaches a service? |
|---|---|---|
| Save appearance/layout preferences | Updates current UI and storage-status copy. | Writes preference keys to browser `localStorage` only. |
| Save a UI config draft | Validates and applies the selected values locally. | No file write and no Host call. |
| Send prompt/cancel turn | Adds local rows and a timer-driven canned response or cancellation message. | No model request or session event. |
| Attach a file | Shows/removes file-name chips. | Does not read or upload file bytes. |
| Save Host settings/profile default | Shows fixture revision/status/read-back. | No Host request or profile write. Copy draft may use the browser Clipboard API. |
| Install marketplace entry or execute terminal command | Changes button/output text. | Does not install a package or run a process. |
| Reconnect/refresh/retry | Shows loading, success, or error state according to fixtures. | No network request. |

`localStorage` is the only persistent storage used by the prototype. All other state returns to its defaults when the document reloads.
