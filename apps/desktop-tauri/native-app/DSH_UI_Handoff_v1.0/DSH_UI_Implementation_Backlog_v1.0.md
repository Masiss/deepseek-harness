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
| T03 Workspace adapter | P0 | Gate 0, T01 | Open/switch workspace, list tree lazy, selection, preview, file events | AC-WS-01…05 và permission/missing-file tests | Đạt 06/11: workspace selector từ `workspace/follow` (lọc theo membership, session mới tạo với `workspace_id`), Open Folder (picker → `workspace/create` → chọn + tạo session), ghi file qua document editor + Save to Host với version guard, search nội dung qua RPC `workspaceFiles/search` mới (bounded scan, bỏ symlink/binary/file lớn), quick-open file đã tải trong cây từ palette (`PaletteEntry::File`); tree lazy + preview + watcher |
| T04 Session adapter | P0 | Gate 0, T03 | Create/list/switch, runtime history, drafts, restore, active run badge | AC-SES-01…05; đổi session không cancel run | Đạt 06/11: draft composer theo Session lưu debounce 750 ms, khôi phục khi quay lại; **Restore last Session** mở lại Session cuối, fallback Session mới nhất kèm lý do khi thiếu; badge **● Running** ở tab Agent lấy từ `session/list` và poll 3 giây khi có lượt chạy; archive/unarchive + pin; cache tối đa 6 transcript theo session để quay lại resume qua cursor `fromSeq` thay vì snapshot mới; đổi Session không cancel run |
| T05 Agent read path | P0 | T04 | Render message/tool events, reconnect/cursor, scroll/log caps | No duplicate/out-of-order event, tool errors thật | Đạt 06/11: Host `session/follow` thêm `fromSeq` (snapshot chỉ event sau sequence đã giữ trong cùng cửa sổ `maxMessages`, `hasMore` đánh dấu khoảng trống điền bằng `session/page`), native resume từ `Transcript::last_seq` khi giữ view; dedupe theo `seq` + test snapshot+live frame + `run_state` theo `turn/end.reason.kind` (running/blocked/partial/interrupted/failed/completed/forked) kèm test; tool projection có stdout/stderr tách ngang, exit code và duration từ ghép call→result (không thêm RPC) |
| T06 Composer send path | P0 | T05 | Model/mode availability, attach/@file, submit ack, cancel/approval | AC-AG-03…07; idempotency, IME | Đạt 06/11: model picker, 1 tệp đính kèm, cancel chờ ack, approval inline, Queue/Steer toggle + gửi `mode` thật, nút `＠` duyệt file đã tải kèm ô lọc, chèn `@<path>`, lịch sử prompt History/Next, test IME cho marked-text rendering; mode selector theo runtime capability: nút Plan toggle qua lệnh `/plan` Host, `plan/mode` được fold vào `Transcript`, Ask disabled kèm giải thích là mode không có trong provider/profile (theo CMP-05 của spec) |
| T07 Changes read path | P0 | T03 | Real status/diff, file list, review metadata invalidation | Diff đúng, no changes/conflict/binary/stale states | Đạt 05–06/11: `git/status` (staged/worktree), `git/diff` staged/unstaged, untracked qua `git diff --no-index`, cap `gitMaxDiffBytes` có cờ truncated; tab Diff có chuyển side, tab Files có badge trạng thái; conflict hiện là `U` và có banner operation; binary được nhận diện qua header `Binary files … differ` (`git_diff_is_binary` + test); tự refresh theo `workspaceFiles/changes` qua `GitUpdate::StatusAndState` + `workspace_changed_since` + test; còn: binary vẫn là heuristic tới khi Host expose cờ binary |
| T08 Changes destructive actions | P0 | T07 | Guarded discard với version/provenance và confirm | Không mất user edits; rollback/recovery được nếu supported | Đạt 05–06/11: `git/stage`, `git/unstage`, `git/discard`, `git/commit`; path được kiểm tra và phải đang thay đổi; discard từ chối untracked/added/renamed/copied/conflicted (`git/path-refused`) thay vì xoá; commit chỉ chạy khi có file staged; discard có hộp xác nhận (Gate C) với chốt stale; còn: chưa có version/provenance ngoài việc Host đọc lại status |
| T09 Terminal/PTY | P0 | Gate 0, T01 | Native terminal attach/input/resize/close lifecycle | Hide không kill; disconnected không hiện alive giả | Xong 05/11: tab/create/rename/close/attach/retain/resize, ANSI, output cap, lifecycle `idle/starting/attached/detached/error`, ô nhập gửi theo dòng, **Raw keys** gửi từng phím kể cả mũi tên/Home/End/PageUp/PageDown/Delete/F1–F12/Esc/Ctrl+C, tối đa 4 pane attach độc lập, kéo vạch đổi độ rộng, chọn dòng + Copy, test dropped-stream/detach giữ scrollback, probe reconnect Host thật; còn: probe không chạy trong CI keyless |
| T10 Git basics | P0 | T03, T07 | Repo detection, branch picker, guarded checkout, Git bottom-left | No-repo / dirty checkout / status refresh | Đạt 05–06/11: bottom bar hiện branch thật hoặc `No repository`, tab Branches list/checkout có lỗi thật, refresh sau thao tác; thêm GIT-06 fetch/pull(`--ff-only`)/push (không force, dùng credential helper/SSH agent của Host) và GIT-07 state/rebase/cherry-pick/continue/abort; checkout có hộp xác nhận (Gate C) và status tự refresh theo watcher |
| T11 Preferences native | P0 | T02 | Persistent appearance, keymap, panels, general | Reload/validation/migration; accessibility | Đạt 06/11: schema v5, theme pack/density/font/keymap/panel width, restore last Session + draft per Session, keymap preset DSH/Zed/VS Code, **override theo workspace** (`workspace_overrides`: drag panel ghi override cho workspace đang chọn, Settings chỉnh explorer/density theo workspace, theme áp qua override khi refresh, validate + migrate v4→v5 kèm test) |
| T12 Skills & extensions | P0–P1 | Gate 0, T11 | Read installed, scoped enable/disable, validation/ack, details | Enabled ≠ installed; không execute skill vô cớ | Đạt 06/11: `skills/list` trả `source` (scope discovery) + `provider` để Skills view hiện provenance thật; **skill enable/disable bền vững** đã có: registry giữ disabled set, lọc ngay trong `collect()` nên model catalog, `/skill` và command list cùng tôn trọng, persist qua settings namespace của entry (`settingsNamespace` trong base bundle), RPC `skills/setDisabled` trả `persisted` trung thực, UI Disable/Enable ở cả hai shell (kèm disabled section); plugin bật/tắt có ack; bundle bật/tắt + Remove; Plugins page có HEALTH đếm fiber active/failed. Còn: **permissions/changelog** cho extension đã cài (index format không khai báo field này; `pluginManager/inspect` chỉ phục vụ spec trước khi cài) |
| T13 Marketplace | P1 | T12 | Trusted catalog/details, permissions, install/update/remove | Offline/fail/retry/verification; không success giả | Đạt 06/11: index/search/install lifecycle; mỗi dòng hiện **trust facts Host trả về** (author, host repo, trạng thái cài + version, compatibility) thay vì badge tự chế; **Update** = remove → inspect → install → bật lại enablement cũ, và khi một bước sau remove thất bại thì thông báo nói rõ package đã bị gỡ (không giả là còn nguyên); **Remove** dùng đầu trang Plugins. Còn: permissions/verified do index format không khai báo nên không bịa; rollback cài đặt cũ không có RPC nên chỉ báo trạng thái thật |
| T14 Enhanced workflows | P1 | T04,T06,T07,T09 | Fork, handoff, stage/commit, multi-terminal/detach khi supported | Không tự rewind filesystem, không auto-run side effects | Đạt 06/11: fork có chọn checkpoint, Goals, stage/commit (T08), tối đa 4 pane terminal, detach UI, **handoff** (module `handoff.rs` dựng Markdown chỉ từ log bền vững: run state, plan mode, goal + rounds/blocked reason, recent requests, tool failures lưu lại, todo snapshot, artifacts, kèm mục "Next steps" để trống; preview ở Goals page cả hai shell rồi ghi vào workspace qua `workspaceFiles/write` với version guard — file đã tồn tại bị từ chối kèm version trừ khi chọn Overwrite, không chạy gì thêm) |

**Kiểm chứng 06/11 (P1 close):** native-app `cargo test` **187 pass** (gpui) và **194 pass** với `--features legacy-egui` (gồm 37 snapshot/audit UI egui, snapshot đã cập nhật cho composer mới); client-core `cargo test` **121 pass**; `packages/skill/skill` + `session-controller` **1032 pass / 44 file** (gồm disabled-skill registry, `source`/`provider`, `skills/setDisabled`); `git.host.spec.ts` **72 pass**; `workspace-files/tests/search.spec.ts` **4 pass**; `bun install --frozen-lockfile` sạch; typecheck `skill`, `session-controller`, `workspace-files` sạch. Chưa chạy được snapshot pixel gpui (không có display) và probe Host thật trong CI keyless. Nợ catalog: `gen-cordis-catalog` còn 41 lỗi gán kiểu git từ slice Git trước nên chưa regen `api-catalog.ts`. Còn mở: permissions/changelog cho extension đã cài (index format không có field), T01 snapshot pixel gpui.

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
