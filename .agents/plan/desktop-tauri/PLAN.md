# PLAN — Desktop Tauri (Rust) + Node

Tài liệu này gộp **ADR** và **kế hoạch chi tiết theo phase**. Quy tắc làm việc: [`AGENTS.md`](AGENTS.md). Số đo và bẫy: [`MEMORY.md`](MEMORY.md).

- Trạng thái: kế hoạch, chưa implement.
- Ngày: 2026-10-19.
- Phạm vi: chỉ `apps/desktop-tauri`, `native/dsh-native`, provider packages và glue tối thiểu.

---

# Phần I — Bối cảnh và mục tiêu

## 1.1 Hiện trạng

| Hạng mục | Giá trị |
|---|---|
| Shell desktop | Electron 44.7 (`apps/desktop`, ~7.900 dòng TS) |
| Runtime | Node 24.21 đóng gói (`apps/desktop-host`) |
| Nền tảng | macOS arm64/x64, Windows x64 |
| Linux | Runtime đã hỗ trợ; **chưa có desktop shell** |
| Kích thước cài | ước tính 450–550 MB |
| Native | koffi (FFI), node-pty, sharp, 2 addon C |

## 1.2 Mục tiêu

| # | Mục tiêu | Đo bằng |
|---|---|---|
| G1 | Desktop chạy Tauri, không Electron | không còn `electron` trong bundle |
| G2 | Cực nhẹ | core ≤ 160 MB, phấn đấu ≤ 105 MB |
| G3 | Giữ gốc sản phẩm | 10 bất biến trong `AGENTS.md` §2 |
| G4 | Giữ merge upstream | `git diff upstream/master...HEAD -- packages/` = 0 |
| G5 | Có Linux | `.deb` + AppImage |
| G6 | Rust hoá phần an toàn | shell + native layer |

## 1.3 Ngoài phạm vi

- Rewrite agent runtime sang Rust.
- Thay đổi session format, tool schema, prompt.
- Sửa `packages/core`, `llm`, `session`, `bundle`.
- Embedded browser parity đầy đủ ở v1.

---

# Phần II — ADR

## ADR-0001 — Tauri thay Electron làm desktop shell

**Trạng thái:** accepted · 2026-10-19

**Bối cảnh:** Electron đóng gói Chromium (~117–124 MB nén) và chỉ hỗ trợ macOS/Windows.

**Quyết định:** dùng Tauri 2 (Rust) cho desktop mới; Electron giữ làm cầu tạm cho Windows/macOS đến khi port xong.

**Hệ quả:**

- Giảm ~150–200 MB, có Linux.
- Shell trở thành Rust, khớp mục tiêu.
- Phải port tray/menu/dialog/updater/signing.
- Phụ thuộc WebView hệ thống (WKWebView, WebKitGTK, WebView2).

**Phương án bị loại:**

- Giữ Electron + tối ưu: không giải quyết size/Linux.
- Viết lại toàn bộ bằng Rust: phá plugin ecosystem.

## ADR-0002 — Runtime JS của app là Node, không phải Bun

**Trạng thái:** ~~accepted, có điều kiện P0~~ → **superseded ở P0 (2026-10-19)**; bản gốc là "Bun thay Node trong app đóng gói".

**Bối cảnh:** Node 24.21 chiếm 121 MB; Bun 67 MB và bundle được JS. P0 đã đo thật.

**Quyết định (bản gốc, đã bác):** dùng Bun làm runtime JS của sidecar; ưu tiên `bun build --compile`, fallback Bun runtime + production tree, fallback cuối là Node.

**Bằng chứng bác bỏ (P0):**

1. **Chặn cứng.** Boot Host dưới Bun chết ngay:
   `node-addon-require-builtin unsupported: Unsupported/no-context (required V8 current-context symbols were not found)`.
   Addon đó là cách `packages/boot/app-boot/src/profile-resolution/resolver.ts` lấy loader ESM/CJS nội bộ của Node; Bun dùng JavaScriptCore, không có symbol V8 current-context, và `installRuntimeInterception()` không có nhánh fallback. Đây là cơ chế lõi của plugin/profile resolution.
2. **`--compile` vướng thêm hai nhóm lỗi độc lập** kể cả khi bỏ qua (1): static import `node:sea` / `node:module#registerHooks` (Bun thiếu), và 10 điểm dùng `import.meta.url` để đọc `package.json` (trong bundle trỏ vào `$bunfs`), gồm cả `INSTALL_ANCHOR`. Vá hết đòi sửa nhiều file upstream → vi phạm L0.
3. `node:sqlite` thì Bun **có** hỗ trợ — không phải lý do bác.

**Quyết định thay thế:** sidecar chạy **Node 24**, cùng cây production `dsh` như bản Electron. Bun chỉ còn giữ vai trò tooling build/test của repo (đã chuyển ở nhánh `chore/web-bun-migration`). Shell Tauri **không** bundle Chromium, nên phần giảm lớn so với Electron vẫn nguyên.

**Hệ quả:**

- Mất ~40–50 MB so với kịch bản Bun (Node 104 MB stripped thay vì Bun 67 MB) và không bỏ được `node_modules`.
- Đổi lại: **không** phải vá 10+ file upstream, không phải viết lại resolver, giữ nguyên đường merge.
- Thắng chính vẫn nằm ở payload decomposition (Office/Python tải sau, prune, brotli, bỏ Chromium).

**Điều kiện chấp nhận P0 đã đạt:** Host boot, phát frame `ready` có URL + injection, nhận `shutdown`, thoát code 0 — đo được cold start ~0,9–1,2 s và RSS idle 266–268 MiB.

## ADR-0002b — Transport shell↔Host là NDJSON

**Trạng thái:** accepted, đã implement ở P0

**Bối cảnh:** Electron fork Host với kênh IPC của Node (`process.send`). Tauri (Rust) không tạo được kênh IPC nội bộ đó.

**Quyết định:** `apps/desktop-host` tự chọn transport: có `process.send` thì dùng IPC (Electron giữ nguyên), không có thì dùng **NDJSON trên stdout/stdin**, mỗi frame một dòng, tiền tố `@@DSH@@` để phân biệt với log lẫn vào cùng luồng.

- Decoder bỏ qua dòng không có tiền tố, JSON hỏng, hoặc thiếu `type`; giới hạn 1 MiB/frame.
- Ở chế độ stdio, `console.log`/`console.info` được chuyển sang stderr.
- `close()` nhả stdin (`unref`) sau khi báo `shutdown-complete` để tiến trình thoát.

**Hệ quả:** Electron cũ không đổi hành vi; shell Tauri chỉ cần đọc/ghi dòng.

## ADR-0003 — Harness runtime giữ TypeScript

**Trạng thái:** accepted

**Bối cảnh:** 449.575 dòng TS/TSX, 342 package, plugin Cordis nạp động.

**Quyết định:** không rewrite runtime sang Rust. Rust chỉ ở shell và native seam.

**Hệ quả:** giữ ecosystem; giới hạn mức giảm kích thước ở sàn Bun 67 MB.

## ADR-0004 — Một crate napi-rs thay koffi/node-pty/C addon

**Trạng thái:** accepted

**Bối cảnh:** 5 cơ chế native khác nhau; koffi gọi Win32 bằng chuỗi signature không type-safe.

**Quyết định:** gom vào `@deepseek-ai/dsh-native` (napi-rs), `#[cfg(target_os)]`.

**Hệ quả:**

- Bỏ koffi (2,3 MB), node-pty, 2 addon C.
- Code Win32 có kiểu, dễ audit.
- Cần CI matrix và prebuilt theo platform/arch/ABI.

**Phương án bị loại:** port từng package riêng lẻ (5 bộ build, 5 ABI).

## ADR-0005 — Fork-friendly: L0/L1/L2 + provider swap

**Trạng thái:** accepted

**Bối cảnh:** phải merge upstream.

**Quyết định:**

- L0 không sửa; L1 thêm mới; L2 glue tối thiểu.
- Thay provider bằng profile patch (`disabled: true` + row mới).
- `apps/desktop-host` truyền `patchFiles`, shell kiểm soát.

**Hệ quả:** `git diff -- packages/` = 0; conflict chỉ ở L2.

## ADR-0006 — Linux first

**Trạng thái:** accepted

**Bối cảnh:** Linux chưa có desktop; macOS/Windows đang có Electron.

**Quyết định:** Linux lên production trước; macOS/Windows port sau, Electron vẫn ship.

**Hệ quả:**

- Bỏ khỏi critical path: Win32 ACL, Job Object, MoveFileExW, koffi, Authenticode, NSIS, notarization.
- Rủi ro mới: WebKitGTK, Wayland, `.deb`/AppImage, auto-update Linux.
- Phải dùng `cfg` từ đầu để port sau là cộng thêm.

## ADR-0007 — Nạp thẳng Host URL thay custom protocol

**Trạng thái:** accepted

**Bối cảnh:** Electron dùng `dsh-app://app/` + proxy HTTP qua IPC.

**Quyết định:** Tauri nạp `http://127.0.0.1:<port>/?token=…`; Host tự serve SPA, cookie và WS xử lý tự nhiên.

**Hệ quả:**

- Bỏ `protocol.handle`, `forwardWebRequest`, `authenticateWebHost` (~300 dòng).
- Giảm một lớp bug.
- Phải validate origin/token trong mọi command Tauri.

## ADR-0008 — Office + Python tải theo nhu cầu

**Trạng thái:** accepted

**Bối cảnh:** LibreOffice Kit 158 MB + Python ~115 MB chiếm phần lớn bản cài.

**Quyết định:** component manifest (version, sha256), tải khi dùng lần đầu, cài vào `$DSH_HOME/components/`.

**Hệ quả:** core giảm ~273 MB; cần UI quản lý component và xử lý lỗi tải.

## ADR-0009 — Giữ policy engine updater

**Trạng thái:** accepted

**Bối cảnh:** updater hiện có policy phức tạp (mandatory update, task lock, quit inspection).

**Quyết định:** giữ `update-coordinator`, `update-journal`, `mandatory-update-policy`; chỉ thay driver tải/chạy installer.

**Hệ quả:** không phải viết lại policy; mỗi nền tảng chỉ cần driver riêng.

## ADR-0010 — Chẩn đoán có cấu trúc (JSON), không parse log chữ

**Trạng thái:** accepted (P0 khảo sát xong)

**Bối cảnh:** người dùng đề xuất log/debug dạng JSON để dễ test hơn.

**Phát hiện:** Cordis **đã có sẵn** điểm mở rộng. `vendor/cordis/src/logger.ts` định nghĩa:

```ts
interface Exporter { export(message: Message): void }
interface Message { sn: number; ts: number; name: string; type: LoggerType; level: number; args: any[] }
```

`LoggerService` cho đăng ký exporter. Nghĩa là chỉ cần **thêm một exporter NDJSON (L1)**, không phải sửa logger hay đổi call site nào.

**Quyết định:** mọi đầu ra cho test đi qua kênh có cấu trúc, theo thứ tự ưu tiên:

1. **Session event log** — đã là JSONL bền vững, là nguồn sự thật cho transcript.
2. **Kênh điều khiển `@@DSH@@` NDJSON** — trạng thái vòng đời (`ready`/`fatal`/`shutdown`/`session`).
3. **Exporter JSON của Cordis** (mới, L1) — chẩn đoán mức debug/warn/error.
4. Chữ người đọc chỉ dành cho người dùng cuối, **không** dùng làm hợp đồng test.

**Đính chính kỳ vọng:** JSON log **không làm test nhanh hơn**; nó làm test *ít giòn hơn* (assert theo trường thay vì regex) và *dễ chẩn đoán hơn* khi đỏ.

Tốc độ test đến từ kiến trúc client/server, không từ định dạng log:

- Host là tiến trình **sống lâu** phục vụ HTTP + WebSocket. Một Host phục vụ nhiều ca test; cold start ~1 s trả một lần cho cả suite.
- Chạy **headless** đã được chứng minh: `/api/remote.mux` trả 101, Chromium headless render FCP 280 ms, 0 console error, không cần cửa sổ Tauri.
- Ca thuộc agent/tool/session chạy không GUI; chỉ ca thuộc shell (window, tray, dialog native, updater) mới cần display.

**Hệ quả:** P1 thêm exporter NDJSON + harness headless dùng lại Host; không dựng hệ log thứ hai song song với session log.

---

## ADR-0011 — Cầu `window.dshDesktop` do Host phục vụ

**Bối cảnh.** PLAN P2 ban đầu dự kiến chèn init script từ Rust rồi mở Tauri command cho WebView. Ba dữ kiện lật lại:

- Tauri 2 chặn IPC từ origin từ xa; mở được phải khai capability `remote.urls`, tức thêm một bề mặt quyền cho một trang do HTTP phục vụ.
- Logic thật của `dshDesktop` là TypeScript dùng chung với Electron main (`ShortcutPersistence` của `@deepseek-ai/dsh-client-shortcuts/protocol`, `readDeviceInfo`). Viết lại bằng Rust tạo **nhà thứ hai** cho cùng thuật toán.
- Host đã phục vụ chính trang đó qua HTTP có token, và đã có sẵn ba khe: `webServer.register`, `connection.requestRejection`, `webserver/index-inject`.

**Quyết định.** Cầu nằm trong Host, tại `apps/desktop-host/src/desktop-bridge.ts`:

- route `/__dsh-desktop` (prefix) qua `webServer.register`, mỗi request qua `connection.requestRejection` (fence Host/Origin + Peer đã xác thực);
- một row `<head>` qua `webserver/index-inject` cài `window.dshDesktop` và `<html data-platform>` **trước** mọi module client;
- `shortcuts` chạy đúng `ShortcutPersistence` như Electron main, ghi `keybindings.json` bằng `writeFileAtomic`.

**Hệ quả.** Shell Rust không thêm dòng nào; không Tauri command, không capability remote. ADR-0007 (nạp thẳng Host URL) là điều kiện tiên quyết. Hợp đồng TS phía client không đổi.

**Giới hạn v1 (có chủ ý).** `browser` vắng mặt (không có tương đương `WebContentsView`). `keyboard.subscribe`, `shortcuts.subscribe`, `updates.subscribe` trả disposer không bao giờ phát: một cửa sổ, chưa có native accelerator, và client tự áp snapshot từ `get`/`edit`. `updates.status()` trả `idle` cho tới P3.

---

# Phần III — Kiến trúc đích

```
Tauri shell (Rust)
 ├─ Window/WebView → http://127.0.0.1:<port>/?token=…
 ├─ Tray, menu, dialog, deep-link, single-instance
 ├─ Init script inject window.dshDesktop (Tauri IPC)
 └─ Sidecar supervisor (NDJSON stdin/stdout)
        │
        ▼
 Node 24 `dsh-host` (`apps/desktop-host/lib/tauri.js`)
 ├─ runProfile('desktop', --host 127.0.0.1 --port 0 --no-open)
 ├─ web-app bundle serve SPA + /api + WS
 └─ native modules external (pty, system, image)
```

## 3.1 Thay đổi transport

`apps/desktop-host` đổi `process.send`/`process.on('message')` → NDJSON trên stdout/stdin, giữ nguyên schema message:

`ready`, `fatal`, `platform-session`, `quit-inspection`, `update-tasks`, `shutdown-complete`.

## 3.2 Provider swap

```yaml
# fork patch (L1)
- id: subprocess
  disabled: true
- id: subprocess-rust
  name: '@deepseek-ai/dsh-subprocess-native'
```

---

# Phần IV — Ngân sách

## 4.1 Kích thước

Đo thật P0: shell Tauri release **8,8 MB** (vượt mục tiêu 7 MB; `strip` không giảm thêm). Runtime Node 24 stripped **104 MB** thay cho Bun 67 MB.

| Hạng mục | Mục tiêu |
|---|---|
| Tauri shell | ≤ 11 MB (đo: 8,8 MB ở P0, 10,3 MB sau tray/dialog/window-state) |
| Node sidecar (stripped) | ≤ 105 MB |
| Native modules | ≤ 12 MB |
| Web dist | ≤ 25 MB (≤ 10 MB nếu brotli) |
| **Core** | **≤ 150 MB** |
| + Office | +158 MB |
| + Python | +115 MB |

## 4.2 Runtime

| Chỉ số | Ngưỡng |
|---|---|
| RSS idle | ≤ 250 MB |
| RSS sau 30 s | ≤ 400 MB |
| Cold start → first paint | ≤ 1,5 s |
| Cold start → ready | ≤ 3,0 s |
| Soak 1 giờ | RSS tăng ≤ 10 % |

## 4.3 Compile

| Đòn | Mục tiêu |
|---|---|
| `sccache` + cache `target/` | −50…−70 % |
| `lto="thin"` CI, `"fat"` release | −30…−50 % |
| Rust incremental (sccache) | ≤ 60 s |
| Clean Rust build | ≤ 8 phút / platform |

---

# Phần V — Phases

Mỗi phase có **Acceptance Criteria (AC)** dạng checkbox. Không chuyển phase khi còn AC chưa đạt.

## P0 — Architecture probe (Linux)

**Mục tiêu:** chứng minh hoặc bác bỏ kiến trúc Tauri + Bun bằng số đo thật.

**Trạng thái (2026-10-19):** đang chạy. Kiến trúc Tauri **được chứng minh**; Bun **bị bác** (`B8`), đã chuyển sang Node theo `ADR-0002` sửa đổi.

**Ước lượng:** 1–2 pw.

### Công việc

1. ✅ Tạo `apps/desktop-tauri` (Tauri 2, Linux target).
2. ✅ Đổi `apps/desktop-host` sang NDJSON transport; test codec + supervisor (`tests/transport.spec.ts`, 7/7 pass).
3. ✅ Spawn sidecar, đọc `ready`, nạp URL token vào webview.
4. ❌ `bun build --compile`: build được 70 MB nhưng runtime chết vì addon V8 → bỏ.
5. ✅ Đo kích thước shell (8,8 MB), cold start (~1,0–1,2 s), RSS idle (~252–268 MiB), FCP headless 280 ms.
6. ⬜ Dựng differential harness (old vs new).

### Acceptance Criteria

- [x] Cửa sổ Tauri mở, nạp được SPA từ Host — xác nhận bằng 4 kết nối ESTAB của `WebKitNetworkProcess` tới cổng Host.
- [x] `/` trả 200 sau token; `__DSH_BOOT__` có mặt — frame `ready` mang 13 injection, gồm `__DSH_BOOT__`.
- [x] `/api/remote.mux` trả `101 Switching Protocols` — đo thật bằng `curl` upgrade, trả `HTTP/1.1 101`.
- [x] PTY spawn được (`/dev/pts/N`), gõ lệnh trong terminal — đo hai tầng: (a) `terminal/create` qua RPC của Host Tauri trả `/bin/bash -i`, `state=running`; (b) gõ lệnh trong terminal thật của app (Chromium trên đúng Host mà shell Tauri nạp) cho `PTY_OK`, `/dev/pts/2`, `ROWS=50 COLS=76`, và `Ctrl+C` trả lại prompt. **Chưa gõ được xuyên WebView Tauri** vì máy không có công cụ bơm phím an toàn vào cửa sổ (`xdotool`/`wmctrl` đều không có).
- [x] `dsh plugin` cài và chạy được một plugin thật — **đo đủ cả hai nửa** qua shell Tauri thật trên cây production:
  - Cài từ registry: `installBundle @deepseek-ai/dsh-headless@0.2.1-rc.2` → `changed: true`, `stage: "enable"`, `application: "restart-required"`; pnpm đi kèm (11.28.5) chạy bằng Node đi kèm (24.21.0), tải 6 gói.
  - Bộ chặn phiên bản từ chối `latest` (`0.0.1-rc.1`) kèm hướng dẫn `dsh plugin allow-version` — đúng thiết kế.
  - Cài từ tarball cục bộ rồi **chạy**: fixture `apps/desktop-tauri/scripts/load-probe/` (`file:` spec) → khởi động lại thì `apply()` của plugin in `DSH_LOAD_PROBE_ACTIVE`, và `pluginManager/listPlugins` trả `{"entryId":"include:load-probe","moduleName":"@masiss/dsh-load-probe","enabled":true,"fiberPhase":"active","patchId":"load-probe"}`.
  - Fixture là **feature bundle** (insert một row) vì mọi bundle cài được trên registry đều là app bundle; cài thêm app bundle thứ hai làm profile hỏng lúc khởi động (`unknown option '--no-open'`) — luật của sản phẩm, giống nhau ở cả hai shell.
  - **Điều kiện dừng #4 của `AGENTS.md` §7 đã qua.**
- [x] Mở được session ghi bản cũ — đo bằng `session-read-probe.sh` trên đúng 7 file đang có trên đĩa: `session/list` trả `ok=true`, 7/7 item, `format_status={"current":5,"migration-required":2}`. Host Tauri đọc được cả hai thế hệ và báo đúng thế hệ cũ là cần migration thay vì đọc nhầm.
- [ ] Transcript cùng prompt khớp bản cũ (diff = 0) — **chặn**: máy này không có `DEEPSEEK_API_KEY`.
- [x] `node:sqlite` hoạt động dưới Bun — có, nhưng không còn cần cho quyết định runtime.
- [x] Bảng số đo thật được ghi vào `MEMORY.md`.
- [x] Quyết định Go/No-Go cho Bun-compiled — **No-Go**, fallback Node (`ADR-0002`).

### Exit gate

Tất cả AC trên đạt, hoặc có quyết định fallback được ghi lại (Bun runtime / Node) và vẫn đạt các AC chức năng.

---

## P1 — Shell essentials (Linux)

**Mục tiêu:** hành vi desktop ngang Electron ở mức cơ bản.

**Ước lượng:** 2–3 pw.

**Trạng thái (2026-10-19):** lõi đã xong — single instance, window state, đóng cửa sổ = ẩn **có bảo đảm nhìn thấy** (tray hoặc giải thích một lần), tray, menu Application/Edit, thoát qua `quit-inspection`, deep link focus theo argv. Còn: đăng ký scheme `.desktop` (P3), directory picker (P2), bản địa hoá copy của shell (xem ghi chú dưới).

### Công việc

- ✅ Single instance + second-launch focus.
- 🔶 Deep link `dsh://open` — shell focus theo argv; còn đăng ký scheme ở `.desktop` (P3).
- ✅ Window state, hide-on-close, tray (appindicator) với fallback không tray.
- ✅ Application menu, About, DevTools toggle.
- ✅ Theme theo hệ thống.
- ⏭ Native directory picker — chuyển sang P2 cùng bridge.
- ✅ Quit confirmation dùng `quit-inspection` qua NDJSON.
- ❌ Locale bootstrap — xem ghi chú.

### Acceptance Criteria

- [x] Mở app lần hai chỉ focus cửa sổ hiện có — lần 2 tự thoát sau 254 ms, tiến trình 1 sống.
- [ ] `dsh://open` mở/focus — shell đã focus theo argv; còn đăng ký scheme ở `.desktop` (P3).
- [x] Đóng cửa sổ giữ process chạy — `map_state=0`, process và Host còn sống. Có tray ⇒ ẩn im lặng; không có tray ⇒ giải thích một lần rồi mới ẩn (`map_state=2` khi chờ xác nhận). "Mở lại giữ scroll" chưa test được.
- [~] Có task → hỏi, không có task → thoát im lặng — luồng đã viết và protocol đã kiểm (`activeTasks:false`); bấm menu tray chưa test được ở máy này.
- [x] Tray trên X11 — đăng ký `StatusNotifierWatcher` với `Id=tray-icon tray app main`, `Status=Active`, icon thật. Degrade: `build_tray` chỉ log.
- [x] Theme hệ thống đổi → UI đổi theo — `theme(None)` (theo hệ thống); WebKitGTK suy `prefers-color-scheme` từ theme GTK.
- [x] Directory picker — **không cần Tauri dialog**: `packages/bundle/web-app` đã compose `dsh-host-directory-picker-auto`, tự chọn backend `native` (zenity/kdialog trên Linux có display) và cả hai mặt Host/client được mount như entry thường. Xem mục P2.
- [x] Menu Application (About, dev: Reload + DevTools `F12`, Quit `Ctrl+Q`) và menu Edit (undo/redo/cut/copy/paste/select-all) — ảnh chụp release xác nhận.
- [ ] Bản địa hoá copy của shell — **còn thiếu**, có lý do: `apps/desktop/src/locale.ts` là nguồn duy nhất và là TypeScript; chép sang Rust sẽ tạo nhà thứ hai cho cùng chuỗi.

### Exit gate

Không còn regression hành vi desktop so với Electron trong phạm vi trên.

---

## P2 — Desktop bridge

**Mục tiêu:** `window.dshDesktop` hoạt động, client dùng được tính năng desktop.

**Ước lượng:** 2–3 pw.

**Trạng thái (2026-10-19):** tầng cầu đã chạy trong app thật (ADR-0011). Còn lại: welcome/onboarding, account/Platform handoff, product analytics, và input menu kiểu native.

### Công việc

- ✅ Init script inject `window.dshDesktop` (`protocolVersion: 1`) — row `<head>` do Host đóng góp.
- 🔶 `deviceInfo` ✅, `keyboard` ✅ (subscribe rỗng), `shortcuts` ✅, `updates` ✅ (`idle`; updater thật ở P3).
- ❌ Welcome/onboarding — **không tới được ở v1**, xem ghi chú dưới.
- ✅ Account/Platform session handoff — đã chạy trong Host (`installPlatformSessionPublisher`); consumer duy nhất là Platform view nhúng, thứ đã bị loại.
- ✅ Product analytics chỉ bật khi có `dshDesktop` — cổng nằm sẵn ở client (`'dshDesktop' in globalThis`), cầu của ta thoả nó.

### Welcome/onboarding — vì sao chưa làm được

Cửa sổ welcome của Electron **không phải** trang app: `welcome-window.ts` mở một tài liệu riêng (`renderer/welcome.html`, preload riêng `preload-welcome.cjs`, kênh `WELCOME_IPC` riêng), với `WelcomeOperations` gồm `startSignIn`/`cancelSignIn`/`copySignInLink` (luồng đăng nhập Platform) và `saveApiKey`. Dựng lại ở Tauri cần: một cửa sổ thứ hai + asset welcome, một bộ operations mới, và **copy welcome do `apps/desktop/src/locale.ts` sở hữu** — TypeScript, shell Rust không import được (cùng lý do đã ghi cho bản địa hoá ở P1).

Overlay onboarding *trong app* thì có, nhưng điều kiện hiển thị là `signedIn && !done` với `signedIn = account.status === 'credential-stored'` — tức phải đăng nhập Platform trước. Nó cần `dshOnboarding.hasApiKey()`, và fact đó (được upstream tính trong `welcome-backend.ts` bằng ba lượt RPC `settings/describe` + `llm/listConfigurableProviders` + `credentials/describe`) chép sang Host sẽ là **nhà thứ hai** cho cùng thuật toán. Chưa chép.

**Hành vi thực tế ở v1 (đã đo):** home mới, không API key, chưa đăng nhập ⇒ **không** overlay, composer hiện ra, `console_errors=0` — người dùng vào thẳng workspace và cấu hình ở Settings → Models. Đây đúng bằng hành vi của `dsh web`. Khác biệt duy nhất so với Electron: người đã có key từ nguồn khác nhưng chưa từng hoàn tất onboarding sẽ thấy một màn hình giới thiệu **một lần**, bỏ qua được (dấu hoàn tất được lưu).

**Hướng đúng nếu làm tiếp:** đừng chép choreography. Cho client tự trả lời — một provider `dshOnboarding` do client dựng từ `ctx.remote` (chỗ `ui-settings-models` đã làm đúng việc này), hoặc phát chuỗi đã bản địa hoá từ một nguồn dùng chung.

### Acceptance Criteria

- [x] `'dshDesktop' in globalThis` = true trong app Tauri — probe trang: `present=true protocol=1 keyboard=true`.
- [x] Settings → Shortcuts lưu/khôi phục được — vòng `get`→`edit`→`saved` qua `keybindings.json`, và lần mở sau đọc lại đúng binding. **Chưa điều khiển thao tác trong UI Settings** (đường UI gọi đúng adapter đã kiểm).
- [x] `updates.status()` trả phase hợp lệ — `{"phase":"idle"}`.
- [ ] Welcome hiển thị khi chưa có API key; vào workspace sau khi cấu hình — **không tới được ở v1** (cửa sổ welcome là tài liệu native riêng, cần copy do TS sở hữu và luồng đăng nhập Platform). Hành vi thay thế đã đo: vào thẳng workspace, cấu hình ở Settings, ngang `dsh web`.
- [x] Không có lỗi console "desktop API missing" — `console_errors=0 page_errors=0`.
- [x] Cầu từ chối origin/token không hợp lệ — không cookie ⇒ 401; GET ⇒ 405; body quá lớn ⇒ 413; method lạ ⇒ 404.

### Exit gate

Bridge contract TS không đổi — đã giữ: chỉ thêm backend Host, không sửa `packages/client/*`.

---

## P3 — Updater + signing

**Mục tiêu:** update an toàn, giữ nguyên policy.

**Ước lượng:** 3–4 pw (Linux + macOS + Windows).

### Công việc

- Tách `UpdateInstallerDriver` khỏi policy.
- Linux: AppImage self-update hoặc repo apt (chọn ở Q3).
- macOS: Developer ID, notarize, DMG.
- Windows: Authenticode, NSIS/MSI.
- Manifest thống nhất: version, sha256, platform, arch, signature.

### Acceptance Criteria

- [ ] Update N → N+1 trên Linux sạch.
- [ ] Mandatory update chặn đúng theo policy.
- [ ] Có task chạy → update hỏi và không mất task.
- [ ] Journal ghi đủ bước, phục hồi được sau crash.
- [ ] macOS: `codesign --verify --deep --strict` + `spctl -a` pass.
- [ ] Windows: `signtool verify /pa` pass.
- [ ] Manifest từ chối sai platform/arch/hash.

### Exit gate

Update không làm mất dữ liệu và không cài bản sai nền tảng.

---

## P4 — Packaging + release

**Mục tiêu:** artifact cài được trên máy sạch.

**Ước lượng:** 3–4 pw.

### Công việc

- Linux `.deb` + AppImage; khai báo dependency WebKitGTK.
- macOS `.app`/`.dmg`, universal.
- Windows NSIS/MSI (không port installer UI tùy biến ở v1).
- CI matrix 5 target; cache Rust.
- Resource layout sẵn sàng ký lồng nhau.
- **Vòng đời profile trong shell Tauri** — hiện làn đóng gói mới tạo `project/` một lần, còn Electron tạo/sửa nó lúc khởi động (`DesktopProjectManager.applyRelease`: `initProfile`, `migrateProfileSettings`, `removeLinkProjections`) và cứu được profile hỏng (`disableAllPlugins`). Thiếu đường này thì một profile hỏng là app không mở lại được. Chỗ đặt hợp lý là Host (TypeScript dùng lại `@deepseek-ai/dsh-app-boot`), không phải Rust.
- **Python/Office tải theo nhu cầu** (ADR-0008): payload hiện 401 MB, riêng Python 257 MB. Đo được: bỏ `dependencies/python` thì Host vẫn `ready=OK`, plugin vẫn chạy, RSS không đổi — nên cắt khỏi payload mặc định là an toàn, nhưng phải có cơ chế tải khi người dùng cần tính năng tài liệu, nếu không là mất tính năng.

### Acceptance Criteria

- [ ] Cài trên Ubuntu 24.04 sạch, mở app, chat chạy.
- [ ] Cài trên Fedora sạch.
- [ ] macOS: mở từ Applications, không bị Gatekeeper chặn.
- [ ] Windows: cài, mở, PTY chạy.
- [ ] Uninstall không để lại file ngoài `$DSH_HOME` do user tạo.
- [ ] CI build đủ 5 target trong ngân sách thời gian.
- [ ] Size gate fail nếu vượt ngân sách.

### Exit gate

Cài → chạy → update → gỡ, tất cả trên máy sạch.

---

## P5 — Rust native crate

**Mục tiêu:** bỏ koffi/node-pty/C addon, thay bằng Rust sau seam.

**Ước lượng:** P5-posix 2–3 pw; P5-win32 5–7 pw.

### P5-posix (làm cùng Linux)

- `flock` (C → `rustix`).
- `landlock-run` (C → `landlock`).
- `execve` (koffi → `nix`).
- PTY (node-pty → `portable-pty`).
- Directory picker (→ `rfd`).
- Image (sharp/libvips → `image` + `fast_image_resize`).

### P5-win32 (chỉ khi Windows vào phạm vi)

- `win32-process` (1.185 dòng) → crate `windows`.
- `sandbox-windows-acl` (2.183 dòng) → crate `windows`.
- `fs-local/win32.ts`, `session-persistence-jsonl/win32.ts`, `windows-inspector.ts`.
- Xoá `koffi`.

### Acceptance Criteria (P5-posix)

- [ ] Không còn `require('koffi')` cho Linux.
- [ ] PTY parity: resize, signal, exit code.
- [ ] Landlock enforcement giữ nguyên test.
- [ ] Image output khớp sharp trong sai số cho phép (hoặc test so sánh).
- [ ] Provider parity suite xanh.
- [ ] `git diff -- packages/` = 0.

### Acceptance Criteria (P5-win32)

- [ ] DACL/token khớp bản koffi trên fixture.
- [ ] Job Object kill được process tree.
- [ ] `MoveFileExW` atomic behavior giữ nguyên.
- [ ] Không còn `koffi` trong `packages/`.
- [ ] Windows CI xanh trước khi xoá code cũ.

### Exit gate

Provider Rust pass parity; code cũ chỉ bị xoá sau một chu kỳ phát hành.

---

## P6 — Payload slimming

**Mục tiêu:** đạt core ≤ 105 MB.

**Ước lượng:** 2–3 pw.

### Công việc

- Component manager cho Office/Python.
- Prune production tree.
- Brotli cho Web dist.
- Ripgrep on-demand.
- CI size budget.

### Acceptance Criteria

- [ ] Core ≤ 105 MB.
- [ ] Office cài lần đầu chạy được, có progress + lỗi rõ ràng.
- [ ] Python cài lần đầu chạy được.
- [ ] Không có Office/Python trong bundle core.
- [ ] Size gate trong CI.
- [ ] `bun run test:web:built` vẫn pass.

---

# Phần VI — Rollout theo nền tảng

| Giai đoạn | Linux | macOS | Windows |
|---|---|---|---|
| v1 | Tauri | Electron | Electron |
| v2 | Tauri | Tauri | Electron |
| v3 | Tauri | Tauri | Tauri |

**Cam kết trung lập nền tảng:**

- [ ] Crate native `#[cfg(target_os)]` từ đầu.
- [ ] Updater tách policy/driver.
- [ ] Manifest update thống nhất.
- [ ] Không hardcode path Linux.
- [ ] Bridge contract TS.
- [ ] Resource layout ký được.

---

# Phần VII — Ma trận rủi ro

| # | Rủi ro | Trace | Fix | Fallback |
|---|---|---|---|---|
| 1 | Bun compiled resolve sai `main`/`exports` | Minimal repro | Resolve absolute path (repo đã làm) | Bun runtime |
| 2 | `import.meta.resolve` → `$bunfs` | Test | Audit 53 file | Bun runtime |
| 3 | `node:sqlite` thiếu | `bun -e` | shim/`bun:sqlite` | Node |
| 4 | Loader plugin trong binary | `dsh plugin` thật | #1 | Bun runtime |
| 5 | Tauri IPC remote origin | DevTools + log | capability + validate | — |
| 6 | Đổi origin | Grep client | bridge độc lập origin | — |
| 7 | Win32 ACL/token | Differential test | Port 1:1 | Giữ koffi |
| 8 | `MoveFileExW` atomicity | Crash test | `ReplaceFileW` | Giữ koffi |
| 9 | Ký binary lồng nhau | `codesign`/`signtool` | Ký đệ quy | — |
| 10 | Embedded browser | Chạy thử | Không đầy đủ | Tắt v1 |
| 11 | WebKitGTK thiếu API | Web e2e | Polyfill | Degrade/bỏ Linux |
| 12 | Wayland tray/shortcut | Chạy Wayland | Portal/appindicator | Không tray |
| 13 | `.deb` dependency | Máy sạch | Khai báo dependency | AppImage |
| 14 | Auto-update Linux | Journal | Chọn driver | Repo apt |
| 15 | Không giảm size | Đo P0 | — | Quyết định dừng |
| 16 | Cài máy sạch fail | VM CI | Gate | — |

---

# Phần VIII — Điều kiện nghiệm thu tổng

Chỉ coi là hoàn thành khi **tất cả** đạt:

| # | Điều kiện | Cách kiểm |
|---|---|---|
| A1 | 10 bất biến giữ nguyên | gate §6 `AGENTS.md` |
| A2 | Không sửa `packages/` upstream | `git diff upstream/master...HEAD -- packages/` rỗng |
| A3 | Core ≤ 105 MB | CI size gate |
| A4 | RSS idle ≤ 250 MB | benchmark |
| A5 | Cold start → first paint ≤ 1,5 s | benchmark |
| A6 | Update N→N+1 pass trên 3 nền tảng | e2e máy sạch |
| A7 | Plugin thật cài + chạy | e2e |
| A8 | Session cũ mở được | e2e |
| A9 | Transcript diff = 0 | snapshot/expected |
| A10 | Không còn Electron trong app mới | kiểm tra bundle |

---

# Phần IX — Bảo trì fork

- `git fetch upstream && git merge upstream/master` định kỳ.
- `MEMORY.md` ghi mọi file L2 đã sửa.
- CI gate: `git diff --name-only upstream/master...HEAD -- packages apps/desktop` phải rỗng (cho phép danh sách L2).
- Không xoá `apps/desktop` cho đến khi Windows/macOS port xong.
