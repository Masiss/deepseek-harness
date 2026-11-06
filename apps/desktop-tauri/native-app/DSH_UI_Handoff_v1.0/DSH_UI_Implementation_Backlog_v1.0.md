# DSH UI V2 — Implementation Backlog v1.0

**Dùng cùng:** `DSH_UI_Functional_Spec_v1.0.md` (hợp đồng hành vi), `runtime-capability-map.md` (hiện trạng runtime) và `dsh-ui-prototype-v2.html` (visual reference).
**Cập nhật:** 06/11/2026 — Gate 0 hoàn tất; đối chiếu lại T01–T14 với code và test. Sửa T01/T05/T07/T08/T10/T12 cho khớp hiện trạng; chỉ ghi "Xong" khi gate của task đã đạt, phần thiếu nêu rõ.
**Quy tắc:** Không rewrite DSH runtime; không thay layout V2; không nối giả UI state thành backend success. Mỗi task phải có test cho đường thành công + lỗi + re-entry/reconnect nếu liên quan.

## 0. Gate trước coding — capability inventory

Đã hoàn tất ngày 2026-10-31, audit lại theo shell gpui ngày 2026-11-01. Deliverable: [`runtime-capability-map.md`](runtime-capability-map.md).

- [x] Xác định Desktop host/UI framework đang chạy; vị trí shell, command dispatcher, preferences.
- [x] Tìm API hiện có cho workspace/filesystem, session/history, run/events/tool approval, Git, PTY.
- [x] Tìm extension/skill registry có sẵn và cơ chế package installer (nếu có).
- [x] Ghi bảng `SUPPORTED / PARTIAL / NOT_SUPPORTED`, source code path và gap cần adapter.
- [x] Nêu rõ những gì cần hỏi user trước khi thay behavior rủi ro; **không tự quyết viết host mới**.

**Deliverable:** `runtime-capability-map.md`, đề xuất adapter tối thiểu, test hiện có. **Không thay code production** ở gate này.

## 1. Sprint theo vertical slices

Cột cuối là trạng thái đọc từ code ngày 2026-11-06, không phải tuyên bố DONE: một task chỉ DONE khi đạt gate của nó và mục 12 của spec.

| Task | Mức | Phụ thuộc | Phạm vi giao coding agent | Gate nghiệm thu | Trạng thái 2026-11-06 |
|---|---|---|---|---|---|
| T01 UI shell | P0 | Gate 0 | Chuyển token/layout V2 vào gpui, 30px/34px/26px, menu, panels; visual test | Không thêm Changes ở giữa, Terminal không có trong hamburger, Git ở bottom trái | Đã port (commit `e1a0346`): bỏ nav rail, Explorer trái, tab Agent 34px, Changes phải (Diff/Files thật từ T07), bộ chuyển Session bằng popover, bottom bar trái/giữa/phải, menu Workspace/Customize/Help + About, kéo vách Explorer/Changes có lưu; còn: snapshot pixel ở 1366×768/1920×1080 (chưa có display trong môi trường dev) |
| T02 Command registry | P0 | T01 | Chuẩn IDs, routes, menu, keymap dispatcher, popup focus/Esc | Một command dùng chung menu/shortcut/palette, không xung đột | Xong 04/11: 14 id một nguồn, Quick open có ↑/↓, Enter chạy mục đang chọn, lệnh Host `commands/list`/`commands/execute` đổ vào palette và slash-command trong composer được route qua Host; còn: preset keymap (KEY-01) thuộc T11 |
| T03 Workspace adapter | P0 | Gate 0, T01 | Open/switch workspace, list tree lazy, selection, preview, file events | AC-WS-01…05 và permission/missing-file tests | Một phần: tree lazy + preview + watcher; đã thêm lọc nhanh theo tên/đường dẫn trong cây đã tải; còn: switch workspace, mutate file và search nội dung bị chặn bởi Host RPC |
| T04 Session adapter | P0 | Gate 0, T03 | Create/list/switch, runtime history, drafts, restore, active run badge | AC-SES-01…05; đổi session không cancel run | Xong phần còn lại 04/11: draft composer theo Session lưu trong `native-ui-preferences.json` với debounce 750 ms, khôi phục khi quay lại; tùy chọn **Restore last Session** mở lại Session cuối, fallback Session mới nhất kèm lý do khi thiếu; badge **● Running** ở tab Agent lấy từ `session/list` và poll 3 giây khi có lượt chạy; còn: bỏ luồng live của Session cũ khi chuyển đi rồi quay lại chỉ tải snapshot runtime (chưa có cursor resume), đổi Session không cancel run |
| T05 Agent read path | P0 | T04 | Render message/tool events, reconnect/cursor, scroll/log caps | No duplicate/out-of-order event, tool errors thật | Một phần: transcript + snapshot + paging; dedupe theo `seq` đã có test (`merge_orders…`, `ignores_internal…`) và test snapshot+live frame đã bật lại 06/11 (`live_follow_frames…`); 06/11 sửa `run_state` theo `turn/end.reason.kind` (running/blocked/partial/interrupted/failed/completed/forked) thay cho cờ dính theo tool error, kèm test; còn: chưa có cursor resume, tool chưa có stdout/stderr/duration/exit code |
| T06 Composer send path | P0 | T05 | Model/mode availability, attach/@file, submit ack, cancel/approval | AC-AG-03…07; idempotency, IME | Đạt phần UI: model picker, 1 tệp đính kèm, cancel chờ ack, approval inline, Queue/Steer toggle + gửi `mode` thật (cả `send_prompt` và `queue_prompt`), nút `＠` duyệt file đã tải, ô lọc query nối thật (`file-picker-query` + `sync_inputs`), chèn `@<path>`, lịch sử prompt History/Next, test IME cho marked-text rendering (`ime_marked_text…`); còn: mode agent Ask/Plan/Build chưa có (Host không expose) |
| T07 Changes read path | P0 | T03 | Real status/diff, file list, review metadata invalidation | Diff đúng, no changes/conflict/binary/stale states | Đạt 05–06/11: `git/status` (staged/worktree), `git/diff` staged/unstaged, untracked qua `git diff --no-index`, cap `gitMaxDiffBytes` có cờ truncated; tab Diff có chuyển side, tab Files có badge trạng thái; conflict hiện là `U` và có banner operation; binary được nhận diện qua header `Binary files … differ` (`git_diff_is_binary` + test); tự refresh theo `workspaceFiles/changes` qua `GitUpdate::StatusAndState` + `workspace_changed_since` + test; còn: binary vẫn là heuristic tới khi Host expose cờ binary |
| T08 Changes destructive actions | P0 | T07 | Guarded discard với version/provenance và confirm | Không mất user edits; rollback/recovery được nếu supported | Đạt 05–06/11: `git/stage`, `git/unstage`, `git/discard`, `git/commit`; path được kiểm tra và phải đang thay đổi; discard từ chối untracked/added/renamed/copied/conflicted (`git/path-refused`) thay vì xoá; commit chỉ chạy khi có file staged; discard có hộp xác nhận (Gate C) với chốt stale; còn: chưa có version/provenance ngoài việc Host đọc lại status |
| T09 Terminal/PTY | P0 | Gate 0, T01 | Native terminal attach/input/resize/close lifecycle | Hide không kill; disconnected không hiện alive giả | Xong 05/11: tab/create/rename/close/attach/retain/resize, ANSI, output cap, lifecycle `idle/starting/attached/detached/error`, ô nhập gửi theo dòng, **Raw keys** gửi từng phím kể cả mũi tên/Home/End/PageUp/PageDown/Delete/F1–F12/Esc/Ctrl+C, tối đa 4 pane attach độc lập, kéo vạch đổi độ rộng, chọn dòng + Copy, test dropped-stream/detach giữ scrollback, probe reconnect Host thật; còn: probe không chạy trong CI keyless |
| T10 Git basics | P0 | T03, T07 | Repo detection, branch picker, guarded checkout, Git bottom-left | No-repo / dirty checkout / status refresh | Đạt 05–06/11: bottom bar hiện branch thật hoặc `No repository`, tab Branches list/checkout có lỗi thật, refresh sau thao tác; thêm GIT-06 fetch/pull(`--ff-only`)/push (không force, dùng credential helper/SSH agent của Host) và GIT-07 state/rebase/cherry-pick/continue/abort; checkout có hộp xác nhận (Gate C) và status tự refresh theo watcher |
| T11 Preferences native | P0 | T02 | Persistent appearance, keymap, panels, general | Reload/validation/migration; accessibility | Phần lớn: JSON `schemaVersion` 4, theme pack/density/font/keymap/panel width, restore last Session + draft per Session, keymap preset DSH/Zed/VS Code (gpui + egui, `apply_preset`); còn: override theo workspace |
| T12 Skills & extensions | P0–P1 | Gate 0, T11 | Read installed, scoped enable/disable, validation/ack, details | Enabled ≠ installed; không execute skill vô cớ | Đạt phần bundle: `skills/list` chỉ đọc; plugin bật/tắt có ack; bundle bật/tắt + Remove (uninstall qua `pluginManager/removeBundle`, có refresh inventory) đã nối ở cả shell gpui và egui; còn: thiếu quyền/health/changelog, skill enablement scope |
| T13 Marketplace | P1 | T12 | Trusted catalog/details, permissions, install/update/remove | Offline/fail/retry/verification; không success giả | Một phần: index/search/install lifecycle; chưa có update/uninstall, chưa có trường trust |
| T14 Enhanced workflows | P1 | T04,T06,T07,T09 | Fork, handoff, stage/commit, multi-terminal/detach khi supported | Không tự rewind filesystem, không auto-run side effects | Một phần: fork có chọn checkpoint (`Fork at seq (blank = latest)` + validate tại client + test; cả gpui lẫn egui, `at_seq` gửi qua RPC `session/fork`), Goals, stage/commit (T08), tối đa 4 pane terminal, detach UI; còn: handoff |

**Kiểm chứng 06/11:** native-app `cargo test` **177 pass** (khôi phục `#[test]` snapshot+live frame; thêm 3 test `run_state` theo `turn/end`, 1 test IME marked-text, 1 test fork checkpoint, file-picker filter + preset parity egui, watcher/confirm/binary từ các slice trước); client-core `cargo test` **120 pass** (101+8+11); `workspace-controller/tests/git.host.spec.ts` **72 pass** sau khi khai báo `@deepseek-ai/dsh-typert-registry` trong `devDependencies`. Chưa chạy được snapshot pixel (không có display) và probe Host thật trong CI keyless.

**Đồng bộ tài liệu:** `runtime-capability-map.md` đã cập nhật 06/11 theo các slice 05–06/11 (§§ 1/5/7/8: Changes/Git auto-refresh + confirm + binary, PREF-01 restore, KEY Host commands + palette arrows, EXT bundle toggle/remove, CMP Queue/Steer + history + `@` picker).

## 2. Gating strategy

### Gate A — Visual/interaction fidelity

- [ ] So sánh với V2 ở 1366×768 và 1920×1080; không thêm toolbar/cột phụ.
- [ ] Explorer trong sidebar; session controls cuối Explorer; cột giữa chỉ Agent; cột phải Changes.
- [ ] Top/right/bottom đúng nghĩa, không lặp branch/workspace/status.
- [ ] Focus/keyboard/IME, resize/hide/restore hoạt động.

### Gate B — Runtime truth

- [ ] Không còn code production phụ thuộc static demo workspace/session/diff/model.
- [ ] UI chỉ hiển thị `online/completed/reviewed/installed` khi có nguồn dữ liệu/canonical ack.
- [ ] Stop và detach rõ; không dùng đóng panel thay cho cancel run.
- [ ] Event cursor/idempotency chống trùng tool/prompt.

### Gate C — Safety & recovery

- [ ] Discard/checkout/file write có xác nhận + scope + stale version guard.
- [ ] Run interrupted/disconnected có lựa chọn recovery đúng capability; không chạy lại lệnh ngầm.
- [ ] Skill/extension có scope và quyền thực thi được runtime kiểm tra, không chỉ frontend.
- [ ] Marketplace không cài code chưa xác minh và không dùng demo toggles.

### Gate D — Performance/release

- [ ] Long logs/file tree/diff không gây freeze; đo p95 tương tác thông thường.
- [ ] Kiểm thử: restart, network loss, hidden panel, hibernate/resume khi hỗ trợ.
- [ ] Release notes nêu tính năng `supported`/`not supported` trung thực.

## 3. Format báo cáo mà coding agent phải trả về mỗi task

1. Đã thay đổi những file nào và vì sao.
2. Capability runtime nào được dùng, cái gì vẫn `NOT_SUPPORTED`.
3. Tests chạy và kết quả, screenshots/diffs nếu là UI.
4. Rủi ro chưa xử lý; công việc còn lại. Không báo “done” nếu chỉ hoàn thành UI mock.

## 4. Prompt giao việc ngắn (dùng từng task, không dồn một lượt)

**Bước inventory:**

> Đọc `DSH_UI_Functional_Spec_v1.0.md` và HTML V2. Audit codebase DSH cho workspace/session/agent stream/PTY/Git/settings/skill/extension. Lập `runtime-capability-map.md` kèm path chứng cứ và gaps. Không sửa implementation, không đề xuất rewrite host.

**Bước shell:**

> Implement T01–T02 theo V2 và backlog. Giữ nguyên bố cục, không thêm toolbar hay tab Changes giữa. Dùng command registry chung; kiểm tra keyboard/focus và so visual ở 1366×768, 1920×1080. Báo files/tests/gaps.

**Bước integration (mỗi lần một lát cắt):**

> Implement **T0X** theo `DSH_UI_Functional_Spec_v1.0.md` và backlog. Reuse DSH runtime có sẵn, giữ layout V2. Test success/error/reconnect; không mô phỏng success nếu thiếu capability. Báo changed files, tests, gaps.

---

**Ghi chú:** Backlog là thứ tự đề xuất và có thể chỉnh theo capability inventory; không nên ép chạy Marketplace trước khi xác nhận security/extension lifecycle hiện có.
