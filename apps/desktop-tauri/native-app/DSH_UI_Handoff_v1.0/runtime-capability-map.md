# DSH Runtime Capability Map (Native Desktop)

**Trạng thái:** Audit read-only, không sửa implementation, không đề xuất rewrite host.
**Ngày audit:** 2026-11-01 (bản 2026-10-31 audit nhầm sang shell egui đã nghỉ; xem §0.1); cập nhật trạng thái 2026-11-05 sau các slice Git và Terminal.
**Nguồn:** `DSH_UI_Functional_Spec_v1.0.md`, `dsh-ui-prototype-v2.html`, codebase DSH tại `/home/masis/deepseek-harness`.
**Đối tượng:** shell ship thật **Rust + gpui** — `apps/desktop-tauri/native-app/src/gpui_main.rs` + `src/shell/` — gọi Host RPC qua `apps/desktop-tauri/client-core`.

## 0. Transport surface và shell đang chạy

### 0.1. Shell nào đang ship

`native-app/Cargo.toml` đặt bin `dsh-desktop` → `src/gpui_main.rs`; `src/main.rs` là bin `dsh-desktop-egui-legacy`, chỉ build khi bật feature `legacy-egui`. Vì vậy:

- **Đang ship:** `src/gpui_main.rs`, `src/shell/{app,mod,theme,widgets,text_input,terminal_state}.rs`, `src/shell/render/{mod,agent,explorer_panel,pages,sessions}.rs`.
- **Dùng chung, không phải legacy:** `command_registry.rs`, `explorer.rs`, `ui_preferences.rs`, `transcript.rs`, `terminal_ansi.rs`, `terminal_input.rs`, `document_preview.rs`, `question_card.rs`, `session_list.rs`, `host.rs`, `fonts.rs`, `dialog.rs`, `layout.rs`.
- **Legacy (chỉ egui):** `main.rs`, `settings_panel.rs`, `transcript_view.rs`, `ui_harness.rs` (egui_kittest + `tests/snapshots/*.png`). Đừng dùng các file này làm bằng chứng cho hành vi gpui.

### 0.2. RPC method quan sát được trong `client-core/src`

- Workspace/files: `workspace/follow`, `workspaceFiles/list`, `workspaceFiles/read`, `workspaceFiles/readBytes`, `workspaceFiles/stat`, `workspaceFiles/changes`.
- Session: `session/list`, `session/page`, `session/create`, `session/rename`, `session/fork`, `session/search`, `session/prompt`, `session/cancel`, `session/follow`, `session/control`, `session/modelCatalog`, `session/selectModel`, `session/initializeDefaultModel`.
- Agent/chat: `session/prompt`, `session/cancel`, `assistant-stream` (frame), `approval/request`, `userQuestions/attachWait`, `userQuestions/answer`.
- Lệnh người dùng: `commands/list`, `commands/execute`.
- Terminal: `terminal/environment`, `terminal/shells`, `terminal/list`, `terminal/create`, `terminal/rename`, `terminal/close`, `terminal/attach`, `terminal/retain`.
- Settings: `settings/describe`, `settings/update`, `settings/replace`, `settings/mutate`, `settings/openSettingsDocument`.
- Skills: `skills/list`.
- Plugin/extension/bundle: `pluginInventory/list`, `pluginManager/listPlugins`, `pluginManager/listBundles`, `pluginManager/setPluginEnabled`, `pluginManager/setBundleEnabled`, `pluginManager/removeBundle`, `pluginManager/inspect`, `pluginManager/installBundle`, `pluginManager/waitForInstall`, `pluginManager/cancelInstall`.
- Marketplace: `pluginMarket/listIndexes`, `pluginMarket/search` (cùng nhóm `pluginManager/install*` ở trên).
- Ngoài phạm vi map: `goals/*`, `schedule/*`, `job/*`, `messageFeedback/*`, `officeToPdf/render`, `account/*`, `remoteAccess/follow`.

Ghi chú transport quan trọng cho §3.3 và §7.3 của spec:

- `session/follow` có `fromSeq` resume cursor (RPC phase 06/11): snapshot chỉ mang event sau sequence client đã giữ, trong cùng cửa sổ `max_messages`; `hasMore` đánh dấu khoảng trống điền bằng `session/page`. Dedupe theo `seq` vẫn là lưới thứ hai.
- `session/page` phân trang lịch sử bằng `throughSeq` + `beforeSeq` → đây là đường tải lịch sử cũ.
- `session/control` là stream projection: frame `baseline` (mọi Session, `asOfSeq`) rồi `projection` delta có `seq` — dùng để resync trạng thái Session, không phải lệnh.

## 1. Bố cục shell gpui so với V2

Đã port ngày 2026-11-02 (T01): bỏ Activity Bar, Explorer sang trái, Changes sang phải, thêm tab Agent và bộ chuyển Session.

| Vùng V2 | Shell gpui hiện có | Kết luận |
|---|---|---|
| Top bar 30px: hamburger, workspace selector, search, ẩn/hiện Changes | Hamburger menu (Workspace / Customize / Help gồm About DSH), chip workspace (tên thư mục `cwd` của Session, mở bộ chuyển Session), tiêu đề căn giữa, nút Search, nút Changes | Đạt; chip workspace là bộ chuyển Session vì Host chưa có workspace registry RPC |
| Không có Activity Bar | Đã bỏ nav rail; Preferences / Extensions / Skills / Marketplace / Goals / Open file mở từ menu hamburger | Đạt |
| Explorer cột trái | Dock trái, kéo vách 4 px để đổi rộng (`workspace_panel_width`, lưu khi thả) | Đạt; chưa có double-click reset về mặc định |
| Tab Agent 34px thuộc cột giữa | Dải tab 34px với tab `Agent` gạch chân accent + tiêu đề Session | Đạt một phần: chưa có nhiều buffer |
| Không có danh sách Session thường trực; `+` và dropdown ở đáy Explorer | Popover Session (search/rename/fork) mở từ chip top bar, nút Search, hoặc footer Explorer có `＋` và tên Session | Đạt |
| Changes cột phải: Diff/Files | Cột phải có tab Diff/Files, kéo vách để đổi rộng (`changes_panel_width`), đóng bằng `×`; nội dung là empty state nói rõ Host chưa expose diff | Đạt khung; dữ liệu là `NOT_SUPPORTED` cho tới khi có RPC |
| Bottom bar: trái branch + Git + sync; phải terminal + diagnostics + dot | Trái: `⎇ No repository`, `Git`, `Sync` (mờ, không có capability); phải: Jobs / Schedules / Deliverables / Terminal dạng icon + chấm kết nối | Đạt một phần: không hiển thị branch giả, nhưng Git panel chưa tồn tại và diagnostics chưa có |
| Preferences một cửa sổ, sidebar General/Appearance/Keymap/Extensions/Skills/Marketplace/About | Vẫn là trang full-pane; Extensions/Skills/Marketplace/Goals là trang riêng | Một phần: chưa gộp thành một cửa sổ Preferences |

## 2. Workspace & Explorer

| Spec ID | Host RPC | Shell gpui | Gaps |
|---|---|---|---|
| WS-01 | Registry là TS `packages/workspace/workspace` (`WorkspaceRegistry`); client có `workspace/follow` + create/rename/delete/insert/pin/archive mutations | Workspace selector từ `workspace/follow` (lọc inventory theo membership, session mới tạo với `workspace_id` đang chọn); workspace vẫn suy từ Session khi chưa có projection | Open Folder (native picker → Host `/workspace` mới) cần RPC; recent workspaces chưa có |
| WS-02 | Như trên | Chuyển Session không gọi `session/cancel` nên job cũ không bị hủy ngầm; draft composer được lưu riêng cho từng Session và khôi phục khi quay lại | Chưa giữ luồng live của Session cũ; quay lại tải snapshot runtime |
| WS-03 | `workspaceFiles/list` trả children trực tiếp + `truncated` | `explorer.rs`: lazy expand, state theo Session (`select_session`) | Không có đệ quy; không phân loại symlink/permission |
| WS-04 | `workspaceFiles/read` (text + offset/limit/eof), `readBytes`, `stat` | Preview text có phân trang, ảnh, tài liệu Office qua `officeToPdf/render` | Không có "open externally"; binary do client tự đoán |
| WS-05 | `workspaceFiles/search` (RPC mới 06/11, có giới hạn) | Ô Search content trong Sessions panel; click kết quả mở document | Quick-open theo tên trong Explorer chưa có; match chỉ tới dòng, chưa jump tới dòng |
| WS-06 | Không có RPC; `ui_preferences.rs` lưu `workspace_panel_width`, `show_workspace_explorer` | Resize/toggle có, lưu file JSON client-local có `schemaVersion` | Chưa có precedence global → workspace của spec |
| WS-07 | `workspaceFiles/changes` → frame `Ready` hoặc `Change {absolutePath, version, absent}`; watch theo **một target** mỗi stream | `explorer.rs`: mỗi thư mục đang mở và file đang preview có một watch; version lặp bị bỏ, thư mục đổi thì list lại và prune entry đã mất, file đổi thì banner stale, file mất thì banner absent (giữ trang cuối) | Frame chỉ nói target đó đổi, không nói entry con nào và không có rename; watch bị Host từ chối thì báo một lần và chỉ thử lại khi bấm refresh |
| WS-08 | `workspaceFiles/write` (stale-version guard + create-if-absent) | Ghi nội dung qua document editor + Save to Host (cả gpui lẫn egui) | Rename/delete file chưa có RPC |
| WS-09 | `workspaceFiles/search` (RPC mới 06/11, có giới hạn) | Như WS-05 | Bỏ qua symlink, NUL binary và file >1MiB; line không jump |
| WS-10 | — | — | Ngoài baseline |

Sandbox/permission: `packages/fs/fs-sandbox/src/containment.ts`, `packages/fs/fs-local/src/fsio.ts`; symlink qua `packages/workspace/workspace/src/paths.ts` (`realpathNormalize`).

## 3. Session Management

| Spec ID | Host RPC | Shell gpui | Gaps |
|---|---|---|---|
| SES-01 | `session/create` (`workspace_id`, `cwd`, `session_id`, `agent_preset`) | `new_session()` tạo và focus composer | Ổn |
| SES-02 | `session/list` (cursor), `session/search` (chỉ trả `sessionId` + snippet); `workspace/follow` cho workspace + archived/pinned set | Panel Sessions có search/rename/fork/archive/pin; chấm chạy cập nhật bằng poll 3 giây khi có lượt hoạt động; lọc theo workspace đang chọn bằng membership từ Host | Membership/archived/pinned đến từ stream, không qua `session/list` |
| SES-03 | `session/page` (`throughSeq`, `beforeSeq`, `maxMessages`, `turnWindow`), `session/follow` snapshot | `load_older_session_page()`, scroll anchor client-local; draft theo Session trong `ui_preferences` | Ổn; chưa chứng minh bằng snapshot test cho paging |
| SES-04 | Không có bản ghi "last session" phía Host | `ui_preferences` lưu `lastSessionId`; **Restore last Session** mở lại khi khởi động, fallback Session mới nhất kèm lý do nếu Session cũ không còn | Chỉ lưu phía client; workspace đi theo `cwd` của Session |
| SES-05 | `session/follow` resume qua `fromSeq`; `session/cancel` chỉ khi user bấm Stop | Đổi Session không cancel; `send_prompt` mở luồng follow riêng và resume từ `Transcript::last_seq`; badge **● Running** đọc `session/list` | Session đổi đi rồi quay lại (khác session) vẫn snapshot mới vì transcript reset; resume chỉ khi giữ view, nhất là reconnect |
| SES-06 | `session/rename` có; `workspace/archiveSession` + `unarchiveSession` đã có ở Host | Rename + Archive/Archive&stop/Unarchive + Pin/Unpin (cả gpui lẫn egui, `stopActivity` tách riêng hai nút) | Hard delete chưa có RPC — archive là đường an toàn được hỗ trợ |
| SES-07 | `session/fork` (`at_seq`, `allow_migration`) | Ô `Fork at seq (blank = latest)` + validate tại client + test; `at_seq` gửi qua RPC (cả gpui lẫn egui) | Chọn checkpoint được; không reset filesystem |
| SES-08 | `session/search` | Có | Không phân trang/lọc trạng thái |
| SES-09 | — | — | P2 |

## 4. Agent, Composer, Tool Activity & Approvals

| Spec ID | Host RPC | Shell gpui | Gaps |
|---|---|---|---|
| AG-01 | `session/page`, `session/follow` (`SessionWireEvent {type, seq, time, data}`) | `transcript.rs` dựng row theo event type | Map role nằm ở client |
| AG-02 | `session/follow` với `assistant_stream`, frame `assistant-stream` | Ghép delta vào `transcript` | Dedupe `runId+seq` chưa có test chống lặp frame |
| AG-03 | Nội dung nằm trong event data | Markdown/code block tự vẽ; preview ảnh/tài liệu có | Chưa có sanitize URL/protocol tập trung; link `path:line` chưa điều hướng |
| AG-04 | `session/page` trả `ToolCall {call_id, name, status}` | Tool activity nhóm, mở rộng được | Không có stdout/stderr/duration/exit code trong client type |
| AG-05 | `ToolCallStatus` = `Pending / Succeeded / Failed / Unknown` | Hiển thị theo enum | Thiếu `Running / Cancelled`; client phải đặt `Unknown` khi mất kết nối |
| AG-06 | `PromptMode` chỉ có `Queue / Steer`; không có mode Ask/Plan/Build | `send_prompt` cố định `PromptMode::Queue` | Không có UI mode; cần xác nhận Host có mode agent hay không |
| AG-07 | `session/cancel` trả `AcceptedResponse {accepted}` | `cancel_turn()` chờ ack rồi mới đổi trạng thái | Ổn |
| AG-08 | `approval/request` (`toolName`, `callId?`, `reason?`, `displayReason?`) + `ApprovalDecision` = `allowed-once / rejected` | Hàng đợi approval inline, hiện request đầu + số còn lại | Không có `requestId`/expiry/scope trong payload; chỉ 2 quyết định |
| AG-09 | Không có `run.status` RPC; trạng thái suy ra từ event | Chưa phân biệt `partial/blocked` với `completed` | Cần event turn-outline hoặc luật suy ra rõ ràng |
| AG-10 | Không có event thinking trong client type | Không render | Đúng hướng: không bịa |
| AG-11 | `packages/session/session-turn-outline`, `packages/goal` | Goals có UI riêng | Handoff/summary chưa nối |
| AG-12 | Không có queue/parallel RPC | Không có | Phải coi là không hỗ trợ |
| CMP-01 | — | `composer_input.rs` + `shell/text_input.rs` (IME, selection, multiline) | Chưa có test composition tiếng Việt |
| CMP-02 | — | Nút `＠` mở danh sách file từ Explorer đã tải kèm ô lọc theo tên/đường dẫn (`file-picker-query`); chọn file chèn `@<path>` vào composer | Duyệt + lọc trong cây đã tải; chưa tìm trong workspace |
| CMP-03 | `upload.rs` (upload + receipt), `officeToPdf` | `pick_attachment` / `attach_path` / `poll_attachment` | Một tệp/lần, chưa có drop |
| CMP-04 | `session/modelCatalog`, `session/selectModel`, `session/initializeDefaultModel` | Picker + nhãn lý do không chọn được | Ổn |
| CMP-05 | Không có capability mode | Không có | `NOT_SUPPORTED` cho tới khi Host có |
| CMP-06 | `PromptRequest.request_id` (client sinh qua `chat.next_request_id()`) | Sinh id mỗi lần gửi, chặn khi `chat_loading` | Chưa xác nhận Host dedupe theo `request_id` |
| CMP-07 | `PromptMode::Queue / Steer` | Hai nút Queue/Steer trong composer; `send_prompt`/`queue_prompt` gửi `mode` hiện tại | Đạt; mode agent Ask/Plan/Build vẫn chưa có (Host không expose) |
| CMP-08 | Không có | `chat_text` là draft theo Session, lưu vào `sessionDrafts` của preferences và xoá khi gửi; lịch sử prompt có nút History/Next | Đạt |
| EV-01..03 | `session/follow` (`fromSeq` resume cursor), `session/control`, `session/page` | Snapshot resume từ `Transcript::last_seq` sau giữ view (reconnect) và lúc submit prompt; dedupe theo `seq` vẫn là lưới thứ hai | Resume có giới hạn bởi cửa sổ `maxMessages` như snapshot thường; khoảng trống giữa được điền bằng `session/page` |

## 5. Changes, Review & Git

| Spec ID | Host RPC | Shell gpui | Gaps |
|---|---|---|---|
| CHG-01..03 | `git/status`, `git/diff`, `workspaceFiles/changes` | Tab Changes → Files liệt kê file thay đổi; tab Diff đọc patch staged/unstaged | Chưa có review comment/inline annotation |
| CHG-04..07 | `git/diff`, `git/stage`, `git/unstage`, `git/discard` | Nút Stage/Unstage/Discard cho file đang chọn; Discard có hộp xác nhận với chốt stale trước khi chạy; patch được nhận diện binary qua header `Binary files … differ` và hiển thị chú thích thay cho diff text | Discard từ chối untracked/conflict thay vì tự xoá hoặc resolve; binary vẫn là heuristic cho tới khi Host expose cờ binary |
| CHG-08..10 | `git/commit` | Ô message + Commit trong tab Files | Chưa có amend, sign-off, hook UI |
| GIT-01..02 | `git/status`, `git/branches`, `git/checkout` (scope theo Session) | Bottom bar hiện branch thật hoặc `No repository`; tab Changes → Branches liệt kê và checkout; checkout có hộp xác nhận chụp trạng thái stale (branch, target, commit, danh sách file đổi) trước khi chạy | Checkout khi dirty trả `git/checkout-refused` kèm stderr của git; UI hiện lỗi, không tự overwrite file |
| GIT-03..04 | `git/status`, `git/state`, `workspaceFiles/changes` | Git icon mở Changes; status + state lấy chung trong một worker (`GitUpdate::StatusAndState`); tự refresh khi Explorer watcher báo thay đổi (`workspace_changed_since`) và sau checkout/mutation/commit, kèm nút Refresh | Đạt; watcher dedupe theo version nên thay đổi ngoài cây đã tải không kích hoạt refresh |
| GIT-05 | `git/diff`, `git/stage`, `git/unstage`, `git/discard`, `git/commit` | Diff staged/unstaged, tab Files có badge, Stage/Unstage/Discard, message + Commit; commit chỉ khi có file staged | Discard từ chối untracked/added/renamed/copied/conflicted; không có auto-resolve conflict |
| GIT-06 | `git/fetch`, `git/pull` (`--ff-only`), `git/push` (không force) | Tab Branches có Fetch / Pull / Push; Push cần xác nhận hai bước; lỗi auth/diverged/no-upstream hiện nguyên văn | Dùng credential helper/SSH agent của Host; không có cấu hình credential riêng trong DSH |
| GIT-07 | `git/state`, `git/rebase`, `git/cherryPick`, `git/continueOperation`, `git/abortOperation` | Banner operation + conflict list, Continue/Abort; mỗi branch có Rebase và Pick | Không có interactive rebase, merge/revert continue hay auto-resolve; người dùng sửa file, stage rồi Continue |

Kết luận: Git đã có đủ read path, diff, stage/unstage, discard, commit, remote sync và rebase/cherry-pick cơ bản cho client native. Các phần còn thiếu là interactive rebase và công cụ resolve conflict tự động.

## 6. Terminal & PTY

| Spec ID | Host RPC | Shell gpui | Gaps |
|---|---|---|---|
| TER-01 | `terminal/attach`, `terminal/retain` | Panel trong layout, ẩn/hiện không kill | Ổn (giữ process là hành vi Host) |
| TER-02 | `terminal/create` (`id`, `cols`, `rows`, `shellPath` — **không có cwd**), `terminal/environment`, `terminal/attach`, `TerminalInput::Write/Resize` | `shell/terminal_state.rs`: attach, resize, input queue, ANSI parse | cwd lấy từ `terminal/environment` theo Session, không truyền khi tạo |
| TER-03 | `TerminalState` = `Running / Exited / Failed` | Nhãn lifecycle `idle / starting / attached / detached / error`, cộng state và exit code của Host | `disconnected` gộp vào `detached` kèm dòng trạng thái stream |
| TER-04 | `TerminalEnvironment {maxInputBytes, maxBufferedBytes, maxCols, maxRows, scrollback}` | ANSI, input mode, giới hạn 128 KiB output hiển thị, ô nhập gửi theo dòng hoặc **Raw keys** gửi ký tự, Enter, Backspace, Delete, ↑/↓/←/→, Home/End, PageUp/PageDown, Insert, F1–F12, Esc, Ctrl+C khi không bôi đen; chọn dòng (Shift để chọn khoảng) rồi **Copy**; bracketed paste | Tab và các tổ hợp Ctrl/Alt khác do giao diện xử lý nên không đi tới terminal; chọn theo dòng, chưa chọn theo cột/ký tự |
| TER-05 | `terminal/list` theo Session | Tab nhiều terminal, create/rename/close | Chính sách per-workspace chưa có |
| TER-06 | `terminal/list`, `terminal/create`, `terminal/rename`, `terminal/close` | Tab + rename/close + tối đa 4 pane, mỗi pane attach/close/rename riêng; thêm/bớt pane theo pane đang focus; kéo vạch giữa hai pane để đổi độ rộng | Giới hạn 4 pane và chỉ trong cùng Session; độ rộng không lưu qua lần chạy sau |
| TER-07 | `terminal/retain` giữ PTY khi client ngắt | `detach()` gửi stop cho worker, không đóng PTY; screen giữ lại; test state cho dropped stream và detach/attach lại; probe `DSH_DESKTOP_PROBE_TERMINAL=1` báo `terminal-reconnect=ok/failed` sau khi ngắt và attach lại | Probe cần Host/PTY thật, không chạy trong CI keyless |
| TER-08 | `packages/ssh` không expose qua RPC | Không có | P2 |

Ranh giới: shell tool của agent (`packages/shell`, `packages/subprocess`) tách khỏi PTY người dùng; không đưa lệnh agent vào terminal của user.

## 7. Preferences, Extensions & Skills

| Spec ID | Host RPC | Shell gpui | Gaps |
|---|---|---|---|
| PREF-01 | Không có RPC; client-local | Tùy chọn Restore last Session trong preferences mở lại Session cuối, fallback Session mới nhất kèm lý do; draft composer theo Session lưu debounce 750 ms | Đạt |
| PREF-02 | Không có RPC cho layout UI | `ui_preferences.rs` lưu width/visibility panel | Client-local, chưa có override theo workspace |
| PREF-03 | `settings/*` với `expected_revision`, `SettingsConflictError`, redact (`packages/settings/settings`) | Preferences ghi JSON có `schemaVersion`, hiển thị lỗi load | Hai kho khác nhau: Host settings (plugin config) vs file UI client-local; cần nói rõ trong UI |
| PREF-04 | Không có export/import | Có sửa JSON trực tiếp trong trang Preferences | Thiếu reset theo nhóm |
| APP-01..04 | Không có | `ThemeMode` (System/Light/Dark), `ThemePack` (Default/HighContrast/Solarized/Custom), `UiDensity`, `ui_scale`, font UI/mono | Không có bộ Dark/Midnight/Light + accent Blue/Teal/Violet đúng như V2; accent chỉ đổi qua `ThemePack`/custom |
| APP-05 | Không có | `ThemeMode::System` + chọn font family/syntax | Thiếu syntax theme riêng |
| KEY-01..04 | **`commands/list` + `commands/execute`** đã tồn tại (Host human-command registry) | `command_registry.rs` 14 id là nguồn duy nhất cho menu, palette và dispatcher; menu dựng từ registry và hiện phím hiện hành; Quick open (Ctrl+P) lọc theo tên/nhóm, `↑/↓` chọn, Enter chạy mục đang chọn; lệnh của Host đã đổ vào palette và slash-command trong composer được route qua Host; Esc đóng lớp phủ trên cùng; `Keybindings` 4 phím tùy chỉnh, validate trùng phím trên toàn bộ id; keymap preset DSH/Zed/VS Code ở cả gpui lẫn egui (`apply_preset`) | Đạt KEY-01/03 và UX-02 |
| EXT-01..03 | `pluginInventory/list`, `pluginManager/*` (`listPlugins`, `setPluginEnabled`, `setBundleEnabled`, `removeBundle`, `inspect`) | Trang Plugins có danh sách + bật/tắt plugin (`setPluginEnabled`), bundle (Enable/Disable/Remove) ở cả hai shell, và mục **HEALTH** đếm fiber active/failed từ `listPlugins` | Đạt health ở mức fiber; quyền/changelog chưa có nguồn dữ liệu |
| EXT-04 | `ChangeResult {application, stage, restartReason, error, warnings}` | Hiển thị lỗi/restart | Chưa có test cô lập lỗi extension |
| SKL-01..03 | `skills/list` → `{path?, name, description, whenToUse?, modelInvocable, source?, provider?, disabledSkills[]}`; `skills/setDisabled` → `{disabledSkills[], persisted}` | Trang Skills đọc danh sách, hiện scope/provider/invocation, và Disable/Enable từng skill (disabled section) ở cả hai shell | Registry lọc trong `collect()` nên model catalog + `/skill` cùng tôn trọng; disablement persist theo settings namespace của entry `skill`; chưa có version/changelog |
| SKL-04/05 | Không có | `commands/execute` mở đường cho `/skill` nếu Host đăng ký | Chưa nối |
| MKT-01..05 | `pluginMarket/listIndexes`, `pluginMarket/search`, `pluginManager/*` (`inspect`, `installBundle`, `waitForInstall`, `cancelInstall`, `removeBundle`, `setBundleEnabled`) | Trang Marketplace có index/search; mỗi dòng hiện trust facts Host trả về (author, host repo, installation status/version, compatibility); Install khi `not-installed`; **Update** = remove → inspect → install → bật lại enablement cũ; Remove ở trang Plugins | Index format không khai báo permissions/verified; không có rollback version cũ nên thất bại giữa chừng chỉ được báo trung thực |

## 8. Tổng hợp khoảng trống (cập nhật 2026-11-06)

Chặn một UI trung thực theo spec:

1. Search nội dung đã có (`workspaceFiles/search` + UI, WS-09); quick-open theo tên trong Explorer chưa có (nút Search chỉ đổi view) → WS-05; `workspaceFiles/write` đã nối UI (document editor + Save to Host, version guard) ở cả gpui lẫn egui.
2. Git P0 đã có `git/status`, `git/diff`, `git/stage`, `git/unstage`, `git/discard`, `git/commit`, `git/branches`, `git/checkout`; GIT-06/07 có fetch/pull/push và state/rebase/cherry-pick/continue/abort. Còn: interactive rebase, merge/revert continue, auto-resolve conflict.
3. Session archive/unarchive + pin đã có (SES-06); hard delete chưa có RPC và cố tình chưa giả lập.
4. Không có mode agent Ask/Plan/Build → AG-06; `run.status` là suy ra từ `turn/end` (Queue/Steer + queue khi busy đã có) → AG-09, AG-12.
5. Skill chỉ đọc danh sách, nay kèm scope/provider → SKL-01/02/03 (enable/disable bền vững cần quyết định Host).
6. Host `TerminalState` chỉ có 3 giá trị `Running/Exited/Failed`; shell tự suy ra `idle/starting/attached/detached/error` từ attachment và lỗi → TER-03.
7. Last-session restore đã có client-local (PREF-01); layout/theme/keymap vẫn client-local, chưa có override theo workspace → PREF-02, APP, KEY (một phần).
8. `session/follow` đã có cursor `fromSeq`; reconnect giữ view resume từ sequence đã giữ, fallback snapshot mới khi đổi session.
9. Tool result đã tách stdout/stderr + exit code + duration từ log (không thêm RPC) → AG-04.
10. Approval chỉ có `allowed-once`/`rejected`, không có expiry/scope → AG-08 (một phần).

Nối được ngay, không cần Host mới: đọc/ghi file trong workspace (list/stat/read/readBytes/write/search), watch (`workspaceFiles/changes`, đã nối cho thư mục mở, file đang preview và Git auto-refresh), Session create/list/page/fork/rename/search/cancel/prompt, model catalog/select, attach/upload tệp, approval + câu hỏi, terminal create/list/attach/rename/close/retain/resize, toàn bộ Git namespace, workspace create/archive/pin, plugin bật/tắt + bundle toggle/uninstall, marketplace inspect/install/update lifecycle, `settings/*` với revision conflict, `commands/list` và `commands/execute`.

## 9. Ghi chú

- Mọi kết luận trên có đường dẫn chứng cứ; không sửa code.
- "Không có trong client" = không expose qua Host RPC ở `client-core`; Host TS package có thể có capability nội bộ chưa mở cho desktop.
- Theo spec §11: adapter chưa có phải báo `NOT_SUPPORTED` kèm fallback trung thực, không mô phỏng thành công.
- Bản đồ này phải cập nhật ngày khi RPC hoặc shell gpui đổi; `USER_GUIDE.md`/`USER_FLOWS.md` vẫn là mô tả ứng dụng đang chạy, không phải đích đến.
