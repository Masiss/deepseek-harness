# MEMORY — Desktop Tauri

Bộ nhớ dài hạn của nhánh desktop. Cập nhật file này khi có quyết định, số đo hoặc bẫy mới. Kế hoạch và ADR nằm ở [`PLAN.md`](PLAN.md); quy tắc làm việc ở [`AGENTS.md`](AGENTS.md).

## Trạng thái

| Mục | Giá trị |
|---|---|
| Giai đoạn | **P1 đang làm — P0 xong (bác D2), P1 phần shell essentials đã có** |
| Nhánh | `chore/web-bun-migration` (đã push lên `fork`) |
| Remote `origin` | `deepseek-ai/deepseek-harness` (upstream) |
| Remote `fork` | `Masiss/deepseek-harness` |
| Commit nền | commit đầu nhánh `chore/web-bun-migration` (xem `git log`) |
| Ngày chốt kế hoạch | 2026-10-19 |
| Ngày bắt đầu P0 | 2026-10-19 |

## P0 — kết quả đã có (2026-10-19)

Đã dựng:

| Đường dẫn | Vai trò |
|---|---|
| `apps/desktop-tauri/src-tauri/` | Shell Tauri 2.12.4: spawn sidecar, đọc frame `ready`, tạo window |
| `apps/desktop-host/src/tauri.ts` → `lib/tauri.js` | Entry Host L1: profile `desktop`, **không** mount Office (ADR-0008) |
| `apps/desktop-tauri/scripts/dev-host.sh` | Launcher dev: chạy `apps/desktop-host/src/tauri.ts` bằng `node` của máy |
| `apps/desktop-host/src/transport.ts` | Transport IPC ↔ NDJSON, tự chọn theo `process.send` |
| `apps/desktop-host/tests/transport.spec.ts` | 7 test codec + reassembly + disconnect + close |

Đã chứng minh bằng chạy thật:

- Shell spawn Host Node → frame `ready` (13 injection) → tạo window → `WebKitNetworkProcess` mở **4 kết nối ESTAB** tới cổng Host.
- Frame `shutdown` → `shutdown-complete` → Host thoát **code 0**.
- **Chạy headless hoàn toàn** (không cần GUI): `/` ẩn danh → **401**; `/` kèm token → **303** rồi **200** + `__DSH_BOOT__`; `/api/remote.mux` → **101 Switching Protocols**.
- Chromium headless render UI thật: **FCP 280 ms**, DOMContentLoaded 118 ms, **0 console error**, 71 client entry.
- Shell release: **8,8 MB**; cold start Host **~1,0–1,2 s**; RSS idle **252–268 MiB**.
- Bản **đã build** `apps/desktop-host/lib/tauri.js` cũng boot tới `ready` và thoát code 0 (1,0 s, 252 MiB).

Còn lại của P0: PTY, `dsh plugin`, mở session cũ, transcript diff = 0, M1/M7.

**Chặn:** AC "transcript diff = 0" không chạy được ở máy này — không có `DEEPSEEK_API_KEY` (không có `.env`, env trống). Cần key hoặc chạy trên CI có key.

## P1 — kết quả đã có (2026-10-19)

Đã implement trong `apps/desktop-tauri/src-tauri/`:

| Việc | Trạng thái | Bằng chứng |
|---|---|---|
| `host.rs` — supervisor NDJSON + request/response theo `requestId` | xong | `cargo test` 1/1; probe trả `activeTasks:false` |
| Single instance → focus cửa sổ cũ | **xong** | lần chạy 2 tự thoát sau **254 ms**, tiến trình 1 sống, Host vẫn phục vụ |
| Window state (size/position/maximized) | xong | plugin `tauri-plugin-window-state`; file ghi lúc thoát sạch (chưa test được) |
| Đóng cửa sổ = ẩn | **xong** | gửi `WM_DELETE_WINDOW`: `map_state=0`, tiến trình sống, 14 kết nối Host còn nguyên |
| Tray + menu Show/Quit | **xong** | đăng ký với StatusNotifierWatcher: `Id=tray-icon tray app main`, `Status=Active`, icon `/run/user/1000/tray-icon/tray-icon-main-0.png` |
| Thoát hỏi khi có task (`quit-inspection`) | xong (mã) | response `{"type":"quit-inspection","requestId":1,"activeTasks":false}` |
| Degrade khi không có tray | xong | `build_tray` chỉ log, không làm chết app |

Số đo mới: shell release **10,3 MB** (tăng 1,5 MB cho tray + dialog + window-state + single-instance).

**Chưa test được ở máy này (cần phiên desktop thật):** bấm menu tray; menu DBusMenu của ayatana không expose `com.canonical.dbusmenu` cho client ngoài nên không click được bằng `gdbus`; đóng app sạch để kiểm tra file window-state; deep link `dsh://open` (cần đăng ký `.desktop`).

### Đầu dò dùng lại được

| Đường dẫn | Việc |
|---|---|
| `.agents/plan/desktop-tauri/probes/measure-host.mjs` | Đo cold start → `ready`, RSS, shutdown |
| `.agents/plan/desktop-tauri/probes/headless-probe.sh` | Boot headless, kiểm tra 401/303/200, `__DSH_BOOT__`, mux 101 |
| `.agents/plan/desktop-tauri/probes/render.mjs` | Playwright headless: render UI, FCP, console error |
| `.agents/plan/desktop-tauri/probes/shell-probe.sh` | Single instance + cửa sổ + host sống |
| `.agents/plan/desktop-tauri/probes/quit-flow-probe.sh` | Thử bấm menu tray qua DBusMenu (chưa click được) |

`render.mjs` cần chạy trong `apps/web` (nơi có `playwright`):

```sh
cd apps/web && node ../../.agents/plan/desktop-tauri/probes/render.mjs "<host url>"
```

Chạy lại thí nghiệm:

```sh
DSH_DESKTOP_HOST_BIN=$PWD/apps/desktop-tauri/scripts/dev-host.sh \
DSH_DESKTOP_RUNTIME_DIR=$HOME/.dsh/profiles \
DSH_DESKTOP_PROJECT_DIR=$HOME/.dsh/profiles/web \
./apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri
```

Đo tự động: `node /tmp/opencode/measure-host.mjs node apps/desktop-host/lib/tauri.js <runtimeDir> <projectDir>` (script tạm, chưa đưa vào repo).

## Quyết định đã chốt

| # | Quyết định |
|---|---|
| D1 | Desktop shell mới là **Tauri 2 (Rust)**; Electron không dùng cho app mới |
| D2 | ~~Runtime JS của app là **Bun**, đóng gói dạng compiled sidecar~~ → **BÁC BỎ ở P0**. Runtime JS là **Node 24**. Xem bẫy `B8`, `B9` |
| D2b | Sidecar là bản Node chạy `apps/desktop-host`; Bun chỉ còn dùng cho tooling build/test của repo |
| D2c | Transport shell↔host là **NDJSON có tiền tố `@@DSH@@`** trên stdout/stdin, tự chọn khi không có kênh IPC |
| D3 | Harness runtime (agent, tools, session, plugin) **giữ TypeScript**, không rewrite Rust |
| D4 | Native layer gom vào **một crate napi-rs** `@deepseek-ai/dsh-native` |
| D5 | Thay provider qua **profile patch**, không sửa package upstream |
| D6 | **Linux first**, macOS/Windows sau Linux production; Electron cũ làm cầu tạm |
| D7 | Nạp thẳng Host URL `http://127.0.0.1:<port>/?token=…`, bỏ custom protocol + HTTP proxy |
| D8 | Office + Python tách thành component tải theo nhu cầu |
| D9 | Giữ policy engine updater, chỉ thay driver cài đặt |
| D10 | "Loại bỏ package Windows" = port lớp koffi sang Rust, **không xoá tính năng** |
| D11 | Shell P1: single-instance, window-state, **đóng cửa sổ = ẩn** (process + Host sống tiếp), tray, thoát qua `quit-inspection` |
| D12 | Thoát: Host không trả lời ⇒ coi như **có** task đang chạy và hỏi, không thoát im lặng |

## Số đo thật

Đo trên máy phát triển Linux x64, 2026-10-19.

| Thành phần | Kích thước |
|---|---|
| Electron 44.7 zip linux-x64 | 117 MB |
| Electron 44.7 zip darwin-arm64 | 124 MB |
| Node 24.21 `bin/node` | 121 MB |
| Node 24.21 sau `strip` | 104 MB |
| Bun 1.4.3 binary | 67 MB |
| `bun build --compile` hello-world | 68,6 MB |
| `bun build --compile --minify` | 68,6 MB |
| `bun build --compile --minify --bytecode` | 68,6 MB |
| Web dist (`apps/web/dist`) | 22 MB |
| `@vscode/ripgrep` linux-x64 | 5,5 MB |
| `sharp` + `libvips` linux-x64 | 19 MB |
| `koffi` | 2,3 MB |
| `node-pty` prebuild 1 platform | ~0,5 MB |
| LibreOffice Kit darwin-arm64 | 158 MB |
| Python 3.12 + wheels | ~100–130 MB |

Kết luận: **Bun runtime 67 MB là sàn**. Cờ compile của Bun không giảm thêm (0 MB chênh lệch).

### Số đo P0 (2026-10-19, Linux x64, `apps/desktop-tauri`)

| Phép đo | Kết quả | Ghi chú |
|---|---|---|
| `cargo build` (debug, từ số 0) | 1 m 52 s | Tauri 2.12.4, webkit2gtk 2.54.1 |
| `cargo build --release` | 1 m 58 s | `opt-level = "s"`, `lto = "thin"`, `strip` |
| **Tauri release binary `dsh-desktop-tauri`** | **8,8 MB** | 8.803.688 byte; `strip` không giảm thêm |
| Render webkit2gtk dev libs | **có sẵn** | Bác bỏ `B4` |
| Host cold start → frame `ready` | **~0,9–1,2 s** | `main.ts` + profile `web`, đo 2 lần |
| Host RSS idle (Node 26) | **266–268 MiB** | Sau 5 s kể từ `ready` |
| Kích thước frame `ready` | 37,6 KB | 13 injection |
| Shutdown qua frame `shutdown` | exit code 0 | `shutdown-complete` rồi thoát |
| Webview → host | **4 kết nối ESTAB** | `WebKitNetworkProcess` → `127.0.0.1:<port>` |
| `bun build --compile` host (L1, không Office) | 70 MB | Chạy được tới profile load rồi chết vì addon V8 |
| Host built `lib/tauri.js` → `ready` | 1,0 s / 252 MiB | Xác nhận đường đóng gói thật |
| Shell P1 release (tray + dialog + window-state) | **10,3 MB** | 10.300.920 byte |
| Lần chạy thứ hai (single instance) | thoát sau **254 ms** | Tiến trình 1 vẫn sống |
| `WM_DELETE_WINDOW` → ẩn cửa sổ | `map_state=0`, process sống | Host còn 14 kết nối |

Các số đo trên là với **Node 26.8.2 của máy dev**, chưa phải Node 24.21 đóng gói. Đo lại ở P1.

## Bẫy đã gặp (đừng mất thời gian lại)

### B1 — "Bun compiled không resolve symlink" là kết luận sai

Ban đầu tưởng lỗi symlink vì `node-pty` fail. Test lại: symlink hoạt động bình thường.

**Nguyên nhân thật:** `createRequire().resolve()` trong compiled binary chỉ tìm `<pkg>/index.js`, **bỏ qua `main` và `exports`** với package bên ngoài.

| Package | `main` | Compiled resolve |
|---|---|---|
| `p4` | `index.js` | ✅ |
| `p6` | `lib/index.js` | ❌ |
| `p8` | `exports: ./lib/index.js` | ❌ |
| `p10` | `entry.js` | ❌ |

**Đường thoát:** resolve **absolute path** hoạt động, kể cả `require()` và native `.node`. Resolver của repo (`packages/boot/app-boot/src/profile-resolution/resolver.ts`) đọc `manifest.main` rồi resolve absolute → nhiều khả năng không bị ảnh hưởng. Phải xác nhận ở P0.

### B2 — `import.meta.resolve` trả `$bunfs`

Trong compiled binary: `import.meta.resolve('./x')` → `file:///$bunfs/root/x`. Chỉ ảnh hưởng specifier tương đối; phải audit 53 file dùng nó.

### B3 — `node:sqlite` trên Bun: **đã xác nhận CHẠY ĐƯỢC**

`bun -e "require('node:sqlite')"` (Bun 1.4.3) trả về `DatabaseSync,StatementSync,Session,constants,backup`. Không còn là ẩn số.

### B4 — ~~Máy phát triển thiếu webkit2gtk dev libs~~: **SAI**

`pkg-config` tìm thấy đủ: `webkit2gtk-4.1` 2.54.1, `gtk+-3.0` 3.24.52, `javascriptcoregtk-4.1` 2.54.1, `libsoup-3.0` 3.8.0, `ayatana-appindicator3-0.1` 0.6.1. `cargo build` cho Tauri 2.12.4 chạy hết trong 1 m 52 s. Build và chạy Tauri **được ngay trên máy này**.

### B5 — Client không phụ thuộc origin `dsh-app://`

Grep cho thấy client chỉ feature-detect `'dshDesktop' in globalThis`; chỉ một test gateway dùng `dsh-app://` làm origin giả. Nên nạp `http://127.0.0.1` an toàn.

### B6 — `test:docs` có failure có sẵn

`verify-repository-references` fail ở `python/sdk/uv.lock:113` (commit-hash), tái hiện ở HEAD sạch. Không sửa trong phạm vi desktop.

### B7 — `scripts/release/pack.spec.ts` có failure có sẵn

`does not broaden an empty parallel family to the whole workspace` fail ở HEAD sạch.

### B8 — Bun **không chạy được** harness runtime (đã xác nhận)

Boot thẳng Host dưới Bun:

```
@@DSH@@{"type":"fatal","message":"dsh: host preparation failed:
  node-addon-require-builtin unsupported: Unsupported/no-context
  (required V8 current-context symbols were not found)"}
```

Nguyên nhân gốc: `packages/boot/app-boot/src/profile-resolution/resolver.ts` hàm `internalModules()` nạp addon N-API `node-addon-require-builtin` để lấy loader ESM/CJS nội bộ của Node. Addon này đọc symbol V8 current-context; Bun dùng JavaScriptCore nên không có. `PluginPackages` gọi `installRuntimeInterception()` **vô điều kiện** khi có resolution → không có nhánh fallback.

Đây là cơ chế lõi của plugin/profile resolution, không phải tiện ích phụ. **Không thể port sang Bun mà không viết lại resolver.**

→ Kích hoạt điều kiện dừng ở [`AGENTS.md`](AGENTS.md) mục 7: bỏ Bun-compiled, fallback Node. `D2` bị bác bỏ.

### B9 — `bun build --compile` còn hai nhóm lỗi độc lập

Kể cả khi bỏ qua B8, `--compile` vẫn vướng:

1. **Static import built-in Bun không có.** `packages/skill/skill-office/src/index.ts` và `apps/desktop-host/src/office-engine.ts` import `node:sea` / `node:module#registerHooks`. Bun 1.4.3 thiếu cả hai → chết ngay lúc load module. Có thể vá bằng `Bun.build` plugin (đã thử, chạy được cho `node:sea`) nhưng phải vá `registerHooks` bằng cách đổi sang namespace import + guard.
2. **Mẫu `import.meta.url` để đọc `package.json`.** Trong bundle, `import.meta.url` là `/$bunfs/root/...` nên:
   - `createRequire(import.meta.url)('../package.json')` → `Cannot find module '../package.json'` (crash thật ở `packages/llm/llm/src/attribution.ts`);
   - `new URL('../package.json', import.meta.url)` → đường dẫn `$bunfs` không tồn tại;
   - `apps/cli/src/profile-boot.ts` `INSTALL_ANCHOR` cũng dựng theo cách này.

   Có **10 điểm** trong graph host dùng mẫu này.

Kết luận: `--compile` đòi sửa nhiều file upstream (vi phạm L0). Không theo đuổi.

### B10 — Host stdio chỉ thoát khi nhả stdin

`process.stdin` dạng pipe giữ event loop sống. Sau `shutdown-complete`, Host vẫn treo tới khi shell đóng đầu pipe (đo được: treo > 30 s). Đã sửa trong `createStdioTransport.close()`: gọi `input.unref()` sau khi báo shutdown. Đo lại: exit code 0 ngay.

Hệ quả cho shell Tauri: gửi `shutdown`, chờ `shutdown-complete`, rồi **đóng stdin của child**; giết tiến trình chỉ là phương án dự phòng sau timeout.

### B11 — `apps/desktop-tauri` cần package.json để Node resolve

Node resolve bare specifier theo vị trí file import. Bản đầu đặt entry ở `apps/desktop-tauri/host/main.ts` (không thuộc package nào) → `ERR_MODULE_NOT_FOUND` cho `@deepseek-ai/dsh-app-boot`. Đã chuyển entry vào `apps/desktop-host/src/tauri.ts`; giờ nó resolve qua `apps/desktop-host/node_modules` và build ra `lib/tauri.js`.

### B12 — `import.meta.main` mất dưới `tsx`, còn dưới `node` native

`node --import tsx/esm x.ts` → `import.meta.main` là `undefined`. `node x.ts` (type-stripping native) → `true`. Entry của Host dùng `import.meta.main`, nên launcher Tauri phải chạy **`node` trực tiếp**, không qua `tsx`.

### B13 — Entry upstream `apps/desktop-host/src/index.ts` cần Office runtime đã stage

Chạy entry gốc với `runtimeDir=$HOME/.dsh/profiles` thì boot tới lúc mount `skill-office` và chết:

```
ENOENT: /home/masis/.dsh/runtime/office-skills/office/SKILL.md
```

Đó là thư mục do bước đóng gói Electron stage sẵn. Vì vậy có thêm entry L1 `apps/desktop-host/src/tauri.ts`: cùng profile, cùng transport, **không** mount Office — đúng hướng ADR-0008 (Office thành component tải sau).

Entry này **phải nằm trong package `apps/desktop-host`** (không phải `apps/desktop-tauri`): nó dùng chung `transport.ts`, `update-tasks.ts`, `quit-inspection.ts`, `platform-session.ts` và các dependency của package. Nếu đặt ở `apps/desktop-tauri` thì bản đóng gói chỉ có `lib/index.js`/`lib/cli.js`, không có các module nguồn để import.

Đã thêm:
- `tsdown.config.ts`: entry thứ ba `tauri` → `lib/tauri.js`;
- `package.json#files`: `lib/tauri.js`.

**Nợ kỹ thuật P1:** thân hai entry gần như trùng nhau. Khi Office tách xong, gộp lại bằng cách rút phần chung thành một hàm nhận `installOffice?: (ctx) => Promise<void>`; lúc đó entry Electron truyền installer Office, entry Tauri không truyền.

### B14 — Cửa sổ Tauri chỉ hiện sau khi Host `ready`

`setup()` đọc frame `ready` đồng bộ rồi mới tạo window, nên app không có cửa sổ trong ~1–2 s đầu (và lâu hơn nếu profile chậm). P1 phải tạo cửa sổ splash trước, rồi điều hướng sang URL Host khi nhận `ready`.

### B16 — Tiến trình cũ còn sống làm mọi lần chạy sau "không có Host"

Single instance khiến lần chạy thứ hai thoát ngay. Nếu một tiến trình từ lần thí nghiệm trước còn sống (ví dụ bị `kill` không ăn), mọi lần chạy mới sẽ **im lặng thoát** và probe báo `host_port=none`. Luôn dọn trước khi đo:

```sh
ps -eo pid,args --no-headers | grep -F "target/release/dsh-desktop-tauri" | grep -v grep | awk '{print $1}' | xargs -r kill -9
```

Đừng dùng `pkill -f "release/dsh-desktop-tauri"` trong chính lệnh đang chạy — mẫu đó khớp cả dòng lệnh của shell và tự giết mình.

### B17 — Đo "đóng cửa sổ" cần `GDK_BACKEND=x11`

Phiên là Wayland, nên cửa sổ GTK không phải cửa sổ X11. Ép `GDK_BACKEND=x11` rồi dùng `XSendEvent` với `WM_DELETE_WINDOW` mới đo được `map_state` (2 = hiện, 0 = ẩn).

### B18 — Menu tray ayatana không click được bằng `gdbus`

Item đăng ký đủ (`Id`, `Status`, `IconName`, `Menu` trỏ tới `/org/ayatana/NotificationItem/tray_icon_tray_app_main/Menu`), nhưng object đó chỉ expose `Properties`/`Introspectable`/`Peer` — không có `com.canonical.dbusmenu`. Muốn test bấm menu phải làm tay trên phiên desktop thật.

### B15 — Không chụp được cửa sổ Tauri bằng `import` trên phiên Wayland

`import -window root` fail (`missing an image filename`); `grim` chụp được toàn màn hình nhưng cửa sổ Tauri không nằm trong khung hình chụp. Cách kiểm chứng đáng tin là `ss -tnp`: thấy `WebKitNetworkProcess` mở kết nối ESTAB tới cổng Host.

## Sự kiện / đo đạc cần làm

| # | Việc | Phase | Trạng thái |
|---|---|---|---|
| M1 | Đo `dsh` production tree thật (`prepare-dsh`) | P0 | chưa |
| M2 | Đo Tauri release binary (Linux) | P0 | đang chạy |
| M3 | Đo RSS idle + cold start | P0 | **xong** (Host Node: 0,9–1,2 s; 266–268 MiB) |
| M4 | Xác nhận `node:sqlite` trên Bun | P0 | **xong** — chạy được |
| M5 | Chạy `dsh plugin` + load plugin trong compiled binary | P0 | **bỏ** — Bun bị bác (`B8`) |
| M6 | So sánh DACL/token provider Rust vs koffi (Windows) | P5-win32 | chưa |
| M7 | Đo lại cold start/RSS với Node 24.21 đóng gói | P1 | chưa |
| M8 | Đo RSS + cold start của shell Tauri có webview | P0 | chưa |

## Câu hỏi mở

| # | Câu hỏi | Ảnh hưởng |
|---|---|---|
| Q1 | Có cần embedded browser (sidebar) + Platform view trên Tauri không? | Nếu có: +4–6 pw, không parity đầy đủ |
| Q2 | Windows có bắt buộc trong 12 tháng tới? | Quyết định P5-win32 (5–7 pw) |
| Q3 | Auto-update Linux dùng AppImage self-update hay repo apt? | P3 |
| Q4 | Có cần ký/notarize macOS ngay v1 không? | +3–5 pw |
| Q5 | Có chấp nhận Office/Python tải sau lần dùng đầu? | −273 MB nếu có |

## Ghi chú môi trường

- Bun 1.4.3, Node host 26.8.2; repo pin Node 24.21.0 cho runtime đóng gói.
- `node_modules` 4,5 GB (Bun isolated layout).
- Rust 1.98.1; webkit2gtk 2.54.1 + gtk3 + libsoup3 + appindicator **đầy đủ**, Tauri build được.
- `CI=1` bỏ qua Lefthook postinstall.
- Host `desktop` dùng được profile `~/.dsh/profiles/web` cho thí nghiệm: bỏ `skill-office`, đặt `runtimeDir=$HOME/.dsh/profiles`, `projectDir=$HOME/.dsh/profiles/web`.

## Lịch sử cập nhật

| Ngày | Thay đổi |
|---|---|
| 2026-10-19 | Tạo bộ tài liệu; chốt D1–D10; ghi B1–B7 |
| 2026-10-19 | P0: bác bỏ D2 (Bun) bằng chứng `B8`; sửa `B4`; thêm `B9`–`B12`; đo cold start/RSS Host Node; dựng `apps/desktop-tauri` + Tauri 2.12.4 build được |
