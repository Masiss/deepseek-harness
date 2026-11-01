# DSH — Đặc tả chức năng UI v1.0

**Trạng thái:** Baseline chức năng để triển khai / cần đối chiếu adapter DSH hiện có trước khi nối runtime
**Ngày:** 30/10/2026
**Nguồn thiết kế:** `dsh-ui-prototype-v2.html`, `dsh-ui-feature-spec-v0.1.md` (HTML V2 đã được duyệt)
**Mục đích:** Biến prototype đã chốt thành hợp đồng hành vi có thể giao coding agent thực hiện, review, kiểm thử. **Không** thiết kế lại UI, không viết lại runtime DSH, không tự thay đổi mô hình session/agent/plugin đang hoạt động của DSH.

> **Phân biệt rõ:** nội dung chat, file tree, diff, model, terminal, Git, Marketplace trong HTML V2 là **dữ liệu mô phỏng**. Tài liệu dưới đây quy định hành vi mong muốn cho ứng dụng thật; các tên adapter/event minh họa **không phải** tuyên bố rằng API đó đã tồn tại trong DSH.

## 0. Design contract và quyết định đã khóa

### 0.1. Khung giao diện

| Vùng | Yêu cầu bất biến | Hành vi |
|---|---|---|
| Top bar | Mỏng **30px**, không có cụm ba nút macOS / logo DSH; hamburger góc trái, workspace selector, tìm kiếm, ẩn/hiện Changes | Settings chỉ vào từ menu ba gạch hoặc command palette; không đặt Settings icon riêng |
| Explorer (trái) | Cây file ngay dưới top bar, **không** thêm header “Explorer”, không tạo Activity Bar kiểu VS Code | Resize/ẩn; chân cột có `+` tạo session và dropdown đổi session; không có danh sách session thường trực |
| Agent (giữa) | Một buffer tab **Agent**, cao **34px**, thuộc riêng cột giữa | Nội dung chat và tool activity giữ mật độ/bố cục mẫu đầu; không có tab Changes ở giữa |
| Changes (phải) | Header và tabs Diff/Files thuộc riêng cột phải | Diff, duyệt file, review; không nhân đôi ở giữa |
| Bottom bar | Mỏng **26px**, thiên về icon, không nhét đoạn status dài | **Trái:** branch + Git + sync; **phải:** terminal + diagnostics + chấm kết nối; tooltip/aria-label giải thích |
| Preferences | Một cửa sổ tập trung, sidebar mục General, Appearance, Keymap, Extensions, Skills, Marketplace, About | Mở qua hamburger hoặc command palette; cùng một cấu trúc, không rải settings trên header/footer |

Màu sắc và kích thước trong HTML V2 là nguồn chuẩn để so visual regression; nên trích token CSS thay vì thiết kế thêm theme/layout tùy tiện. Tại desktop đủ rộng, Explorer và Changes resize độc lập. Khi không đủ rộng, cho phép ẩn một/both panel thay vì ép cột Agent xuống mức không đọc được.

### 0.2. Nguyên tắc product

1. **Agent-first, không giả làm IDE:** Explorer và Changes phục vụ luồng agent, không đòi xây editor/LSP hoàn chỉnh để ship UI.
2. **Một hành động, một nơi hiển thị chính:** terminal có icon ở bottom, Settings trong hamburger, Changes ở cột phải, session switch ở đáy Explorer.
3. **Dữ liệu thực mới có trạng thái thực:** tuyệt đối không hiển thị “đã lưu”, “job đã hoàn tất”, “connected”, “installed” dựa trên state giả khi đã nối production.
4. **Hành động nguy hiểm cần chỉ rõ phạm vi:** discard, overwrite, chạy shell, quyền truy cập filesystem, extension executable, thay branch có xung đột.
5. **UI không nắm quyền tự quyết scope agent:** model/agent có thể đề xuất; hành động trong vùng rủi ro phải qua approval do runtime/policy quyết định.
6. **Không thay thế host:** dùng session/agent/event/tool APIs hiện có của DSH; viết adapter nếu cần, giữ lõi runtime hiện tại.

### 0.3. Quy ước mức ưu tiên

- **P0 (cốt lõi):** cần để ứng dụng dùng được với runtime thật.
- **P1 (bước tiếp):** có giá trị rõ ràng, không chặn P0.
- **P2 (mở rộng):** chỉ triển khai sau khi chứng minh nhu cầu.
- Các `Acceptance criteria` (AC) dưới đây là **điều kiện để ghi DONE**, không phải ý tưởng trang trí.

---

## 1. Workspace & Explorer

### Mục tiêu

Dùng workspace làm ranh giới ngữ cảnh và truy cập file. Cho người dùng hiểu agent đang làm việc ở thư mục nào, chọn/xem file nhanh, không biến Explorer thành IDE toàn năng.

### 1.1. Chức năng

| ID | Ưu tiên | Hành vi bắt buộc | State cần có |
|---|---|---|---|
| WS-01 | P0 | Nút tên workspace ở top mở popover **Recent workspaces**, có Open Folder và khả năng chuyển workspace | `none / opening / ready / error` |
| WS-02 | P0 | Khi chuyển workspace: lưu draft/session context hiện tại, hỏi xử lý nếu còn lệnh nguy hiểm/chưa lưu tùy policy; không tự hủy job chạy | `activeWorkspaceId`; `pendingSwitch` |
| WS-03 | P0 | Explorer hiển thị cây thư mục từ root, lazy load thư mục mở lần đầu, giữ expanded paths riêng cho từng workspace | `loading / expanded / collapsed / inaccessible / missing` |
| WS-04 | P0 | Click folder: expand/collapse; click file: chọn file và mở **preview an toàn** trong buffer hoặc viewer phù hợp; selection không có side effect ghi file | `selectedPath`, `previewStatus` |
| WS-05 | P0 | Quick open tìm file theo path/tên + command (ưu tiên nội bộ, fuzzy match); chọn kết quả sẽ điều hướng đúng workspace | `query / searching / results / empty / error` |
| WS-06 | P0 | Sidebar resize và toggle từ menu/shortcut; width ghi nhớ theo workspace hoặc user theo cấu hình | `visible`, `width` |
| WS-07 | P0 | Khi file thay đổi ngoài UI: cập nhật cây/marker; nếu file đang mở bị xóa/đổi tên, không tự biến mất không báo | `modified / deleted / moved / conflict` |
| WS-08 | P1 | Context menu file/folder: Copy Path, Reveal, Open terminal here, Rename, New file/folder, Delete | `pending / succeeded / failed`; xác nhận thao tác phá hủy |
| WS-09 | P1 | Search trong workspace (tên + nội dung), filter exclude, hủy được; trả kết quả có path/line | `searchId`, `cancelled` |
| WS-10 | P2 | Drag/drop, đa workspace roots, bookmark | Chỉ sau P0/P1 |

**Quy tắc UI:** top bar chỉ hiện workspace một lần; tên root có thể hiện trong Explorer vì là node của file tree, nhưng **không thêm hàng tiêu đề Explorer**. Breadcrumbs dài không nằm cố định ở top bar. File preview có thể dùng buffer phụ; không mở vĩnh viễn panel mới làm co cột Agent ngoài ý muốn.

**Dữ liệu cần:** `workspaceId`, `displayName`, `rootUri`, `gitRootUri?`, `recentAt`, `permissionState`, `tree nodes {path,type,childrenState,isSymlink,modifiedFlag}`, `viewState {expandedPaths,selectedPath,width,visible}`.

**Lỗi/phòng vệ:** folder không còn tồn tại, permission denied, symlink trỏ ra ngoài root, file quá lớn/binary, watcher bị mất kết nối, workspace không còn mount. Preview không cố render file nhị phân như text; giới hạn đọc phần preview và cung cấp “open externally”/“load more” khi cần. Không mở file ngoài phạm vi đã cấp quyền chỉ bằng cách chỉnh URI.

**Acceptance criteria**

- **AC-WS-01:** Open workspace hợp lệ → hiển thị file tree thật, active workspace đổi đúng; không dùng tree mẫu.
- **AC-WS-02:** Đổi A→B→A → expanded paths, selected file, session của A có thể khôi phục; job của A không bị hủy ngầm.
- **AC-WS-03:** File bị xóa/rename ngoài app → tree và preview xử lý rõ, không crash và không để stale selection giả như còn tồn tại.
- **AC-WS-04:** Ẩn Explorer → Agent mở rộng; bật lại → kích thước trước đó được phục hồi.
- **AC-WS-05:** Quick open 0 kết quả/permission error → có trạng thái hiển thị; Enter/Esc/keyboard navigation chạy được.

---

## 2. Session Management

### Mục tiêu

Một session là lịch sử/điểm tiếp tục của agent gắn với workspace, không phải chỉ đổi nội dung HTML. Chuyển session phải **an toàn khi task/job đang chạy**.

### 2.1. Các trường và trạng thái

- `sessionId` là định danh từ runtime, không dùng counter số tạm của browser làm ID chuẩn.
- `workspaceId`, `title`, `createdAt`, `updatedAt`, `status`, `lastOpenedAt`, `modelRef?`, `executionMode?`, `draft?`, `runIds[]`, `forkedFrom?`, `archivedAt?`.
- `status` thuộc `idle | active | awaiting_approval | detached | interrupted | error | archived` (đây là vocabulary UI được đề xuất; adapter map từ DSH hiện có).
- **Tách rõ session và run:** một session có lịch sử nhiều run/tool actions; “session active” không đồng nghĩa có một tiến trình OS đang sống.

### 2.2. Chức năng

| ID | Ưu tiên | Hành vi | Điều kiện đặc biệt |
|---|---|---|---|
| SES-01 | P0 | Nút `+` đáy Explorer tạo session thuộc workspace đang chọn; focus composer | Không tự gọi model khi session rỗng |
| SES-02 | P0 | Dropdown cạnh `+` tìm/sắp xếp/chọn session; không có list luôn mở ở sidebar | Có status dot nhẹ cho run đang hoạt động |
| SES-03 | P0 | Chuyển session lưu draft per-session, render lại message/tool timeline từ runtime, giữ scroll anchor hợp lý | Không terminate run của session cũ |
| SES-04 | P0 | Khởi động lại app khôi phục last workspace/session nếu setting bật; nếu session thiếu hoặc không đọc được → fallback và nêu lý do | Runtime history là canonical |
| SES-05 | P0 | Run đang hoạt động mà user chuyển session: UI detach khỏi view nhưng **process tiếp tục theo policy**; khi quay lại nhận kết quả/stream đã bỏ lỡ | Không tạo run thứ hai do reconnect |
| SES-06 | P1 | Rename/archive; xóa session có confirm và mô tả hệ quả history/artifacts | Archive ≠ Delete |
| SES-07 | P1 | Fork session từ checkpoint/message; session mới có `forkedFrom`, policy đối với tool context và file snapshot rõ ràng | Fork không tự rewind filesystem |
| SES-08 | P1 | Search session theo title/metadata; filter active/archived | Paginated nếu nhiều session |
| SES-09 | P2 | Export/import/share session | Privacy gate riêng |

**Quy tắc tác vụ dài:** người dùng rời view, minimize app, mất kết nối, hibernate/resume: UI phải phân biệt `running`, `disconnected`, `interrupted`, `completed`. Nếu host không hỗ trợ detach bền vững, UI không được quảng cáo “background job”; hiển thị cảnh báo và hành vi thật.

**Acceptance criteria**

- **AC-SES-01:** Tạo session không làm mất lịch sử hoặc draft session khác.
- **AC-SES-02:** Bắt đầu run ở A, chuyển sang B, về A → không tự cancel, không tạo duplicate run; hiển thị tiến trình đúng từ runtime.
- **AC-SES-03:** Restart app sau khi nhập draft chưa gửi → draft còn nếu cấu hình restore bật; không hiện draft của workspace khác.
- **AC-SES-04:** Archive session đang có run → có policy rõ (chặn hoặc yêu cầu dừng/detach) trước khi archive.
- **AC-SES-05:** Runtime trả session missing/forbidden → thông báo cụ thể, fallback an toàn; không giả lập transcript.

---

## 3. Agent, Composer, Tool Activity & Approvals

### Mục tiêu

Đây là vùng chính của DSH. Người dùng điều khiển mục tiêu, mô hình, mode và quyền thực thi; agent phải báo trạng thái chuẩn xác, không bỏ dở giữa chừng mà chỉ báo “còn việc có thể làm”.

### 3.1. Timeline và nội dung

| ID | Ưu tiên | Hành vi |
|---|---|---|
| AG-01 | P0 | Load lịch sử theo `sessionId`, phân vai `user / assistant / tool / system-event`; giữ thứ tự event và timestamp từ nguồn chuẩn |
| AG-02 | P0 | Stream các delta nội dung assistant; đánh dấu trạng thái `queued / streaming / completed / interrupted / failed` |
| AG-03 | P0 | Markdown/code block an toàn, nút copy; link `path:line` dẫn tới file thực, link web mở theo chính sách an toàn |
| AG-04 | P0 | Tool activity một nhóm gọn, expand từng tool để xem arguments, stdout/stderr đã redacted, duration, exit code |
| AG-05 | P0 | Tool state `queued / running / succeeded / failed / cancelled / unknown`; không đánh dấu thành công khi mới gửi request |
| AG-06 | P0 | Plan/Ask/Build mode (tên hiển thị có thể map từ runtime), thay mode trước khi gửi; thay đổi mode giữa run đang chạy phải có semantics rõ |
| AG-07 | P0 | Nút Stop/Cancel run riêng với đóng panel. Stop phải nhận ack/timeout, không chỉ ẩn spinner |
| AG-08 | P0 | Permission prompt inline hoặc modal theo rủi ro, mô tả lệnh/tệp/phạm vi và hành động Allow Once / Deny (nếu runtime hỗ trợ thêm thì map) |
| AG-09 | P0 | Khi agent dừng với phần việc chưa hoàn thành: trạng thái `partial` hoặc `blocked`, trình bày việc còn lại và action Continue; không tự báo Done |
| AG-10 | P1 | Collapsible thinking/analysis **chỉ nếu runtime thật sự cung cấp phần nội dung được phép hiển thị**; không giả lập nội dung nội bộ |
| AG-11 | P1 | Summarize/handoff sau run, giúp resume: summary, quyết định, blockers, artifact refs; ghi vào storage hợp lệ |
| AG-12 | P1 | Multiple active runs/session hoặc queue task: phải có policy concurrency, chỉ mở khi runtime hỗ trợ và có UX rõ |

**Message presentation:** giữ task heading, Summary, Plan và Tool activity như mẫu đầu; không bắt agent luôn trả theo ba card cứng. Plan khi có sẽ là nội dung có version và tiến độ; nếu không có thì không render khung trống. `Tool activity` mặc định gọn; log rất dài phải virtualize và giới hạn memory.

### 3.2. Composer

| ID | Ưu tiên | Hành vi |
|---|---|---|
| CMP-01 | P0 | Textarea tự giãn theo số dòng, multiline; Enter gửi / Shift+Enter xuống dòng theo keymap, IME composition không submit nhầm |
| CMP-02 | P0 | `@` mở bộ chọn file/context, chèn reference có ID/path thực; hiển thị file mất quyền/mất trên disk |
| CMP-03 | P0 | Attach file: duyệt chọn hoặc drop (nếu hỗ trợ), tiến độ index/upload, remove trước gửi; kiểm tra size/type/trust |
| CMP-04 | P0 | Model picker chỉ liệt kê model/provider khả dụng từ runtime, hỗ trợ unavailable, quota/error; chọn model không tự đổi session cũ |
| CMP-05 | P0 | Mode selector (Ask/Plan/Build) từ runtime capability; mode không có trong provider phải disabled có giải thích |
| CMP-06 | P0 | Gửi prompt là giao dịch có `clientRequestId` chống double-send; trạng thái `draft → submitting → accepted/rejected`; retry không gửi trùng |
| CMP-07 | P0 | Khi run đã chạy: composer hiển thị Send/Queue/Stop tùy capability; không âm thầm gửi prompt thứ hai vào cùng run |
| CMP-08 | P1 | Prompt history, edit/recall unsent prompt; append context attachments có preview |

**An toàn prompt:** Các reference file do user chọn không đồng nghĩa cấp quyền thực thi. Lệnh shell, file write, network tool phải qua runtime policy. Không lưu secrets trong localStorage hoặc transcript plain text; tool output bị strip/redact credentials theo policy nhưng không giấu dữ liệu lỗi thiết yếu.

### 3.3. Event stream và recover

- Event có `eventId`, `sessionId`, `runId`, `sequence`, `timestamp`, `type`; adapter deduplicate/out-of-order theo `runId + sequence`.
- Mất websocket → state `reconnecting`, tiếp tục hiển thị lịch sử đã xác nhận; reconnect dùng cursor/resync snapshot, **không gửi lại prompt**.
- Nếu tool chạy nhưng UI không biết kết quả → `unknown/reconnecting`, không tự suy luận success.
- Resume từ run bị ngắt phải kiểm tra run đó có thể tiếp tục hay cần run mới. “Continue” không được gửi lại lệnh có side effect một cách mù quáng.
- Approval prompt phải chứa `requestId`, scope, expiration, risk summary; chỉ gửi quyết định sau thao tác người dùng thực sự.

**Acceptance criteria**

- **AC-AG-01:** Thử stream delta bị lặp hoặc mất kết nối → message không nhân đôi; khi reconnect tiếp tục đúng sequence.
- **AC-AG-02:** Tool exit code khác 0 → trạng thái Failed, nút mở log; không có tick xanh giả.
- **AC-AG-03:** Permission Deny → tool không chạy; Allow Once → chỉ áp dụng request đó.
- **AC-AG-04:** Nhập tiếng Việt qua IME, nhấn Enter trong composition → không gửi nhầm.
- **AC-AG-05:** Double click Send / reconnect / retry → một prompt tương ứng tối đa một accepted run.
- **AC-AG-06:** Agent kết thúc khi còn việc → `partial/blocked` và tóm tắt còn gì, không dùng completed giả.
- **AC-AG-07:** Draft/attachments giữ đúng session khi chuyển qua lại.

---

## 4. Changes & Review

### Mục tiêu

Cho người dùng thấy **agent đã thay đổi chính xác những gì** trước khi xác nhận. Tách rõ Git working tree, thay đổi do agent tạo, và review state. Không để nút Review âm thầm stage/commit/discard.

### 4.1. Chức năng

| ID | Ưu tiên | Hành vi |
|---|---|---|
| CHG-01 | P0 | Panel phải luôn là chủ sở hữu Changes; Diff/Files tabs; file count và +/- lấy từ dữ liệu thật |
| CHG-02 | P0 | List file `added / modified / deleted / renamed / untracked / conflicted`; click file nhảy tới diff/preview |
| CHG-03 | P0 | Hunk diff có line numbers, syntax highlight, expand context, file lớn progressive load |
| CHG-04 | P0 | Review current/all chỉ đánh dấu **đã xem xét** (local/metadata), không phải Accept, Stage hay Commit |
| CHG-05 | P0 | Khi workspace files đổi trong lúc đang xem: diff invalidated/stale phải refresh, báo nếu line anchoring không còn hợp lệ |
| CHG-06 | P0 | Discard chỉ tác động đối tượng đã chỉ định, bắt confirm nêu rõ file/hunk và có mất dữ liệu user hay không; nếu không thể hoàn tác, nêu rõ |
| CHG-07 | P0 | Không được discard changes do user tạo cùng file bằng thao tác “discard agent changes” nếu provenance không chứng minh được; fallback sang diff thủ công |
| CHG-08 | P1 | Stage/unstage theo file/hunk; commit nằm trong Git workflow, không gộp nút Review |
| CHG-09 | P1 | Checkpoint/rollback agent edit khi runtime có snapshot đáng tin cậy; review UI không tự coi Git HEAD là checkpoint của run |
| CHG-10 | P1 | Compare snapshots (before run vs after run) nếu runtime cấp file provenance |

### 4.2. Semantics nút

- **Review changes:** mở/chuyển diff và đánh dấu tiến trình xem xét sau thao tác rõ; không ghi thay đổi vào disk.
- **Reviewed:** metadata UI, có thể toggle; khi file content đổi, trạng thái reviewed của phiên bản cũ phải invalidated.
- **Accept:** tùy runtime/editor có thể chỉ là ghi nhận kết quả agent; **không** có nghĩa Git stage/commit. Nếu không có nghĩa kỹ thuật rõ, **đừng thêm nút Accept** ở P0.
- **Discard:** destructive; không dùng toast “đã hủy” khi backend chưa ack.

### 4.3. Data và consistency

`ChangeSet {workspaceId, baseRevision?, headRevision?, sourceRunId?, files[], observedAt}`; `ChangedFile {path, oldPath?, status, isBinary, isConflict, additions, deletions, contentHashBefore?, contentHashAfter?, provenance?, reviewedAt?, reviewHash?}`. Dữ liệu Git/FS live có thể khác patch agent; cần hiển thị nguồn và cảnh báo mismatch, không ghép mù.

**Acceptance criteria**

- **AC-CHG-01:** Agent sửa file → Changes cập nhật, diff phản ánh bytes trên disk/patch thật, không dùng hard-coded diff.
- **AC-CHG-02:** File do user sửa sau khi review → reviewed indicator mất hiệu lực.
- **AC-CHG-03:** Discard trên file có cả edits user+agent → chặn nếu không cô lập được vùng cần hoàn tác; không xóa work user.
- **AC-CHG-04:** No changes, binary diff, merge conflict, stale file → có empty/edge state riêng.
- **AC-CHG-05:** Ẩn Changes và bật lại không xóa selection hay review state còn hợp lệ.

---

## 5. Terminal & Git

### 5.1. Terminal

**Nơi mở:** icon terminal ở **góc phải bottom bar**; shortcut (tùy chọn trong Keymap) vẫn hợp lệ nhưng **không thêm Toggle Terminal vào menu ba gạch**.

| ID | Ưu tiên | Hành vi |
|---|---|---|
| TER-01 | P0 | Mở panel terminal trong layout, không che mất composer; close chỉ ẩn panel, không kill tiến trình |
| TER-02 | P0 | PTY attach vào native runtime, cwd theo workspace đã chọn; IO stream, resize PTY khi panel resize |
| TER-03 | P0 | Thể hiện trạng thái `starting / attached / running / exited / detached / error` và mã thoát; không hiển thị “live” khi không có PTY |
| TER-04 | P0 | Keyboard input, paste, select/copy, ANSI colors cơ bản, escape handling, scrollback có giới hạn |
| TER-05 | P0 | Khi đổi workspace/session, hành vi terminal per-workspace/per-session phải nhất quán theo cấu hình; không tự chạy lệnh khởi động lại |
| TER-06 | P1 | Nhiều terminal tabs + new/split/rename/close có confirm nếu process active |
| TER-07 | P1 | Detach/reconnect bền vững nếu runtime PTY hỗ trợ; phân biệt session agent với session terminal |
| TER-08 | P2 | Profiling, structured task runners, SSH remote terminal |

**Ranh giới:** tool-run do agent gọi (shell tool) không đồng nhất với terminal tương tác của user. Không tự đưa lệnh agent vào terminal user, không tự phát lại lệnh có side effect sau reconnect. Các lệnh do user gõ terminal thuộc runtime shell permission của user, không bị suy luận là đã được agent kiểm chứng.

**AC-TER-01:** Hide/show terminal không kill process; close terminal tab/process là thao tác riêng có xác nhận nếu còn job.

**AC-TER-02:** Terminal output đang stream vẫn đúng thứ tự sau resize; paste text không bị thực thi sai do UI handling.

**AC-TER-03:** Mất backend → cảnh báo disconnected; không mô phỏng terminal output giả.

### 5.2. Git + statusbar

**Nơi mở:** branch và Git icon **góc trái** bottom bar. Git icon mở Changes/Git workspace view; Sync icon chỉ hiện trạng thái thật và nhãn tooltip, không có chữ mô tả kéo dài. Phía phải là terminal, diagnostics và connection dot.

| ID | Ưu tiên | Hành vi |
|---|---|---|
| GIT-01 | P0 | Phát hiện repo và branch ở workspace hiện tại; ngoài Git repo thì biểu tượng neutral/no-repo, không giả `main` |
| GIT-02 | P0 | Click branch mở danh sách branch hiện có; checkout phải xử lý dirty working tree, conflict, detached HEAD |
| GIT-03 | P0 | Click Git icon mở Changes và focus tab đúng; không thêm tab Changes ở giữa |
| GIT-04 | P0 | Refresh status khi checkout/agent edit/FS watcher; indicator uncommitted có thể dùng icon nhỏ |
| GIT-05 | P1 | Stage/unstage, commit với message, diff staged vs unstaged, rõ trạng thái conflict |
| GIT-06 | P1 | Fetch/pull/push theo explicit action; không auto-push, không hiểu “sync” là push vô điều kiện |
| GIT-07 | P2 | Rebase/cherry-pick/interactive conflict tools |

**AC-GIT-01:** Repo trống hoặc không phải Git → không hiện branch giả và không crash.

**AC-GIT-02:** Checkout khi dirty → runtime trả outcome/needs-resolution; UI không tự overwrite file.

**AC-GIT-03:** Git icon ở bên trái, terminal bên phải, không có labels dư thừa làm dày statusbar.

---

## 6. Preferences, Extensions, Skills & Marketplace

### 6.1. Information architecture đã khóa

Menu hamburger ngắn, chia nhóm:

- **Workspace:** Quick open · Toggle Explorer · New session.
- **Customize:** Preferences… · Appearance · Keymap · Extensions · Skills · Marketplace (các mục sau deep-link vào cùng Preferences, không mở nhiều loại settings window).
- **Help:** About DSH.

**Không** có Settings icon top-right. **Không** có Toggle Terminal trong hamburger (đã có bottom). Cửa sổ Preferences sidebar: General / Appearance / Keymap / **EXTEND** Extensions, Skills, Marketplace / **MORE** About. Với nhiều nguồn integrations mới trong tương lai, thêm màn Connections riêng **chỉ khi** nhu cầu có thật và không nhồi chung vào Extensions.

### 6.2. General

| ID | Ưu tiên | Hành vi |
|---|---|---|
| PREF-01 | P0 | Restore last workspace/session toggle: phản ánh vào startup restore thực |
| PREF-02 | P0 | Restore panel layout toggle: kích thước/visibility của Explorer, Changes, Terminal |
| PREF-03 | P0 | Preferences validate, lưu atomic vào storage ứng dụng, reload không mất, báo lỗi khi write fail |
| PREF-04 | P1 | Export/import preferences, Reset từng nhóm |

Persistence nên theo precedence **default → global → workspace override**; UI phải chỉ rõ option nào được override ở workspace. Không để secrets trong preference blob export được.

### 6.3. Appearance

| ID | Ưu tiên | Hành vi |
|---|---|---|
| APP-01 | P0 | Theme `Dark / Midnight / Light`, preview tức thời; persisted |
| APP-02 | P0 | Accent `Blue / Teal / Violet`, ảnh hưởng active/focus, không đổi màu semantic error/success |
| APP-03 | P0 | Density `Comfortable / Compact` tác động spacing, không xóa content |
| APP-04 | P0 | Font scale 11–16px theo V2; giữ code mono/readability, zoom/OS scaling không làm vỡ panel |
| APP-05 | P1 | Follow system theme, syntax theme, font family |

**AC-APP:** Chuyển theme và reopen app → đúng theme; font scale lớn không che nút Send/Close; tương phản & focus ring đủ nhận diện.

### 6.4. Keymap

| ID | Ưu tiên | Hành vi |
|---|---|---|
| KEY-01 | P0 | Presets `DSH Default / Zed-inspired / VS Code-inspired`; là quy tắc keybinding, không cam kết hành vi IDE khác |
| KEY-02 | P0 | Search command → record shortcut → validate conflict → Save/Reset |
| KEY-03 | P0 | Command registry làm nguồn duy nhất cho menu tooltip, command palette, keyboard dispatcher |
| KEY-04 | P0 | Phân xử keyboard theo dialog/shortcut capture → editor/composer focus → user overrides → preset → fallback; Esc ưu tiên đóng lớp trên cùng |
| KEY-05 | P1 | Context-sensitive shortcut và multi-stroke chords (nếu runtime hỗ trợ) |

**AC-KEY:** Không cho 2 commands cùng binding trong cùng scope nếu xung đột; khi đang gõ composer không tự kích hoạt shortcut thường (trừ tổ hợp được định nghĩa global); IME và OS-reserved shortcut không bị hijack.

### 6.5. Extensions

**Definition:** Extension là mã/integration có thể mở capability (command, UI provider, LSP, MCP adapter...), cần lifecycle và permission. **Không** đồng nhất extension với skill văn bản.

| ID | Ưu tiên | Hành vi |
|---|---|---|
| EXT-01 | P0 | Installed list: `id, name, version, publisher, source, scope, enabled, compatibility, health, permissions` |
| EXT-02 | P0 | Enable/disable chỉ hoàn tất sau runtime ack; nếu requires restart hiển thị rõ |
| EXT-03 | P1 | Details: permissions, changelog, update, disable, uninstall, failure logs |
| EXT-04 | P1 | Extension process lỗi/timeout → isolate failure, degrade capability có cảnh báo; không làm sập toàn bộ DSH |
| EXT-05 | P2 | Sandbox policy / host protocol extension theo DSH architecture hiện tại, không tự tạo runtime song song |

**AC-EXT:** Bật lỗi hoặc incompatible → giữ state cũ và báo nguyên nhân; extension không có quyền mặc định đọc toàn bộ thư mục hoặc network nếu permission chưa chấp thuận.

### 6.6. Skills

**Definition:** Skill là gói hướng dẫn/quy trình cho agent, có metadata và scope; không tự có quyền truy cập tool chỉ vì enabled.

| ID | Ưu tiên | Hành vi |
|---|---|---|
| SKL-01 | P0 | List installed: `id, name, description, version, source, scope(global/workspace), enabled, invocationPolicy` |
| SKL-02 | P0 | Enable/disable và khả năng áp dụng đến session hiện tại/lần chạy tiếp theo; tránh silently re-inject giữa run |
| SKL-03 | P0 | Xem nội dung/manifest read-only, nơi lưu, provenance; phân biệt skill cục bộ vs marketplace |
| SKL-04 | P1 | Search/invoke qua composer `/skill` hoặc command palette nếu runtime có cơ chế hỗ trợ |
| SKL-05 | P1 | Workspace override, conflict giữa skill cùng tên, skill dependencies, version pin |

**AC-SKL:** Tắt skill → lần chạy tiếp theo không inject; run đang chạy không bị đổi context ngầm; skill có chỉ dẫn gọi tool vẫn phải qua approval policy gốc.

### 6.7. Marketplace

| ID | Ưu tiên | Hành vi |
|---|---|---|
| MKT-01 | P1 | Search/filter All / Extensions / Skills; phân loại rõ ràng |
| MKT-02 | P1 | Details trước install: publisher, repository/source, version, runtime compatibility, dependencies, requested permissions, verified/not verified |
| MKT-03 | P1 | Cài qua runtime package manager: resolving → verifying → permission confirm → downloading/installing → installed/failed; có cancel nếu safe |
| MKT-04 | P1 | Update/uninstall; dependency impact; rollback last known good khi supported |
| MKT-05 | P1 | Catalog offline/error/cache stale; không ngụy tạo danh mục online nếu đang dùng bản cache |
| MKT-06 | P2 | Ratings/curation/signed publishers và private registries, chỉ khi có backend & trust model |

**Bảo mật tối thiểu trước P1 production:** registry allowlist hoặc nguồn xác thực, checksum/signature khi khả thi, strict manifest validation, command execution isolation theo host hiện hữu, audit log install/update/uninstall, permission confirmation, no silent auto-run. “Installed” ≠ “Enabled”; “Verified” chỉ khi chứng thực nguồn thực sự. Marketplace HTML V2 chỉ là simulated catalog.

**AC-MKT:** Cài lỗi → trạng thái Failed + lý do + retry, không tự bật `installed=true`; uninstall package đang dùng phải nêu hậu quả; offline không xuất hiện “download complete”.

---

## 7. UX states, lỗi, accessibility và non-functional

### 7.1. State matrix chung

| Khu vực | Empty | Loading | Success | Failure / recovery |
|---|---|---|---|---|
| Workspace | Chưa mở workspace → Open Folder | Đọc root/index | Tree + recent state | Missing/permission; Retry/Remove recent |
| Session | Chưa có session → New session | Fetch/resume | Timeline + draft | Missing/interrupted; recover hoặc tạo mới |
| Agent | Chưa có message → composer focus | Streaming + tool activity | Completed/partial rõ | Failed/reconnect, Retry nếu idempotent |
| Changes | No changes → “No changes” | Index/diff | Diff/Files | Stale/conflict/binary, Refresh/resolve |
| Terminal | Chưa mở | Spawning/attaching | Attached/output | Exited/disconnected, reconnect/spawn explicitly |
| Git | Not a repository | Status refresh | Branch/status | Dirty checkout/conflict/auth, hướng xử lý |
| Marketplace | No installed items | Search/install | Installed/enabled tách bạch | Offline/package invalid/permission denied |

### 7.2. Accessibility / keyboard

- Mọi icon-only button có tooltip và accessible name; focus rõ; hỗ trợ Tab/Shift+Tab, Arrow ở list/tree, Enter/Space để activate, Esc đóng popover/dialog.
- Modal trap focus và khôi phục focus về trigger sau khi đóng; popover không chặn nhập composer nếu đã đóng.
- `aria-live` chỉ dùng cho status ngắn/approval cấp thiết, không đọc lại toàn bộ streamed tokens gây ồn.
- Màu xanh/đỏ diff phải có ký hiệu `+/-`, không chỉ phân biệt bằng màu; high contrast state khi font scale lớn.
- Respect reduced motion; resize panel và scroll không giật; chọn text trong tool output mà không trigger row action.
- IME tiếng Việt: composer và quick open không submit khi đang composition.

### 7.3. Reliability/security

- Không render markdown/untrusted output bằng raw HTML; sanitize, chặn injection URL/protocol, log control sequences, terminal escape hazards.
- Không để prompt/file attachment không được cấp quyền tràn sang workspace khác; sessions cách ly theo ID và permissions.
- Các hành động mutating cần acknowledgment; timeout → unknown/retry safe, không claim success.
- Async operations cancellable (search, file read, install, some tool actions) khi backend hỗ trợ, cancel idempotent.
- Reconnect/reload: tôn trọng event cursor, không double-dispatch write/terminal commands.
- Tất cả trạng thái `Offline/Online` lấy từ heartbeat/health thực, không dựa vào websocket UI khởi tạo thành công một lần.

### 7.4. Mục tiêu hiệu năng (đề xuất, cần đo trên máy target)

- Click/focus/popover/panel toggle: phản hồi visual **<100ms** p95 trong điều kiện thông thường; animation không cần để che delay.
- Scroll file tree/timeline/diff mượt khi hàng nghìn mục nhờ windowing/virtualization, không render toàn bộ log cực lớn.
- File tree lazy load; Quick Open có debounce/cancel stale search và hiển thị kết quả liên tục.
- Terminal scrollback có cap cấu hình; assistant event buffer có backpressure; không giữ toàn bộ stdout và attachment binary trong DOM.
- Test viewport desktop 1366×768, 1920×1080 và HiDPI; khi panel width conflict, ưu tiên Agent và cho ẩn Changes/Explorer.

### 7.5. Test scenarios bắt buộc

| ID | Scenario | Expected |
|---|---|---|
| UX-01 | Ba panel ở width thấp → hide Explorer/Changes | Agent vẫn đọc được, no horizontal overflow toàn app |
| UX-02 | Menu + Preferences + command palette mở đóng chồng nhau | Topmost Esc, focus return, không kẹt overlay |
| UX-03 | Thay theme/font/keymap rồi restart | Persistence chính xác, fallback nếu value không hợp lệ |
| UX-04 | Tool đang chạy, đổi session rồi reopen | Không mất stream/cancel, không báo completed giả |
| UX-05 | Network disconnect rồi reconnect | Đúng trạng thái, dedup event, resync |
| UX-06 | 10k file tree / long diff / long tool output | UI không freeze; scroll và selection còn hoạt động |
| UX-07 | Deny permission/failed install/discard conflict | Không có tác động phá hủy, thông báo cụ thể |
| UX-08 | Gõ tiếng Việt IME + screenreader/keyboard only | Composer không send nhầm; action có label |

---

## 8. Hợp đồng dữ liệu & event (đề xuất; map vào DSH hiện hữu)

### 8.1. Domain models tối thiểu

```ts
type Id = string;
type Workspace = {
  id: Id; rootUri: string; name: string;
  gitRootUri?: string; permission: 'granted'|'denied'|'unknown';
};
type Session = {
  id: Id; workspaceId: Id; title: string;
  status: 'idle'|'active'|'awaiting_approval'|'detached'|'interrupted'|'error'|'archived';
  currentRunId?: Id; forkedFrom?: { sessionId: Id; messageId?: Id };
  updatedAt: string;
};
type AgentRun = {
  id: Id; sessionId: Id; status: 'queued'|'running'|'partial'|'completed'|'failed'|'cancelled'|'unknown';
  modelId: string; mode: string; startedAt?: string; finishedAt?: string;
};
type ToolInvocation = {
  id: Id; runId: Id; name: string; status: 'queued'|'running'|'succeeded'|'failed'|'cancelled'|'unknown';
  argsSummary?: string; durationMs?: number; exitCode?: number; requiresApproval?: boolean;
};
type AgentEvent = {
  eventId: Id; sessionId: Id; runId: Id; seq: number; time: string;
  type: 'message.delta'|'message.completed'|'tool.started'|'tool.updated'|
        'tool.finished'|'approval.requested'|'run.updated'|'artifact.updated'|'error';
  payload: unknown;
};
type UIState = {
  activeWorkspaceId?: Id; activeSessionId?: Id;
  panels: { explorer: {visible:boolean;width:number}; changes: {visible:boolean;width:number}; terminal:{visible:boolean;height:number} };
  selectedFileUri?: string; activeChangeFileUri?: string;
};
```

**Không nên phụ thuộc trực tiếp vào names/type ví dụ** nếu API thực tế khác. Có thể dùng adapter layer chuẩn hóa và xác nhận stream error/reconnect. Tránh copy runtime state sang DOM/localStorage như nguồn chân lý.

### 8.2. UI↔runtime operations (capability adapter)

| UI action | Capability/operation cần có | Response lỗi/cảnh báo |
|---|---|---|
| Open workspace | `workspace.open(rootUri)` / `workspace.list` | permission, missing, unsupported |
| Read tree/file | `fs.list/read/stat/watch` trong workspace sandbox | forbidden, missing, binary, large |
| Create/switch session | `session.create/list/get` | unavailable, archived, denied |
| Fetch timeline | `session.history(sessionId,cursor)` | stale cursor/retry |
| Send prompt | `run.start({sessionId,clientRequestId,...})` | busy, invalid model/mode, quota |
| Stop/continue | `run.cancel` / `run.resume` nếu supported | not cancellable / unsafe retry |
| Subscribe events | `run.subscribe({sessionId,cursor})` + snapshot | disconnect/gap |
| Tool approval | `approval.respond(requestId,decision)` | expired, already resolved |
| Diff/review | `changes.list/diff`, `review.set` | stale hash/conflict |
| Discard | `changes.discard({fileVersion,scope})` | dirty user edits/irreversible |
| Terminal | `pty.create/attach/input/resize/close` | unavailable, exited |
| Git | `git.status/branches/checkout` | dirty/unmerged/auth |
| Preferences | `settings.read/update` | schema/version conflict |
| Extensions/skills | `package.list/enable/disable`, `skills.list...` | incompatible, denied |
| Marketplace | `catalog.search`, `package.install/update/remove` | offline/trust/permission |

**Rule API:** operations mutating có `requestId`/idempotency key; confirm qua ACK/canonical re-fetch. UI loading indicator không đồng nghĩa thành công. DSH runtime có thể có API tương đương khác tên, dùng name mapping và tests cho adapter.

### 8.3. Data ownership

| Dữ liệu | Chủ sở hữu authoritative | Cache UI cho phép |
|---|---|---|
| Messages, run events, approvals | DSH runtime/session store | read-only view cache + pagination |
| Workspace tree/file version | Filesystem/provider | tree cache có watcher invalidation |
| Git status/diff | Git/fs adapter | snapshot có revision/timestamp |
| Terminal process | PTY host | rendering buffer + scrollback cap |
| UI layout/theme/keymap | App preferences store | in-memory state |
| Installed extensions/skills | Runtime package/skill registry | catalog metadata cache |
| Marketplace catalog | Registry backend/source | offline cache có timestamp |
| Session draft | App/session draft store | local autosave + conflict protection |

---

## 9. Mapping từ HTML V2 sang production

| Khu vực HTML V2 | Hiện tại | Khi đưa vào app thật |
|---|---|---|
| Layout / top-bottom / panel split | CSS/DOM hoạt động | Giữ gần như nguyên, port sang UI framework hiện có |
| Explorer file tree | Sample object literal | Runtime FS, lazy load, errors, watcher |
| Workspace selector | Demo recent list | Workspace registry/permission |
| Session | Browser-local ID/chat HTML | Canonical session store, streamed events, drafts |
| Agent composer | Chèn message local và notice demo | Send/ack/run streaming/approval/stop |
| Tool activity | 5 sample rows tĩnh | Tool lifecycle stream và raw logs có kiểm soát |
| Changes/diff | Diff hard-coded | Workspace/Git/agent provenance, review semantics |
| Terminal | Help/pwd/ls mock | PTY, lifecycle, attach/detach |
| Git/status | Nhãn branch/chấm status demo | Git adapter, heartbeat/diagnostics thực |
| Appearance/keymap | localStorage demo | Versioned persistent settings + command registry |
| Extension/skills toggles | localStorage booleans | Registry/lifecycle, scope, source, permissions |
| Marketplace | Sample catalog/install toggle | Package manager, trusted source, install lifecycle |

**Đặc biệt phải sửa khi port:** HTML V2 lưu `innerHTML` message history trong localStorage; ứng dụng thật **không** dùng cách này (rủi ro injection, thiếu dữ liệu event/run). V2 dùng dữ liệu giả về checkpoints trong workspace HIS_EMR; production phải hiển thị đúng session/workspace được runtime trả về, không hard-code nội dung.

---

## 10. Roadmap thực thi theo lát cắt có thể nghiệm thu

**R0 — Freeze V2 + command registry (P0).** Chuẩn hóa token UI, route Preferences, IDs command và test visual regression. Không đổi layout.

**R1 — Shell, workspace, session (P0).** Workspace adapter, Explorer lazy load, session list/create/switch, draft restore, panel persistence. Demo data bị loại khỏi runtime path. Gate: AC-WS và AC-SES P0.

**R2 — Agent core (P0).** History/event stream, composer, model/mode capability, tool states, approval, stop, partial/resume. Gate: AC-AG, disconnect/reconnect test.

**R3 — Review + Terminal + Git (P0).** Real diff/read-only review, guarded discard, PTY, Git branch status; không phải Git full IDE. Gate: AC-CHG, AC-TER, AC-GIT P0.

**R4 — Preferences hardening (P0).** App persistence, keymap registry, appearance, extension/skill registry read/enable lifecycle; Marketplace **chưa cần install thật**. Gate: AC-APP/KEY/EXT/SKL.

**R5 — Marketplace and enhanced workflows (P1).** Trusted catalog, verification, install/update/uninstall, skill invocation, Git stage/commit, session fork/handoff. Gate: AC-MKT plus security review.

**Release gate chung:** 0 critical loss-of-data; 0 silent dangerous action; không có UI giả “completed/online/installed”; smoke tests desktop + viewport nhỏ + hibernate/reconnect; đo performance thay vì suy đoán.

---

## 11. Các quyết định phải kiểm tra với codebase DSH trước khi implement

Đây là **điểm tích hợp**, không phải yêu cầu người dùng trả lời trước khi viết code:

1. Session store/API hiện tại lưu message history, run IDs, checkpoint/handoff ở đâu? Có cursor event stream chưa?
2. Host DSH đang có PTY multiplex/detach thật chưa, hay terminal chỉ hoạt động khi cửa sổ còn mở?
3. Tính năng approve tool/fs/shell hiện tại do component nào sở hữu? Không tạo một policy engine mới trong UI.
4. Git/Changes hiện tại là Git working tree hay patch do agent tạo? Có source provenance/snapshot không?
5. Runtime có mechanism extension, skill và marketplace nào rồi? Phải làm inventory trước khi chốt schema/lifecycle mới.
6. Cơ chế lưu settings/keymap hiện có là gì? Khả năng merge global/workspace, migration qua phiên bản?
7. Native app/framework (desktop shell) có đang xử lý titlebar/OS keybindings riêng không? HTML V2 chỉ là reference visual.

**Quy tắc khi thiếu adapter:** ghi `NOT_SUPPORTED` và hiển thị UX fallback trung thực; không dựng handler mô phỏng thành công. Chỉ tạo abstraction mới khi thiếu capability thật, không duplicate logic hiện hữu.

## 12. Definition of Done cho mọi feature

Một hạng mục chỉ được đánh DONE khi: (a) khớp visual baseline V2 nếu có UI; (b) state transitions đúng; (c) error/empty/reconnect handled; (d) có unit/integration test cho behavior chính; (e) accessibility và keyboard hoạt động; (f) data lấy từ đúng owner; (g) đã kiểm tra không phát sinh side effect ngoài scope và không claim success giả; (h) có đường phục hồi/backout rõ cho mọi thao tác nguy hiểm.

**Out of scope của baseline v1.0:** làm IDE hoàn chỉnh như Zed/VS Code; rewrite host bằng Rust; thay agent harness; tự xây model gateway; unrestricted parallel agents; remote collaboration; marketplace mở không kiểm duyệt; tự động commit/push; auto-run scripts sau resume.
