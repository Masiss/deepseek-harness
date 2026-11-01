# DSH UI V2 — Implementation Backlog v1.0

**Dùng cùng:** `DSH_UI_Functional_Spec_v1.0.md` (hợp đồng hành vi) và `dsh-ui-prototype-v2.html` (visual reference).
**Quy tắc:** Không rewrite DSH runtime; không thay layout V2; không nối giả UI state thành backend success. Mỗi task phải có test cho đường thành công + lỗi + re-entry/reconnect nếu liên quan.

## 0. Gate trước coding — capability inventory

- [ ] Xác định Desktop host/UI framework đang chạy; vị trí shell, command dispatcher, preferences.
- [ ] Tìm API hiện có cho workspace/filesystem, session/history, run/events/tool approval, Git, PTY.
- [ ] Tìm extension/skill registry có sẵn và cơ chế package installer (nếu có).
- [ ] Ghi bảng `SUPPORTED / PARTIAL / NOT_SUPPORTED`, source code path và gap cần adapter.
- [ ] Nêu rõ những gì cần hỏi user trước khi thay behavior rủi ro; **không tự quyết viết host mới**.

**Deliverable:** `runtime-capability-map.md`, đề xuất adapter tối thiểu, test hiện có. **Không thay code production** ở gate này.

## 1. Sprint theo vertical slices

| Task | Mức | Phụ thuộc | Phạm vi giao coding agent | Gate nghiệm thu |
|---|---|---|---|---|
| T01 UI shell | P0 | Gate 0 | Chuyển token/layout V2 vào desktop UI, 30px/34px/26px, menu, panels; visual test | Không thêm Changes ở giữa, Terminal không có trong hamburger, Git ở bottom trái |
| T02 Command registry | P0 | T01 | Chuẩn IDs, routes, menu, keymap dispatcher, popup focus/Esc | Một command dùng chung menu/shortcut/palette, không xung đột |
| T03 Workspace adapter | P0 | Gate 0, T01 | Open/switch workspace, list tree lazy, selection, preview, file events | AC-WS-01…05 và permission/missing-file tests |
| T04 Session adapter | P0 | Gate 0, T03 | Create/list/switch, runtime history, drafts, restore, active run badge | AC-SES-01…05; đổi session không cancel run |
| T05 Agent read path | P0 | T04 | Render message/tool events, reconnect/cursor, scroll/log caps | No duplicate/out-of-order event, tool errors thật |
| T06 Composer send path | P0 | T05 | Model/mode availability, attach/@file, submit ack, cancel/approval | AC-AG-03…07; idempotency, IME |
| T07 Changes read path | P0 | T03 | Real status/diff, file list, review metadata invalidation | Diff đúng, no changes/conflict/binary/stale states |
| T08 Changes destructive actions | P0 | T07 | Guarded discard với version/provenance và confirm | Không mất user edits; rollback/recovery được nếu supported |
| T09 Terminal/PTy | P0 | Gate 0, T01 | Native terminal attach/input/resize/close lifecycle | Hide không kill; disconnected không hiện alive giả |
| T10 Git basics | P0 | T03, T07 | Repo detection, branch picker, guarded checkout, Git bottom-left | No-repo / dirty checkout / status refresh |
| T11 Preferences native | P0 | T02 | Persistent appearance, keymap, panels, general | Reload/validation/migration; accessibility |
| T12 Skills & extensions | P0–P1 | Gate 0, T11 | Read installed, scoped enable/disable, validation/ack, details | Enabled ≠ installed; không execute skill vô cớ |
| T13 Marketplace | P1 | T12 | Trusted catalog/details, permissions, install/update/remove | Offline/fail/retry/verification; không success giả |
| T14 Enhanced workflows | P1 | T04,T06,T07,T09 | Fork, handoff, stage/commit, multi-terminal/detach khi supported | Không tự rewind filesystem, không auto-run side effects |

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
