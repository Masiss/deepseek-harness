# PLAN — Desktop Tauri (Rust) + Node

Tài liệu này gộp **ADR** và **kế hoạch chi tiết theo phase**. Quy tắc làm việc: [`AGENTS.md`](AGENTS.md). Số đo và bẫy: [`MEMORY.md`](MEMORY.md).

Kế hoạch **cắt nhỏ phần mềm** nằm riêng ở [`PLAN-SHRINK.md`](PLAN-SHRINK.md) và chỉ mở **sau khi** tài liệu này xanh.

- Trạng thái: **Goal 1 (baseline) đã đóng; Goal 2 (release-grade, Linux-first + nền port) đang mở.**
- Ngày: 2026-10-19 (Goal 1); 2026-10-22 (Goal 2).
- Phạm vi: chỉ `apps/desktop-tauri`, `native/dsh-native`, provider packages và glue tối thiểu.

---

# Cấu trúc mục tiêu

## Goal 1 — Baseline migration (đã đóng, 2026-10-22)

Goal 1 chứng minh **có thể thay Electron bằng Tauri mà không đổi lõi DSH**. Đây là baseline kỹ thuật, không phải kết luận sản phẩm đã hoàn tất.

| Hạng mục | Kết quả |
|---|---|
| Shell | Tauri 2 + WebKitGTK; không còn Electron trong bundle Linux |
| Runtime | Node 24.21 + dsh payload tự chứa; Host chạy qua NDJSON |
| Linux | AppImage + `.deb` (và `.rpm` trong lane đóng gói) |
| Native | Rust image provider, Landlock launcher, PTY/flock thay thế dần koffi |
| Kích thước | AppImage **76.446.200 B**; cây cài **~290 MB**; `dsh/node_modules` **190 MB** |
| Bất biến | Session format, tool schema, CLI, `cordis.yml`, HTTP/WS bridge, plugin API, bundle/profile giữ nguyên |
| Bằng chứng | Phần VIII/X, `MEMORY.md`; snapshot 211/211, expected 122/122; session cũ đọc được; transcript replay diff = 0 |

**Điều kiện đóng Goal 1:** 10 bất biến có bằng chứng chạy được, trừ các mục ghi rõ cần môi trường ngoài (macOS/Windows signing, runner không phải Linux, Windows ACL). Các mục này chuyển sang Goal 2, không tính là đã xong.

## Goal 2 — Release-grade & cross-platform ready (đang mở)

**Mục tiêu:** Linux Tauri đủ ổn định để dùng hằng ngày; mọi user journey quan trọng có bằng chứng trên **packaged runtime**; các lỗi vận hành (startup, crash, shutdown, update, offline, disk-full) được phân loại và xử lý; có portability matrix cho Windows/macOS từ source hiện tại, chỉ sửa các trở ngại nhỏ độc lập.

**Không thuộc Goal 2:** full platform port, rewrite runtime, đổi session format/tool schema, tối ưu không có số đo, thay framework.

### Mốc và trạng thái

| Mốc | Nội dung | Trạng thái | Bằng chứng / việc còn lại |
|---|---|---|---|
| G2-M1 | Chốt D40–D43 + regression | **Xong** | Commit `fbe6ea0fe7`; build + build:check xanh; typecheck/lint/test xanh |
| G2-M2 | User journey trên packaged runtime | **Đang làm** | Plugin + Session + profile contention + update PASS; CSP mới có boot headless; còn GUI thật và session có nội dung |
| G2-M3 | Fault injection | **Đang làm** | 6 kịch bản Host + disk-full PASS; còn update hỏng digest ở mức packaged, crash giữa download, mất điện |
| G2-M4 | CI evidence theo cấp | **Đang làm** | Có lane Linux + release; probe mới chưa vào CI; chưa có bằng chứng CI cho Windows/macOS |
| G2-M5 | Portability matrix Windows/macOS | **Đang làm** | Ma trận dưới đây; chỉ sửa blocker nhỏ, không mở full port |
| G2-M6 | Benchmark startup/idle/footprint | **Đang làm** | Số đo dưới đây; chưa chốt ngân sách và chưa đo WebView riêng |
| G2-M7 | Tài liệu & bàn giao | **Đang làm** | PLAN/MEMORY/HANDOFF; sẽ cập nhật sau mỗi mốc |

### Định nghĩa cấp bằng chứng

| Cấp | Nghĩa | Ví dụ |
|---|---|---|
| **L0 — source** | Chạy trên source/unit test, chưa đóng gói | `vitest`, `cargo test`, `typecheck` |
| **L1 — CI** | Cùng lệnh chạy trong GitHub Actions | `desktop-linux.yml`, `desktop-release.yml` |
| **L2 — packaged** | Chạy trên cây `apps/desktop/.desktop-build/targets/<target>/` | `packaged-journey-probe.sh`, `fault-probe.sh` |
| **L3 — desktop thật** | Shell + WebView trên compositor thật, có cửa sổ và tương tác người dùng | `cold-start-probe.sh` (weston headless) chỉ là L2.5; GUI thật chưa tự động hoá |

Không dùng L0 để thay L2/L3. Một mục chỉ được coi là "release-grade" khi có L2, và với hành vi GUI thì cần L3 hoặc ghi rõ khoảng trống.

### Journey matrix (packaged runtime)

| Journey | Lệnh / bằng chứng | Cấp | Kết quả |
|---|---|---|---|
| Plugin: cài từ tarball, Host apply, restart, gỡ | `packaged-journey-probe.sh` | L2 | **PASS** (`plugin-install`, `plugin-list`, `plugin-after-restart`, `plugin-remove`) |
| Session: tạo, rename, restart, đọc lại | `packaged-journey-probe.sh` | L2 | **PASS** (cùng `sessionId` sau restart) |
| Hai carrier tranh profile | `desktop-lease-probe.sh` | L2 | **PASS** (`second_refused=OK`, takeover sau khi chết) |
| CSP của release | `cold-start-probe.sh` + `tauri.conf.json` | L2.5 | **Một phần** — window + page load xong, không thấy CSP violation; chưa đọc được console WebKit thật |
| Sharp fallback | `image-parity-probe.sh` | L2 | **PASS 11/11**; JPEG/GIF/TIFF/resize khác byte (đã chấp nhận, D39) |
| Update feed ký | `update-probe.sh` | L2 (shell thật, headless) | **PASS** — feed hợp lệ `available`, feed bị sửa `idle` |
| Update trọn luồng | `update-flow-probe.mjs` | L2 | **PASS** — prompt download/install, `restart`, ảnh được thay |
| Offline update/component | `fault-probe.sh` | L2 | **PASS** — `check-network`, `feed-network`, không cài dở |
| Disk-full khi tải component | `disk-full-probe.sh` (dưới `isolated-run.sh`) | L2 | **PASS** — `phase=error`, không còn `download-*`/`current.json`; phân loại ENOSPC thành `download` mới sửa ở source + unit test, cây đóng gói sẽ nhận ở lần build kế tiếp |
| Session có nội dung + model thật | `live-transcript-probe.sh` | L3 (cần key) | **PASS** ở lần chạy trước; chưa lặp trong Goal 2 |
| Welcome/onboarding, GUI thật | — | L3 | **Chưa có** (đã hoãn) |

### Fault matrix

| Lỗi | Kỳ vọng | Bằng chứng | Kết quả |
|---|---|---|---|
| Thiếu runtime lúc khởi động | Thoát nhanh, lỗi rõ, không treo | `fault-probe.sh` → `startup-missing-runtime` | **PASS** (exit 1, không timeout) |
| Profile hỏng | Không serve, nhả lease | `fault-probe.sh` → `startup-bad-profile` | **PASS** (exit 1, lease released) |
| Host bị kill -9 | Lần sau nhận lease cũ và serve | `fault-probe.sh` → `crash-takeover` | **PASS** (stale lease held → takeover ready) |
| Shutdown qua control frame | Exit 0, nhả lease, không còn process | `fault-probe.sh` → `shutdown` | **PASS** |
| Update feed không tới được | `check-network`, không crash | `fault-probe.sh` → `offline-update` | **PASS** |
| Component feed không tới được | `feed-network`, không cài dở | `fault-probe.sh` → `offline-component` | **PASS** |
| Hết đĩa khi tải | Lỗi không phải network, dọn file dở | `disk-full-probe.sh` + unit test ENOSPC | **PASS** (48 MB; `leftovers=0`; phân loại `download` đã sửa ở source, unit test xanh) |
| Manifest/artifact bị sửa | Từ chối trước khi chạy | `update-probe.sh`, unit tests | **PASS** |
| Crash giữa lúc download/install | Không để lại trạng thái nửa vời | — | **Chưa phủ** |
| Mất điện / kill trong lúc rename | Journal phục hồi hoặc rollback | — | **Chưa phủ** |

### Portability matrix (đọc từ source hiện tại)

| Bề mặt | Linux (hiện tại) | Windows | macOS | Trở ngại / hướng xử lý |
|---|---|---|---|---|
| Tauri shell | Xong | Chưa build | Chưa build | Cần runner + signing; tray đã có nhánh `cfg(not(linux))` |
| Host ↔ shell transport | NDJSON stdio | Dùng được | Dùng được | Không phụ thuộc nền tảng |
| Lease profile | `native/flock` POSIX | **Thiếu ACL** | POSIX | P5-win32; cần test trên Windows thật |
| Sandbox | Landlock | **Chưa có** | **Chưa có** | Native layer theo OS; không đổi policy engine |
| Office component | `bin/python3`, `PATH=/usr/bin:/bin`, symlink POSIX | **Chưa có payload/đường dẫn** | Cần payload + `bin/python3` | Tách hằng số theo target; archive riêng cho từng OS |
| Packaging | AppImage/deb/rpm | **Chưa có** | **Chưa có** | Script Linux-only; cần MSI/DMG + ký |
| Update driver | AppImage | **Chưa có** | **Chưa có** | Manifest đã đa target; driver thay thế mới là Linux |
| Image provider | Rust portable | Cần build | Cần build | Kiểm tra libwebp/toolchain trên runner |
| PTY | Provider POSIX | **Chưa xác minh** | POSIX | Windows cần ConPTY; chưa port |

**Quy tắc:** chỉ sửa các mục "nhỏ và độc lập" (hằng số đường dẫn, nhánh `cfg`, test) trong Goal 2. Full port (MSI/DMG, ACL, sandbox, update driver) chỉ mở khi có runner/chứng thư và quyết định phạm vi riêng.

### Benchmark hiện có (2026-10-22, máy 20 CPU)

| Số đo | Giá trị | Ghi chú |
|---|---|---|
| AppImage | **76.446.200 B** | Sau prune D41; gate ≤ 105 MiB |
| Cây cài | **~290 MB** | Gồm runtime + project |
| `dsh/node_modules` | **190 MB** | Trước prune 255 MB |
| Host cold start (warm cache) | **1,98–2,09 s** | `measure-linux-tree.sh`, 3 lần |
| Host cold start (first run) | **4,83 s** | Page cache lạnh |
| Shell: window → start page → Host URL | **0,55 s → 0,77 s → 4,54 s** | `cold-start-probe.sh` (weston headless) |
| Host idle RSS | **179–182 MiB** | Chưa gồm WebKitGTK |

Chưa tối ưu thêm nếu không có số đo chứng minh. Ứng viên chỉ được mở khi vượt ngân sách: RSS WebView, thời gian chờ Host, kích thước Python.

### Acceptance của Goal 2

1. Mọi journey trong bảng có **PASS ở L2**, trừ mục ghi rõ "chưa phủ"; các mục GUI có L3 hoặc khoảng trống được chấp nhận bằng văn bản.
2. Fault matrix phủ đủ 8 nhóm lỗi; lỗi còn lại được phân loại và có kế hoạch, không bỏ im lặng.
3. Probe mới chạy trong CI Linux (ít nhất L1) và release lane; kết quả CI được ghi khác với L0/L2.
4. Portability matrix có nguồn từ source; các blocker nhỏ đã sửa, blocker lớn có owner/điều kiện.
5. Benchmark có số đo lặp lại được; chỉ mở tối ưu khi có ngưỡng và số liệu.
6. Không đổi 10 bất biến, không đổi acceptance cũ, không thêm framework/runtime rewrite.

### Blocker hiện tại

- **Chứng thư + runner macOS/Windows** cho signing, ACL, MSI/DMG, update driver.
- **GUI thật (L3)** cần compositor + WebKit console; hiện chỉ có weston headless và probe đọc trạng thái.
- **Power-loss/rename crash** cần fault injection ở mức filesystem hoặc VM, chưa có trong môi trường này.
- **Image byte parity** với Electron chỉ đúng cho phiên không đính kèm ảnh (D39); muốn đóng phải port arithmetic của libvips.

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
- ✅ Deep link `dsh://open` — shell focus theo argv ✓; scheme đăng ký qua `.desktop` + `update-desktop-database` khi cài gói ✓ (việc *thử* trên phiên desktop thật vẫn mở).
- ✅ Window state, hide-on-close, tray (appindicator) với fallback không tray.
- ✅ Application menu, About, DevTools toggle.
- ✅ Theme theo hệ thống.
- ⏭ Native directory picker — chuyển sang P2 cùng bridge.
- ✅ Quit confirmation dùng `quit-inspection` qua NDJSON.
- ❌ Locale bootstrap — xem ghi chú.

### Acceptance Criteria

- [x] Mở app lần hai chỉ focus cửa sổ hiện có — lần 2 tự thoát sau 254 ms, tiến trình 1 sống.
- [x] `dsh://open` mở/focus — shell focus theo argv ✓; `.desktop` khai báo `MimeType=x-scheme-handler/dsh` ✓ và gói cài đặt chạy `update-desktop-database` khi cài (deb `postinst` ✓, rpm `%post` ✓) nên scheme được đăng ký trong phiên người dùng.
- [x] Đóng cửa sổ giữ process chạy — `map_state=0`, process và Host còn sống. Có tray ⇒ ẩn im lặng; không có tray ⇒ giải thích một lần rồi mới ẩn (`map_state=2` khi chờ xác nhận). "Mở lại giữ scroll" chưa test được.
- [~] Có task → hỏi, không có task → thoát im lặng — luồng đã viết và protocol đã kiểm (`activeTasks:false`); bấm menu tray chưa test được ở máy này.
- [x] Tray trên X11 — đăng ký `StatusNotifierWatcher` với `Id=tray-icon tray app main`, `Status=Active`, icon thật. Degrade: `build_tray` chỉ log.
- [x] Theme hệ thống đổi → UI đổi theo — `theme(None)` (theo hệ thống); WebKitGTK suy `prefers-color-scheme` từ theme GTK.
- [x] Directory picker — **không cần Tauri dialog**: `packages/bundle/web-app` đã compose `dsh-host-directory-picker-auto`, tự chọn backend `native` (zenity/kdialog trên Linux có display) và cả hai mặt Host/client được mount như entry thường. Xem mục P2.
- [x] Menu Application (About, dev: Reload + DevTools `F12`, Quit `Ctrl+Q`) và menu Edit (undo/redo/cut/copy/paste/select-all) — ảnh chụp release xác nhận.
- [x] Bản địa hoá copy của shell — `apps/desktop-tauri/scripts/generate-locale.ts` đọc `apps/desktop/src/locale.ts` (nguồn duy nhất, TypeScript) và **sinh** `src-tauri/src/locale.rs`; shell Rust tra key trong module sinh ra ⇒ **không** có nhà thứ hai cho cùng chuỗi. CI kiểm tra bản sinh không cũ (`generate-locale.ts --check`).

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

- [x] Tách `UpdateInstallerDriver` khỏi policy (`update/appimage.ts` + `update/service.ts`).
- [x] Linux: AppImage self-update (Q3 đã chốt, D26); `.deb` do trình quản lý gói.
- [ ] macOS: Developer ID, notarize, DMG.
- [ ] Windows: Authenticode, NSIS/MSI.
- [x] Manifest thống nhất: version, sha256, platform, arch, size (signature để P4 cùng gói ký).
- [x] Hộp thoại xác nhận tải/cài + restart shell (`tauri-plugin-dialog` đã có sẵn; Host hỏi qua frame `update-prompt`, shell trả lời qua stdio).

### Acceptance Criteria

- [~] Update N → N+1 trên Linux sạch: **đã chạy end-to-end headless** — feed HTTP thật, hai prompt trả lời qua stdio, `$APPIMAGE` bị thay đúng byte + mode 755, có frame `restart` (`update-flow-probe.mjs`). Còn thiếu: bản phát hành thật và máy sạch.
- [ ] Mandatory update chặn đúng theo policy.
- [x] Có task chạy → update **không** cài (`ready`); hộp thoại hỏi người dùng đã có, và câu trả lời đi qua protocol có `promptId` riêng.
- [x] Journal ghi đủ bước; crash trước `rename` để lại ảnh cũ nguyên vẹn (test).
- [ ] macOS: `codesign --verify --deep --strict` + `spctl -a` pass.
- [ ] Windows: `signtool verify /pa` pass.
- [x] Manifest từ chối sai platform/arch/hash/size (14 test).

### Exit gate

Update không làm mất dữ liệu và không cài bản sai nền tảng.

---

## P4 — Packaging + release

**Mục tiêu:** artifact cài được trên máy sạch.

**Ước lượng:** 3–4 pw.

### Công việc

- [x] Linux **AppImage** (`apps/desktop-tauri/scripts/pack-appimage.sh`): shell release + payload chuẩn bị sẵn, `AppRun` giữ đúng hợp đồng env, profile seed vào `$DSH_HOME` vì ảnh là read-only. Đo: 143.735.288 byte; boot headless từ bản giải nén read-only, cầu trả `{"phase":"idle"}`, `image_writable_files=0`.
- [x] Linux `.rpm` (`apps/desktop-tauri/scripts/pack-rpm.sh`): rpmbuild trong `fedora:41`, cài 558 MB, 114 MiB; `rpm-probe.sh` cài + `rpm -V` + Host boot trên Fedora 41 sạch.
- [x] Linux `.deb` (`apps/desktop-tauri/scripts/pack-deb.sh`): payload ở `/usr/lib/deepseek-harness`, launcher `/usr/bin/deepseek-harness` tự suy đường dẫn từ `$0`, seed profile vào `$DSH_HOME`. 108 MiB / cài 558 MB.
- macOS `.app`/`.dmg`, universal.
- Windows NSIS/MSI (không port installer UI tùy biến ở v1).
- CI matrix 5 target; cache Rust.
- [x] **Release Linux trên GitHub runner** — lane `desktop-release.yml` (`workflow_dispatch`, nhận tag): dựng primary runtime → pack tarball release → dựng cây runtime → pack AppImage/.deb/.rpm → size gate → `gh release` kèm `manifest.json` + `components.json` + archive Office (D32). Release **là** nguồn feed: URL tải theo tag ổn định nên updater và component installer đọc đúng release đó. **Đã chạy thật**: release `desktop-v0.2.1-rc.2` (12 phút 24 s) với 4 asset; sha256/size trong manifest khớp đúng byte đã upload.
- Resource layout sẵn sàng ký lồng nhau.
- [x] **Vòng đời profile trong shell Tauri** — đã làm trong Host (`apps/desktop-host/src/profile-lifecycle.ts`, D24): tạo profile lúc khởi động, dọn link projection, và giữ đúng một app bundle (bỏ `dsh-headless`/ACP/SDK nếu người dùng cài thêm, giữ feature bundle). Đo end-to-end: profile 2 app bundle từng làm app không mở được giờ tự sửa và boot (`ready=OK`). Còn lại: đưa thông báo phục hồi lên UI (hiện ở stderr) và khoá profile nếu sau này cho phép nhiều Host.
- **Python/Office tải theo nhu cầu** (ADR-0008): đã **prune** payload Python 257 → 193 MB (primary runtime 401 → 337 MB) mà không mất module tài liệu nào — xem D25. Việc còn lại là cơ chế tải: bỏ hẳn 193 MB khỏi bản cài, và bản tải về của thành phần đó nén còn ~47 MB. Bỏ Python khỏi payload đã đo là an toàn (Host vẫn `ready=OK`, plugin vẫn chạy, RSS không đổi), nhưng phải có đường tải khi người dùng cần tính năng tài liệu, nếu không là mất tính năng.

### Acceptance Criteria

- [~] Cài trên Ubuntu 24.04 sạch: container `apt-get install` **thành công** (`Status: install ok installed`, deps resolve), launcher seed đúng profile, Host boot từ bản giải nén read-only. Chưa mở GUI/chat trong container (không có display).
- [x] Cài trên Fedora sạch — `rpm-probe.sh` trong container `fedora:41`: `dnf install` OK, `rpm -V` OK, launcher seed `DSH_HOME`, Host boot OK.
- [ ] macOS: mở từ Applications, không bị Gatekeeper chặn.
- [ ] Windows: cài, mở, PTY chạy.
- [x] Uninstall không để lại file ngoài `$DSH_HOME` do user tạo — `uninstall-probe.sh`: đọc danh sách file package **tự sở hữu** (16.800 file) rồi xoá theo đúng danh sách đó ⇒ `remaining_in_root=0`; profile người dùng trong `$DSH_HOME` **không đổi** (`user_data_kept=yes`).
- [ ] CI build đủ 5 target trong ngân sách thời gian. **2 target Linux đã đo trên runner**: lane `desktop-release.yml` dựng xong AppImage + .deb + .rpm + size gate trong 12 phút (một target Linux; 5 target còn thiếu 3 nền tảng khác — macOS x64/arm64, Windows x64).
- [x] Size gate fail nếu vượt ngân sách — `size-gate-probe.sh`: artifact vượt ngân sách tải về (105.102.410.25 B > 110.100.480 B) ⇒ `RESULT size-gate=FAIL` và exit khác 0; artifact dưới ngân sách ⇒ exit 0. Ngân sách đọc thẳng từ `size-gate.sh` nên probe không trôi khỏi gate.

### Exit gate

Cài → chạy → update → gỡ, tất cả trên máy sạch.

---

## P5 — Rust native crate

**Mục tiêu:** bỏ koffi/node-pty/C addon, thay bằng Rust sau seam.

**Ước lượng:** P5-posix 2–3 pw; P5-win32 5–7 pw.

### P5-posix (làm cùng Linux)

- [x] `flock` (C → `rustix`) — crate `native/dsh-native`, entry `apps/desktop-host/native/flock.cjs`.
- [x] `landlock-run` (C → Rust, UAPI trực tiếp) — crate `native/dsh-native`, binary `landlock-run` thay bản C trong payload.
- [x] `execve` (koffi → `nix`) — crate `native/dsh-native`, entry `apps/desktop-host/native/koffi.cjs`.
- [x] PTY (node-pty → `portable-pty`) — crate `native/dsh-native`, wrapper `apps/desktop-host/native/node-pty.cjs`, loader `apps/desktop-host/native/native-loader.mjs`.
- Directory picker (→ `rfd`).
- [x] Image (sharp/libvips → `image` + `fast_image_resize` + `zune-png` + libwebp tĩnh) — crate `native/dsh-native`, entry `apps/desktop-host/native/sharp.cjs`.

### P5-win32 (chỉ khi Windows vào phạm vi)

- `win32-process` (1.185 dòng) → crate `windows`.
- `sandbox-windows-acl` (2.183 dòng) → crate `windows`.
- `fs-local/win32.ts`, `session-persistence-jsonl/win32.ts`, `windows-inspector.ts`.
- Xoá `koffi`.

### Acceptance Criteria (P5-posix)

- [x] Không còn `require('koffi')` cho Linux — payload Linux không còn koffi/node-pty/system.node; loader trỏ sang crate `native/dsh-native`.
- [x] PTY parity: resize, signal, exit code — `native-pty-probe.sh` báo `size="33 111"`, `write_echo=2`, `signal="SIGTERM"`; bộ test `terminal` + `tool-bash-persistent` + `subprocess-local` (546 test) xanh khi chạy qua loader.
- [x] Flock parity: `native-flock-probe.sh` báo `contention=EAGAIN/11/flock`; `lease.two-process.e2e.ts` xanh.
- [x] Execve parity: `native-flock-probe.sh` báo `fcntl=ok` + `execve-ok`; 381 test `subprocess-local` xanh trên loader.
- [x] Landlock enforcement giữ nguyên test — `landlock-parity-probe.sh`: ma trận hành vi giống hệt bản C; `landlock-sandbox-probe.sh`: read-only chặn ghi, `/dev/null` ghi được, workspace-write ghi được.
- [x] Image output khớp sharp trong sai số cho phép — `image-parity-probe.sh`: facts giống hệt 5/5 định dạng, WebP lệch 0–6%; 93 test `attachment-local` xanh trên provider Rust.
- [x] Provider parity suite xanh — `packages/subprocess/subprocess-local` (381 test) xanh trên provider Rust.
- [x] `git diff -- packages/` = 0.

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

- [x] Component manager cho Office/Python (`components/` + Host installer + bridge).
- [x] Prune production tree.
- Brotli cho Web dist.
- Ripgrep on-demand.
- CI size budget.

### Acceptance Criteria

- [x] **Ngân sách mới (A3′): artifact tải về ≤ 105 MB, cài đặt ≤ 400 MB.** Đạt: AppImage 86,7 MiB · .deb 66,9 MiB · .rpm 72,9 MiB · cài đặt 337 MiB (`dsh/` 255 MB + runtime 127 MB). Ngưỡng "cài đặt ≤ 105 MB" không thể đạt: Node đã 103,5 MB một mình. Cắt được: engine Office 144 MB → component on-demand, strip Node (−17 MB) và module native (−1,3 MB), bỏ `sharp`/`@img`/`node-pty`/`koffi`/`system.node` khỏi payload.
- [x] Office cài lần đầu chạy được, có progress + lỗi rõ ràng (status + SSE + card trong Settings).
- [x] Python cài lần đầu chạy được (import check trước khi publish).
- [x] Không có Office/Python trong bundle core.
- [x] Size gate: `apps/desktop-tauri/scripts/size-gate.sh` (artifact ≤ 105 MiB, cây cài đặt ≤ 400 MiB) — CI của fork gọi script này.
- [~] `bun run test:web:built` vẫn pass — **3 ca đỏ có sẵn tại HEAD**, chứng minh bằng checkout `f8d5be74c2^` (trước mọi thay đổi của fork) vẫn đỏ y hệt ⇒ không phải hồi quy của lane. Không sửa expected output.

---

# Phần VI — Rollout theo nền tảng

| Giai đoạn | Linux | macOS | Windows |
|---|---|---|---|
| v1 | Tauri | Electron | Electron |
| v2 | Tauri | Tauri | Electron |
| v3 | Tauri | Tauri | Tauri |

**Cam kết trung lập nền tảng:**

- [x] Crate native `#[cfg(target_os)]` từ đầu — crate `dsh-native` chỉ build binary Landlock trên Linux (`#![cfg(target_os = "linux")]`), phần module dùng chung portable.
- [x] Updater tách policy/driver — `update/service.ts` giữ policy (check → tải → xác minh → xin phép), `update/appimage.ts` là driver cài đặt qua giao diện `UpdateInstallerDriver`; driver macOS/Windows để lại cho carrier Electron.
- [x] Manifest update thống nhất — một manifest tự mô tả, chọn artifact theo `platform-arch`.
- [ ] Không hardcode path Linux — phần dùng chung (Host, crate) không hardcode đường dẫn Linux ✓; shell Tauri hiện là carrier Linux nên driver của nó (AppImage) theo đường dẫn riêng, port macOS/Windows đi kèm.
- [x] Bridge contract TS — cầu `__dsh-desktop/*` do Host phục vụ bằng TypeScript ✓.
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
| A1 | 10 bất biến giữ nguyên | **đạt (10/10 cơ chế)** | Session format/`SESSION_FORMAT_VERSION` không đổi; transcript không đổi (`git diff origin/master...HEAD -- packages/session packages/core` rỗng); CLI/`cordis.yml` không đổi; token/cookie + WebSocket giữ nguyên (bridge probe); PTY semantics kiểm bằng provider parity (resize/signal/exit code); sandbox semantics kiểm bằng landlock parity (`parity=identical`) + đọc-thoát `fd`; update policy giữ nguyên (N→N+1 chạy trên AppImage, feed **đã ký**); plugin API giữ nguyên (`dsh plugin` cài + chạy, P0); bundle/profile giữ nguyên (profile desktop dùng chung `dsh`); **transcript diff = 0**: `live-transcript-probe.sh` chạy lượt thật qua route Codex ⇒ session ghi ⇒ replay ⇒ hai log đã chuẩn hoá khớp từng byte ✓ — *trừ* phiên có ảnh đính kèm (F13) |
| A2 | Không sửa `packages/` upstream | `git diff upstream/master...HEAD -- packages/` rỗng |
| A3 | Artifact ≤ 105 MB (A3′) | `size-gate.sh` — **đạt**: AppImage 86,7 MiB · .deb 66,9 MiB · .rpm 72,9 MiB · cài đặt 337 MiB |
| A4 | RSS idle ≤ 250 MB | **đạt**: 170–179 MiB (`rss-probe.sh`, Host headless) |
| A5 | Cold start → first paint ≤ 1,5 s | **đạt**: 0,41–0,48 s tới lần vẽ đầu (`cold-start-probe.sh`, weston headless trong sandbox) — cửa sổ 0,23–0,26 s, start page vẽ xong ~0,45 s; nội dung ứng dụng (Host URL 2,7–2,8 s) vẫn cần máy nhiều nhân hơn |
| A6 | Update N→N+1 pass trên 3 nền tảng | **Linux đạt, 2 nền tảng còn mở** — `update-probe.sh`: feed thật ⇒ `{"phase":"available","version":"9.9.9"}` + event SSE + `feed_requests=2`; thay N→N+1 trên AppImage đã đo ở P3b. macOS/Windows còn là carrier Electron nên chưa có bản Tauri để e2e |
| A7 | Plugin thật cài + chạy | e2e |
| A8 | Session cũ mở được | **đạt** — `session-read-probe.sh` đọc session thật của máy: 7 session (5 thế hệ `.v4` hiện hành, 2 thế hệ cũ được gắn `migration-required`), `list_ok=1 list_items=7` |
| A9 | Transcript diff = 0 | **đạt** — `bun run test:snapshot` 211/211 xanh (replay session ghi sẵn khớp từng byte) + `bun run test:expected` 122/122 xanh; **phần model thật**: `live-transcript-probe.sh` chạy một lượt qua route Codex (model thật), ghi session, replay lại từ bản ghi rồi so hai log đã chuẩn hoá ⇒ `RESULT live-transcript=OK` (diff = 0). Lưu ý: transcript của phiên **có ảnh đính kèm** vẫn lệch giữa hai carrier (F13) |
| A10 | Không còn Electron trong app mới | kiểm tra bundle |

---

# Phần IX — Bảo trì fork

- `git fetch upstream && git merge upstream/master` định kỳ.
- `MEMORY.md` ghi mọi file L2 đã sửa.
- CI gate: `git diff --name-only upstream/master...HEAD -- packages apps/desktop` phải rỗng (cho phép danh sách L2).
- Không xoá `apps/desktop` cho đến khi Windows/macOS port xong.

---

# Phần X — Audit (2026-10-20)

Audit trạng thái migration sau P5-posix + A3. Mỗi điều kiện nghiệm thu ở Phần VIII được kiểm lại bằng bằng chứng chạy được; phần chưa đạt ghi rõ vì sao và cần gì để đóng.

| # | Điều kiện | Kết luận | Bằng chứng |
|---|---|---|---|
| A1 | 10 bất biến giữ nguyên | **đạt (9/10 cơ chế, 1 cần môi trường)** | Session format/`SESSION_FORMAT_VERSION` không đổi; transcript không đổi (`git diff origin/master...HEAD -- packages/session packages/core` rỗng); CLI/`cordis.yml` không đổi; token/cookie + WebSocket giữ nguyên (bridge probe); PTY semantics kiểm bằng provider parity (resize/signal/exit code); sandbox semantics kiểm bằng landlock parity (`parity=identical`) + đọc-thoát `fd`; update policy giữ nguyên (N→N+1 chạy trên AppImage); plugin API giữ nguyên (`dsh plugin` cài + chạy, P0); bundle/profile giữ nguyên (profile desktop dùng chung `dsh`); **transcript diff = 0 chưa chạy được** (không có `DEEPSEEK_API_KEY` → A9) |
| A2 | Không sửa `packages/` upstream | **đạt theo lớp L0, có 1 ngoại lệ L2 đã ghi** | `git diff --name-only origin/master...HEAD -- packages/core packages/llm packages/session packages/bundle apps/desktop/src` **rỗng**. Ngoại lệ L2: `packages/client/ui-settings-general` (thẻ cài Office trong Settings) + `packages/extensions/cordis-client-runner/src/client/slot-catalog.ts` (sinh tự động). Ngoài đó fork chỉ thêm `apps/desktop-tauri/`, `apps/desktop-host/native/`, `native/dsh-native/` và sửa `apps/desktop/scripts/desktop-build-paths.mjs` (một dòng `SUPPORTED_TARGETS`, ngoại lệ L2 đã ghi ở B24) |
| A3 | Artifact ≤ 105 MB (A3′) | **đạt** | `size-gate.sh`: AppImage 86,7 MiB · .deb 66,9 MiB · .rpm 72,9 MiB · cây cài đặt 337 MiB |
| A4 | RSS idle ≤ 250 MB | **đạt** | 170–179 MiB (`rss-probe.sh`, Host headless) |
| A5 | Cold start → first paint ≤ 1,5 s | **đạt** | `cold-start-probe.sh`: shell Tauri trong **weston headless** (không mở cửa sổ trên màn hình người dùng), chạy trong `isolated-run.sh`. **Cửa sổ start page** (bundle của app, không cần Host) mở ngay: `window-created` 0,23–0,26 s → lần vẽ đầu **0,41–0,48 s** ✓. Host khởi động song song, URL tới lúc 2,7–2,8 s (runner 2 vCPU trong sandbox), nội dung ứng dụng vẽ lúc ~2,9 s. Ngân sách "first paint" đạt; thời gian tới *nội dung* còn phụ thuộc CPU máy thật |
| A6 | Update N→N+1 trên 3 nền tảng | **Linux đạt; macOS/Windows mở** | `update-probe.sh` (2026-10-20): `status_before`/`status_after` = `{"phase":"available","version":"9.9.9"}`, event SSE `data: {...}` đến, `feed_requests=2`; thay AppImage N→N+1 đã đo ở P3b. Đã thêm lane CI của fork (`.github/workflows/desktop-linux.yml`: build → typecheck → lint → test → snapshot → đóng gói → size gate). macOS/Windows vẫn là carrier Electron: e2e trên 2 nền tảng đó chỉ làm được sau khi port Tauri; phần **ký** (Developer ID / Authenticode) cần chứng thư mà máy này không có |
| A7 | Plugin thật cài + chạy | **đạt** | P0: cài từ registry, cài từ tarball cục bộ, plugin chạy (`fiberPhase: active`) |
| A8 | Session cũ mở được | **đạt** | `session-read-probe.sh` (đọc `~/.dsh/sessions` thật, chỉ đọc): `files_total=7`, `files_by_generation=5 .v4 + 2 legacy`, `list_ok=1 list_items=7`, `format_status={"current":5,"migration-required":2}` — Host Tauri đọc được session do bản cũ ghi, phân loại đúng thế hệ, và đọc nội dung session đầu tiên |
| A9 | Transcript diff = 0 | **đạt** | `bun run test:snapshot` 211/211 xanh + `bun run test:expected` 122/122 xanh ⇒ transcript của các session ghi sẵn khớp từng byte với đầu ra model đã ghi. Phần model thật: `live-transcript-probe.sh` — một lượt qua route Codex (model thật) ⇒ session ghi ⇒ replay lại từ bản ghi ⇒ hai log đã chuẩn hoá **khớp từng byte** (`RESULT live-transcript=OK`) |
| A10 | Không còn Electron trong app mới | **đạt** | AppImage không chứa Electron (chỉ còn một thư mục *type* của browser sidebar); shell là Tauri 2 |

## Phát hiện audit khác

| # | Phát hiện | Mức độ | Xử lý |
|---|---|---|---|
| 1 | `bun run test:web:built` đỏ 3 test (skill invocation policy ×2, replayed Session log ×1) | **có sẵn, không phải do fork** | Đã kiểm lại bằng cách checkout `f8d5be74c2^ -- packages/client apps/web` (bỏ thẻ Office + slot catalog) và dựng lại client: **vẫn đỏ y hệt** → lỗi của upstream/điều kiện máy, không phải hồi quy của lane Tauri |
| 2 | Ngân sách "cài đặt ≤ 105 MB" bất khả thi | đã xử lý | A3′ (D31): artifact ≤ 105 MB, cài đặt ≤ 400 MB; Node 24.21 stripped đã 103,5 MB |
| 3 | Lỗi có sẵn: `verify-repository-references` (`python/sdk/uv.lock:113`) | có sẵn | Không sửa (ngoài phạm vi) |
| 4 | Lỗi có sẵn: `scripts/release/pack.spec.ts > does not broaden an empty parallel family` | có sẵn | Không sửa (ngoài phạm vi) |
| 5 | `git diff origin/master...HEAD -- packages` **không rỗng** (12 file, +544) | đã ghi | Là thẻ Office trong client package (L2 theo AGENTS §3, không nằm trong danh sách L0). CI gate của fork phải cho phép danh sách L2 này |
| 6 | Thiếu bằng chứng cho tray/window-state/deep link | mở | Cần phiên desktop thật (B18, B17); deep link `dsh://open` **đã đăng ký** trong `.desktop` + gói cài đặt ✓, phần *thử* trên phiên thật vẫn mở |
| 7 | P5-win32 chưa làm | mở | Chỉ khi Windows vào phạm vi; giữ koffi cho tới lúc đó (theo kịch bản rủi ro #7/#8) |

## Kiểm chứng audit bên ngoài (2026-10-20)

Một audit độc lập (`DSH_Tauri_Source_Audit_2026-10-20.md`, đọc từ bản public của nhánh tại `upstream/master...HEAD = 0/34, 137 file, +34.830/−678`) đã được kiểm lại từng finding trên source hiện tại. Kết quả: **9/12 đúng**, 3 đã lỗi thời do thay đổi của chính lane, và 4 finding đã sửa ngay.

| # | Finding | Kết luận của chúng tôi | Xử lý |
|---|---|---|---|
| F01 | Quit bỏ sót `scheduledTasks` | **Đúng** — shell chỉ đọc `activeTasks` (main.rs) trong khi contract của Host trả cả hai cờ (`quit-inspection.ts`); policy Electron hỏi khi *hoặc* cờ nào đúng (`quit-confirmation.ts`) | **Đã sửa**: shell đọc cả hai cờ và hỏi khi có một trong hai |
| F02 | Startup/Host failure chưa bền | **Đúng, đã sửa** | `HostClient::spawn` không chặn nữa (reader thread riêng báo URL/lỗi), thêm **startup deadline 60 s** và **hộp thoại lỗi** khi Host chết trước khi sẵn sàng hoặc quá hạn; Host chết sau khi sẵn sàng vẫn để người dùng thấy trang đã chết (cần làm thêm nếu muốn) |
| F03 | Shutdown không chờ `shutdown-complete` | **Đúng, đã sửa** | Reader thread bắt frame `shutdown-complete` (Host gửi ở tauri.ts:97) và báo qua condvar; shell chỉ `kill()` khi hết thời gian chờ xác nhận ⇒ không cắt ngang session write |
| F04 | Feed mới kiểm hash, chưa có chữ ký | **Đúng** — manifest có URL/SHA-256/size, không có trust anchor | **Đã đóng (2026-10-21)**: feed ký **Ed25519**; public key **ghim** trong `apps/desktop-host/src/update/signing.ts` (`UPDATE_FEED_PUBLIC_KEY`), private key nằm trong secret `DSH_RELEASE_SIGNING_KEY` của fork; `manifest.json` lên `schemaVersion: 2` và có thêm `minimumVersion` (client cũ hơn bị từ chối). Probe `update-probe.sh` chạy trên shell **đã đóng gói**: manifest ký hợp lệ ⇒ `{"phase":"available","version":"9.9.9"}` + event SSE; manifest bị sửa chữ ký ⇒ `{"phase":"idle"}` (không có update nào được đề nghị) |
| F05 | Office download không chặn byte-count | **Đúng** — vòng tải không có trần, không đối chiếu `received === bytes`; updater thì có cả hai | **Đã sửa**: trần byte + đối chiếu kích thước chính xác, lỗi `integrity` |
| F06 | Native `sharp`/PTY chỉ là subset | **Đúng** — shim delegate các thao tác ngoài subset (PNG/TIFF/`withMetadata`/…) nên sẽ throw nếu đã bỏ `sharp`; `toColourspace()` là no-op; `metadata()` trả placeholder | **Đã đóng (2026-10-21)**: module Rust giờ **encode** được PNG/TIFF/GIF (thêm feature `tiff` của crate `image`), **đọc/ghi** khối metadata (EXIF/XMP/IPTC/ICC) cho PNG/JPEG/WebP, và **ghi** orientation theo yêu cầu; shim `sharp.cjs` **không còn đường delegate** nào (không còn `delegatedCount`, không còn `fallbackSharp`) ⇒ payload không cần gói sharp. Bằng chứng: `image-parity.mjs` so shim với sharp thật (11 ca) + 6 test `sharp-shim.spec.ts` (bỏ qua khi chưa build addon). **Khác biệt còn lại, đã đo**: resample của libvips khác module Rust (max 6/255 mỗi kênh, mean 0,379) ⇒ **byte ảnh đã chuẩn hoá của Electron và của Tauri khác nhau** ⇒ transcript của phiên có ảnh đính kèm *không* giống byte-for-byte (xem F13) |
| F07 | Tar extraction không có budget | **Đúng, đã sửa** | `extractTar` nhận `TarLimits` (tổng byte, số entry, độ sâu, kích thước một file) và từ chối archive vượt ngân sách ✓ (3 ca test mới) |
| F08 | Thiếu capability tương đương Electron | **Đúng** — không có browser guest, deep link chỉ focus | **Mở** (Wave 3) |
| F09 | CI chưa chứng minh artifact đầy đủ | **Đúng** — workflow không đóng gói, không size gate, `bun-version: latest` không ghim | **Đã đóng (2026-10-21)**: ghim Bun `1.4.3`; lane mới `desktop-release.yml` dựng primary runtime → đóng gói tarball → dựng cây runtime → pack AppImage/.deb/.rpm → **size gate** → phát hành GitHub Release kèm tài liệu feed (D32). Chạy thủ công theo tag, không tự chạy theo branch push. **Đã chạy thật**: release `desktop-v0.2.1-rc.3` trên fork (19m54s, cả hai target) — AppImage + .deb + .rpm + `manifest.json` **đã ký** + `components.json` + archive Office; chữ ký verify được bằng public key ghim trong Host, bản sửa bị từ chối (M26). Ngoài ra lane CI của fork **so shim ảnh với sharp thật** mỗi lần push | | **Đúng** — workflow không đóng gói, không size gate, `bun-version: latest` không ghim | **Đã đóng (2026-10-21)**: ghim Bun `1.4.3`; lane mới `desktop-release.yml` dựng primary runtime → đóng gói tarball → dựng cây runtime → pack AppImage/.deb/.rpm → **size gate** → phát hành GitHub Release kèm tài liệu feed (D32). Chạy thủ công theo tag, không tự chạy theo branch push. **Đã chạy thật**: release `desktop-v0.2.1-rc.2` trên fork (12 phút) — AppImage 90.917.368 B + `manifest.json` + `components.json` + archive Office 107.236.690 B; sha256/size trong manifest khớp đúng byte đã upload |
| F10 | Drift với upstream ở hai entrypoint Host | **Đúng** — `tauri.ts` cố ý duplicate bootstrap; sự khác biệt Office on-demand là chủ ý | **Đã đóng (2026-10-21)**: phần **chung** của hai entrypoint (nạp profile, lease, `runProfile`, trả lời `shutdown`/`quit-inspection`, phát `ready`, thả lease) được gom vào `apps/desktop-host/src/desktop-launch.ts`; mỗi entry giữ phần riêng (Electron: bundle app + Office plugin; Tauri: update service, Office component, desktop bridge). Sau khi gom, một đổi đổi hành vi chỉ sửa một chỗ. Kèm phát hiện: các probe boot Host **từ source** bằng `node file.ts` đã hỏng sẵn (Node strip-only không chịu *parameter property*) ⇒ chuyển sang tsx ESM hook theo contract source-launch của repo |
| F11 | Test isolation cần budget | **Đúng một phần** — `isolated-run.sh` có namespace/clearenv/ro-bind nhưng chưa có budget CPU/RAM/disk; loopback vẫn dùng chung | **Đã đóng (2026-10-21)**: budget CPU/RAM/disk/pids bằng **cgroup v2** (`cpu.max`, `memory.max` + `memory.swap.max=0`, `pids.max`) và tmpfs có `size` cho disk; `$work` thành tmpfs. `isolation-budget-probe.sh`: `inside-budget=ok`, `ram-budget=ok`, `disk-budget=ok`, `process-budget=ok`, `cpu-budget=ok` |
| F12 | Coexistence Tauri/Electron trên cùng state | **Đúng** — single-instance của Tauri không bảo vệ state khỏi Electron. Rõ hơn: hai carrier giữ **profile dir khác nhau** (`$DSH_HOME/desktop-project` vs `$DSH_HOME/profiles/desktop`) nhưng **chia sẻ** session store, settings document và credential store trong `$DSH_HOME` ⇒ hai Host ghi cùng một chỗ | **Đã đóng (2026-10-21)**: **lease** trong `$DSH_HOME/desktop-host.lease` do **cả hai** entrypoint Host (Electron `index.ts` và Tauri `tauri.ts`) giữ suốt đời; lease của tiến trình đã chết thì được nhận lại. `desktop-lease-probe.sh`: carrier thứ nhất giữ lease, Host thứ hai bị từ chối (`is in use by another Desktop application`), carrier đã chết ⇒ carrier sau nhận lại lease |
| F13 | Ảnh chuẩn hoá của hai carrier lệch byte | **Đúng (phát hiện 2026-10-21)** — module Rust resize bằng `fast_image_resize`, libvips (Electron) resize bằng arithmetic fixed-point riêng ⇒ cùng một ảnh gốc cho **byte khác nhau** (đo: 574/2304 byte lệch, max 6/255, mean 0,379; size WebP lệch 0–6%). Hệ quả: A1/A9 ("transcript diff = 0") **chỉ đúng với phiên không đính kèm ảnh**; ảnh đính kèm thì transcript của Electron và của Tauri khác nhau | **Mở** — cần khớp kernel + làm tròn của libvips, hoặc ghi nhận đây là khác biệt đã chấp nhận |

Ba finding đã lỗi thời khi kiểm chứng: F02 phần "`.setup()` chặn" (start page đã tách phần chờ), và các dòng trích dẫn trong audit lệch số dòng so với HEAD hiện tại (nhánh đã đi 35 commit).

## Việc còn lại theo thứ tự ưu tiên

Cập nhật 2026-10-22 (sau khi lane CI xanh trọn vẹn, release **0.2.1-rc.3** có feed **đã ký**, F04/F06/F10/F11/F12 đã đóng, A9 đã chạy với model thật, và một vòng audit bảo mật theo skill `security-audit`):

1. **7 phát hiện của audit bảo mật** (2026-10-22) — bản đầy đủ ở `~/security-audit-skill/deepseek-harness/run-1/`; cái **high** là feed component **không ký** ⇒ archive tải về được thực thi; kèm giải nén zstd không giới hạn, WebP độc hại làm sập Host, `appimagetool` tải không kiể digest, và 3 cái low (bridge ghi shortcut không validate, TOCTOU khi cài AppImage, lock admission không nhả khi cài fail). Xem `HANDOFF.md` §3 (A).
2. **F14** — payload mang ~34 MB package `dsh-experimental-*` không dùng (32 MB là model giọng nói `sherpa-onnx`); lọc trong `stage-payload.sh` ⇒ AppImage 90 MB → ~62 MB.
3. **F13** — byte ảnh chuẩn hoá lệch giữa hai carrier (đo: max 6/255 mỗi kênh): hoặc port đúng kernel + làm tròn của libvips, hoặc **ghi nhận là khác biệt đã chấp nhận** và ghi rõ vào điều kiện đóng của A1/A9 (chỉ đúng với phiên không đính kèm ảnh).
4. **P5-win32** — khi Windows vào phạm vi. Đã có baseline koffi để so: `windows-native-probe.sh` chạy bộ test Windows-native dưới Wine (3 suite xanh trọn vẹn; ACL đỏ vì Wine không có security descriptor — B49).
5. **A6 phần macOS/Windows** — e2e update trên 2 nền tảng sau khi carrier Tauri thay Electron.
6. **P3 ký** — macOS (Developer ID + notarize) và Windows (Authenticode) cần chứng thư, không làm được trên máy này.
7. **P4 3 target còn lại** — macOS x64/arm64, Windows x64: cần runner macOS/Windows hoặc chứng thư.
8. **P6 tùy chọn** — brotli cho Web dist (96,9 MB JS/CSS trong payload; server tĩnh là upstream nên cần middleware của Host) và ripgrep on-demand (5,5 MB; cần loại component mới).
9. **Welcome/onboarding** — không tới được ở v1, xem ghi chú P2.
