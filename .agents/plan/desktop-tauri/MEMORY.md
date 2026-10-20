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

Số đo mới: shell release **10,3 MB** (tăng 1,5 MB cho tray + dialog + window-state + single-instance; `devtools` +39 KB).

### Cầu desktop (đo bằng `bridge-probe.sh` / `bridge-page-probe.sh`)

| Phép đo | Kết quả |
|---|---|
| `POST /__dsh-desktop/*` không cookie | 401 (fence Host/Origin + Peer) |
| GET / method lạ / body quá lớn | 405 / 404 / 413 |
| `device-info` | `platform=linux; os=…; app_arch=x64; cpu=…; memory_gib=15.3` |
| `shortcuts-get` chưa có file | `status=ready` |
| `shortcuts-edit` | `saved`; `keybindings.json` xuất hiện, chứa `KeyK`; lần cài sau đọc lại đúng |
| `updates-status` | `{"phase":"idle"}` |
| Shim trong index phục vụ | `globalThis.dshDesktop` ×1, `dataset.platform` ×1 |
| Trong trang thật (Chromium) | `present=true protocol=1 keyboard=true platform=linux`, `updates_phase=idle`, `saved=saved`, `keybindings_after_page=present` |
| Home mới, không key, chưa đăng nhập | `onboarding_visible=false`, `composer=true`, `console_errors=0` — vào thẳng workspace |
| Client có **thật sự** dùng cầu | 33 lời gọi `shortcuts-get` + 2 `updates-status` **do trang tự phát** (runtime desktop, không phải localStorage) |
| Lỗi console / page | 0 / 0 |
| App Tauri thật (release + WebView) | render bình thường, 10 kết nối Host, menu Application/Edit |
| Lời gọi cầu đồng thời (trước sửa) | `peak_in_flight=33`, `failed=5`, `console_errors=5` |
| Lời gọi cầu đồng thời (sau sửa) | `peak_in_flight=1`, `failed=0`, `console_errors=0` |

### Vòng đời profile trong Host (D24) — đã đo

`apps/desktop-host/src/profile-lifecycle.ts`, gọi ở đầu `tauri.ts` trước `loadProfileDirectory`:

1. `initProfile(dir, PROFILE_TEMPLATES.web.bundles)` — tạo profile nếu thiếu, giữ nguyên nếu có.
2. `removeLinkProjections(dir)` — dọn link do dev để lại.
3. Đọc manifest, tìm các **application bundle** (`dsh-web-app`, `dsh-headless`, `dsh-acp-app`, `dsh-sdk-app`, `dsh-sdk-minimal`). Desktop Host chỉ chạy được đúng `dsh-web-app`; nếu có app bundle khác thì bỏ nó, nếu thiếu web thì thêm lại. **Feature bundle không bị bỏ.**

Vì sao không phải "thử boot rồi cứu": `loadProfileDirectory` vốn đã bỏ qua bundle hỏng (`skippedBundles`) nên lỗi bundle lẻ không làm chết app; thứ làm chết app là **hai app bundle** cùng tồn tại, và điều đó kiểm được **trước khi boot** nên không cần một vòng crash–restart.

| Phép đo | Kết quả |
|---|---|
| Unit (`apps/desktop-host/tests/profile-lifecycle.spec.ts`) | 6/6 — tạo mới, idempotent (giữ nguyên byte), bỏ app bundle thứ hai, khôi phục web, profile không có app, và `dsh-base` không bị coi là app |
| End-to-end trên cây production | Cài `dsh-headless@0.2.1-rc.2` (profile thành 2 app bundle) → khởi động lại: `ready=OK`, bundle sửa còn `[base, web-app]`, stderr `dsh: dropped incompatible application bundle(s) …: @deepseek-ai/dsh-headless`, không còn `unknown option '--no-open'` |
| Toàn bộ `apps/desktop-host/tests` | 51/51 (đặt `TMPDIR` ngoài tmpfs) |

Còn hở: hiện thông báo nằm ở stderr; chưa đưa lên UI (P4). Và chưa có khoá profile như `DesktopProjectManager.withLock` của Electron — dựa vào single-instance của shell.

### Làn đóng gói Linux (M1) — đã dựng và đo

Bốn script thuộc fork, chỉ chạm `apps/desktop` một bản vá đường dẫn:

| Script | Việc |
|---|---|
| `apps/desktop-tauri/scripts/pack-linux.sh` | `build:official` rồi `release:pack` cho `dsh`/`vendor`, đóng thêm tarball `@deepseek-ai/dsh-desktop-host`, và `node-addon-system` cho Landlock |
| `apps/desktop-tauri/scripts/prepare-linux-runtime.ts` | Chọn package set, cài runtime project bằng pnpm của Node đi kèm, đóng băng cây `dsh/`, tạo profile, phát launcher + `versions.json` |
| `apps/desktop-tauri/scripts/linux-tree-probe.sh` | Chạy shell Tauri thật trên cây đó rồi gọi RPC |
| `apps/desktop-tauri/scripts/measure-linux-tree.sh` | Đo cold start + RSS của Host đã đóng gói |

```
apps/desktop/.desktop-build/targets/linux-x64/
  bin/desktop-host        launcher shell spawn (chạy lib/tauri.js bằng Node đi kèm)
  runtime/primary-runtime node 24.21.0 + pnpm 11.28.5 + Python 3.12.14   401 MB
  runtime/bin/node        symlink tới Node đi kèm
  dsh/                    runtimeDir: node_modules của dsh                414 MB
  project/                projectDir: profile
  packed/{dsh,vendor,landlock}   tarball release mà `prepareDesktopPackageSet` tiêu thụ
```

| Phép đo | Kết quả |
|---|---|
| Node đi kèm | `v24.21.0`, `dependencies/node/bin/node` = 126.595.440 B (120,7 MiB) |
| pnpm đi kèm | chạy bằng chính Node đó, `11.28.5`, **23 MB** |
| Cold start Host (spawn → frame `ready`) | 1147–1482 ms qua nhiều lần đo |
| RSS idle của Host | 309–318 MiB |
| Boot shell Tauri thật trên cây này | `ready=OK`, `pluginManager/*` trả dữ liệu của profile |

### Ngân sách runtime: giữ pnpm, prune Python

| Thành phần | Kích thước | Kết luận |
|---|---|---|
| Python + wheels | **193 MB** sau prune (trước 257 MB) | Bỏ hẳn thì Host vẫn `ready=OK`, plugin vẫn active, RSS không đổi (Python chỉ nằm trên đĩa) → on-demand của ADR-0008 là khả thi. Nhưng xem mục "Làm nhẹ payload Python" bên dưới: phần lớn chỗ đó **cắt được ngay** mà không mất gì. |
| Node | 121 MB | Bắt buộc — đây là runtime JS của app (ADR-0002). |
| **pnpm** | **23 MB** | **Giữ.** Đây chính là cơ chế cài/gỡ plugin (`installBundle`/`removeBundle`). Cắt nó là mất hệ plugin — yêu cầu số một của migration và là nội dung điều kiện dừng #4. Tự viết lại resolver+installer cũng trái luật "prefer maintained dependencies". |

Cắt Python đưa primary runtime từ 401 MB xuống 144 MB; việc còn lại là cơ chế tải theo nhu cầu của ADR-0008.

### Update trên Linux: feed thật → `available` (P3a, đã đo)

**Q3 đã chốt:** AppImage self-update. AppImage là định dạng duy nhất có thể tự thay thế chính nó mà không cần root; gói `.deb` (và mọi cài đặt không có `$APPIMAGE`) do trình quản lý gói sở hữu, driver trả `unsupported` chứ không đoán.

Hợp đồng manifest (`apps/desktop-host/src/update/manifest.ts`), một tài liệu cho mọi nền tảng:

```json
{"schemaVersion":1,"version":"0.3.0","artifacts":{
  "linux-x64":{"url":"https://…/dsh.AppImage","sha256":"<64 hex>","size":123456789}}}
```

- khoá là `${platform}-${arch}` nên **không thể** chọn nhầm artifact; target lạ bị bỏ qua để bản cũ đọc được bản phát hành mới;
- URL chỉ `https:` (trừ loopback `http://127.0.0.1` để diễn tập phát hành);
- sha256 chữ thường 64 hex, size nguyên dương; sai → `UpdateManifestError` với `failure` cố định;
- `latest` từ chối prerelease, `nightly` nhận — cùng chính sách với coordinator Electron.

Đường cài (`update/appimage.ts`) chứng minh được tính nguyên tử: verify size + sha256 → copy sang `<APPIMAGE>.new` cùng thư mục → `fsync` file → `chmod 0755` → `rename` → `fsync` thư mục. Crash trước `rename` để lại ảnh cũ nguyên vẹn (có test), crash sau để lại ảnh mới hoàn chỉnh. `.new` sót lại từ lần lỗi bị ghi đè.

Dịch vụ (`update/service.ts`) giữ đúng chính sách Electron: check không tải, tải cần đúng version người dùng đã thấy, cài cần artifact đã verify **và** một lần xác nhận riêng; lỗi tự động thì im lặng, lỗi thủ công mới hiện. Trước khi cài nó gọi `update-tasks` (khoá nhận request) — có task chạy thì trả `ready` và **không** cài.

**Đo thật** (`apps/desktop-tauri/scripts/update-probe.sh`, shell release + Host đóng gói + feed HTTP thật):

```
RESULT ready=OK port=38649
status_before={"phase":"available","version":"9.9.9"}
status_after={"phase":"available","version":"9.9.9"}
data: {"phase":"available","version":"9.9.9"}
feed_requests=2
```

42 unit test cho manifest/AppImage/service; 95 test toàn `apps/desktop-host`. Env do shell truyền: `DSH_DESKTOP_UPDATE_FEED` (URL manifest) và `DSH_DESKTOP_VERSION` (bản đang cài); thiếu một trong hai → **không** có dịch vụ update (idle), đúng kiểu một bản phát hành không có nguồn cập nhật.

**Còn thiếu để đóng P3:** hộp thoại xác nhận tải/cài (Electron dùng `dialog.showMessageBox`; Tauri cần dialog native hoặc UI), restart shell sau `install`, và ký gói (macOS `codesign`/notarize, Windows `signtool`). Vì chưa có chỗ xác nhận, cầu chỉ mở đường **check**; `download`/`install` đã có API + test nhưng chưa nối vào nút bấm nào.

### Gói .deb Linux (P4b, đã đo)

`apps/desktop-tauri/scripts/pack-deb.sh` dựng đúng định dạng Debian bằng `ar` + `tar` + `zstd` — máy build là **Arch, không có dpkg**, nên không thể dựa vào `dpkg-deb --build`; bộ giải nén cũng đọc `ar`/`tar` trực tiếp.

| Số đo | Giá trị |
|---|---|
| gói | **153.868.430 byte** (147 MiB) |
| cài đặt | 750 MB (`Installed-Size` 778.348 KiB) |
| layout | `/usr/lib/deepseek-harness/{bin,dsh,runtime,project,dsh-desktop-tauri}` + `/usr/bin/deepseek-harness` |
| `Depends` | `libgtk-3-0, libwebkit2gtk-4.1-0` (lấy từ `ldd` của shell) |
| `Recommends` | `libayatana-appindicator3-1` (tray được dlopen, không link cứng) |

Launcher suy `root` từ `readlink -f "$0"` rồi lùi hai cấp, nên **cùng một cây** chạy được từ cài đặt thật hoặc từ `ar x` + giải nén; profile vẫn seed sang `$DSH_HOME` và ảnh/gói không bị ghi (`image_writable_files=0`).

Kiểm chứng thật:

- `deb-probe.sh` (trong `isolated-run.sh`): giải nén read-only, stub shell kiểm hợp đồng env + seed, Host boot → `bridge_status={"phase":"idle"}`.
- Container **Ubuntu 24.04** sạch: `apt-get install /pkg/*.deb` → `Status: install ok installed`, deps resolve; launcher tạo `$DSH_HOME/desktop-project/{cordis.yml,package.json,…}`; `runtime/bin/node` là symlink đúng vào primary runtime. Shell đồ hoạ panic vì container không có display — ngoài phạm vi phép thử này.

Còn mở: tên gói cho Fedora (RPM) và cài trên máy thật có GUI.

### Update N → N+1 end-to-end (P3b, đã đo)

Host sở hữu toàn bộ luồng; shell chỉ là người trả lời hộp thoại. Trước mỗi bước không thể hoàn tác, Host gửi `@@DSH@@{"type":"update-prompt","promptId":N,"action":"download"|"install","version":"…"}`; shell hiện dialog native (`tauri-plugin-dialog`, vốn đã là dependency) rồi trả `{"type":"update-prompt","promptId":N,"accepted":bool}`.

Hai chi tiết phải đúng, nếu không là hỏng âm thầm:

1. **`promptId` tách khỏi `requestId`.** Reader của shell định tuyến *mọi* frame có `requestId` vào bảng request shell→Host; dùng chung không gian id thì một prompt của Host có thể "trả lời" nhầm một `quit-inspection` đang chờ.
2. **Restart phải chạy `$APPIMAGE`.** AppImage chạy từ mount tạm; `app.restart()`/`current_exe()` sẽ chạy lại **ảnh cũ trong mount**, không phải file vừa được ghi. Shell spawn `sh -c 'while kill -0 <pid>; do sleep 0.2; done; exec "$0"'` với `$APPIMAGE`, rồi `app.exit(0)` — chờ pid cũ chết để không đua khoá single-instance.

Đo bằng `apps/desktop-tauri/scripts/update-flow-probe.mjs`: script đóng vai shell — tự phục vụ feed HTTP, spawn Host qua stdio, trả lời prompt, và kiểm tra `$APPIMAGE` giả bị thay. Chạy trong `isolated-run.sh` (xem dưới):

```
RESULT ready=OK port=46183
prompts=download:2.0.0,install:2.0.0
open_status=204 presentation={"phase":"installing","version":"2.0.0"}
restart_frame=true
image_replaced=true mode=755
```

**Cách ly khỏi phiên desktop** (`apps/desktop-tauri/scripts/isolated-run.sh`, bwrap): PID/UTS/IPC namespace riêng, `/run` rỗng (mất socket Wayland, D-Bus, Hyprland), `/tmp` riêng, không controlling terminal, `clearenv`, `/` chỉ đọc và chỉ thư mục làm việc được ghi. Đo lại sau khi chạy: `clipse` và watcher vẫn sống, `wl-paste --list-types` exit 0. Mọi probe từ giờ chạy qua wrapper này; probe nào cần ghi vào cây thì đặt state trong `$DSH_ISOLATION_DIR`.

### AppImage Linux (P4a, đã đo)

`apps/desktop-tauri/scripts/pack-appimage.sh` ghép shell release + `bin/`, `dsh/`, `runtime/`, `project/` của tree đã chuẩn bị vào một AppDir rồi pack bằng appimagetool. **200.698.360 byte** (191 MiB) từ AppDir ~751 MB.

Ảnh AppImage là **read-only**, nên `AppRun` làm hai việc mà tree không cần: tính lại mọi đường dẫn từ `$here` (không phụ thuộc cwd), và seed profile `project/` sang `$DSH_HOME/desktop-project` ở lần chạy đầu. Mọi ghi của profile (plugin, patch) rơi vào bản sao trong `$DSH_HOME`; ảnh không bị đụng — probe đo `image_writable_files=0` sau khi boot xong.

Hai lỗi thật lộ ra khi chạy, không đoán được từ đọc code:

1. **Thiếu `DSH_CLIENT_VERSION`** → entry `desktop-product-telemetry` chết với `ValidationError: invalid config: $.serviceVersion missing required value` (config web-app đọc `process.env.DSH_CLIENT_VERSION`). `AppRun` giờ export cả `DSH_DESKTOP_VERSION` (so sánh update) và `DSH_CLIENT_VERSION` (telemetry) cùng một release identity.
2. **appimagetool bản `continuous` báo sai**: nó kết luận "Desktop file contains errors" cho một file mà chính `desktop-file-validate` của nó trả exit 0 — hàm `validate_desktop_file()` `waitpid(..., WUNTRACED | WCONTINUED)` nên `WIFEXITED` sai. Ghim **1.9.1**; bản này pack sạch.

`apps/desktop-tauri/scripts/appimage-probe.sh` kiểm chứng hoàn toàn headless (giải nén thay vì mount FUSE, `env -i` với HOME/TMPDIR/DSH_HOME/XDG riêng, `setsid`, không DISPLAY/WAYLAND):

```
RESULT appimage=DeepSeek-Harness-0.2.1-rc.2-x86_64.AppImage bytes=200698360
version=0.2.1-rc.2  client_version=0.2.1-rc.2  project_has_cordis=yes  node_exec=yes
seeded_in_home=1
RESULT host=OK port=35455
bridge_status={"phase":"idle"}
image_writable_files=0
RESULT done=1
```

Feed update lúc build: đặt `DSH_APPIMAGE_UPDATE_FEED` khi chạy `pack-appimage.sh` thì `AppRun` export nó thành `DSH_DESKTOP_UPDATE_FEED` (mặc định không có → không nguồn update).

### Làm nhẹ payload Python (D25) — 257 MB → 193 MB, đo được

`apps/desktop-tauri/scripts/prune-python-payload.ts`, gọi trong `prepare-linux-runtime.ts` ngay sau khi đọc manifest primary runtime, rồi **verify bằng import thật** (`numpy, pandas, openpyxl, docx, pptx, PIL, lxml`) — payload prune xong vẫn boot app (`ready=OK`).

| Hạng mục bỏ | Vì sao |
|---|---|
| bản sao thứ hai của `libpython3.12.so` (**−31 MB**, thành symlink) | loader chỉ cần soname; payload khoá ship hai file y hệt nhau |
| `share/terminfo` + `share/man` (−12 MB) | cơ sở dữ liệu cho TUI, app desktop không dùng |
| `pip` + `ensurepip` (−8 MB) | skill Office dặn *"Do not install packages"*; runtime không cài gì |
| `tkinter` + cây `tcl/tk/itcl/thread` + `_tkinter*.so` (−13 MB) | chỉ dùng qua GUI toolkit |
| `idlelib`, `lib2to3`, `pydoc_data`, `include/` (−4 MB) | công cụ phát triển |

| Phép đo | Trước | Sau |
|---|---|---|
| thư mục `dependencies/python` | 257 MB | **193 MB** (−64 MB, −25%) |
| số file | 6.971 | 3.802 |
| nén `zstd -19` | 58,7 MB | 47,4 MB |
| primary runtime | 401 MB | **337 MB** |
| import đủ 7 module tài liệu | ok | **ok** (verify trong prepare) |
| boot shell Tauri + RPC | `ready=OK` | `ready=OK` |

**Ba thứ đã cân nhắc và từ chối:**

1. **Strip binary** — **không được**. `strip --strip-unneeded` và cả `--strip-debug` phá layout section đã relocate của `python-build-standalone`: interpreter chết với `symbol lookup error: undefined symbol: , version` và cảnh báo `no version information available`. Bản lock vốn đã là `install_only_stripped`, nên không còn debug info để cắt.
2. **Bỏ numpy + pandas (76 MB)** — không. `packages/skill/skill-office/assets/office-xlsx/SKILL.md` nói rõ với model: *"run its Python executable with bundled `openpyxl` and `pandas`"*. Đó là hợp đồng model-visible (bất biến #3); cắt là đổi hành vi sản phẩm.
3. **Nén sẵn để giảm bản tải** — lợi rất ít: `.deb`/AppImage vốn đã nén cả payload, mà 58,7 → 47,4 MB chỉ chênh 11 MB; còn trên đĩa thì vẫn phải giải nén lại thành 193 MB. Chỉ đáng nếu phát hành dạng tarball thô hoặc làm bước tải của on-demand.

**Kết luận:** giữ Python, đã prune. On-demand (ADR-0008) vẫn là đòn lớn nhất — bỏ hẳn 193 MB khỏi bản cài — nhưng giờ bản tải về của thành phần đó chỉ còn ~47 MB.

### `dsh plugin` trên cây production — cài **và chạy** đã đo được

Đo qua shell Tauri thật với `installBundle`:

| Lời gọi | Kết quả |
|---|---|
| `@deepseek-ai/dsh-headless` (dist-tag `latest` = `0.0.1-rc.1`) | `application: "failed"` — bộ chặn phiên bản từ chối: *"is incompatible with dsh 0.2.1-rc.2 … grant the exact-version exemption … with `dsh plugin allow-version`"*. Đúng thiết kế, thông điệp có hành động. |
| `@deepseek-ai/dsh-headless@0.2.1-rc.2` | `changed: true`, `stage: "enable"`, `application: "restart-required"`; pnpm đi kèm tải 6 gói |
| `file:…/masiss-dsh-load-probe-0.2.1-rc.2.tgz` (bundle fixture của fork) | pnpm cài từ tarball cục bộ; khởi động lại thì plugin **chạy** |

Bằng chứng "chạy" của fixture `apps/desktop-tauri/scripts/load-probe/`:

- stderr của Host có `DSH_LOAD_PROBE_ACTIVE` (do chính `apply()` của plugin in ra);
- `pluginManager/listPlugins` trả `{"entryId":"include:load-probe","moduleName":"@masiss/dsh-load-probe","enabled":true,"fiberPhase":"active","patchId":"load-probe"}`.

**Vì sao phải là feature bundle:** cài thêm một *app bundle* (`dsh-headless`) làm profile có hai app bundle, lần khởi động sau hỏng (`error: unknown option '--no-open'` → `cannot create effect on inactive context`) vì bundle app thay thế bề mặt CLI của app kia. Bundle **insert** một row thì ghép được vào app đang chạy. Đây là luật của sản phẩm, giống nhau ở cả hai shell.

**Điều kiện dừng #4 của `AGENTS.md` §7 đã qua hẳn:** plugin cài lúc chạy được nạp và ở trạng thái `active`.

### Ba đường đóng (đo bằng `close-paths-probe.sh`)

Gửi `WM_DELETE_WINDOW` thật rồi đọc `map_state`:

| Trường hợp | `map_state` | Tiến trình | Host |
|---|---|---|---|
| Có tray (`DSH_DESKTOP_TRAY=1`) | 0 (ẩn ngay) | sống | còn 14 kết nối |
| Không tray, chưa xác nhận | **2 (cửa sổ vẫn hiện)** — dialog giải thích đang chờ | sống | còn kết nối |
| Không tray, đã xác nhận | 0 (ẩn) | sống | còn kết nối |

Không trường hợp nào giết tiến trình hay Host. Ảnh chụp release xác nhận menu bar **Application / Edit** và UI render đầy đủ.

Menu Application: About, (dev) Reload Page + Toggle Developer Tools `F12`, Quit `Ctrl+Q`. Menu Edit: undo/redo/cut/copy/paste/select-all — ngang Electron trên Linux.

**Chưa test được ở máy này (cần phiên desktop thật):** bấm menu tray (B18); đóng app sạch để kiểm tra file window-state; deep link `dsh://open` (shell đã focus theo argv, còn thiếu đăng ký `.desktop` ở P3).

**Còn thiếu của P1 — bản địa hoá copy của shell.** `apps/desktop/src/locale.ts` là nguồn duy nhất cho chuỗi của shell (menu, tray, dialog) và nó là TypeScript; shell Rust không import được. Chép lại chuỗi sang Rust sẽ tạo **hai nhà** cho cùng một sự thật, đúng thứ `AGENTS.md` cấm. Hiện shell dùng tiếng Anh; muốn ngang Electron phải phát chuỗi đã bản địa hoá từ một nguồn dùng chung (ví dụ Host phục vụ chúng qua HTTP, hoặc sinh một crate từ `locale.ts`).

### Đầu dò dùng lại được

| Đường dẫn | Việc |
|---|---|
| `.agents/plan/desktop-tauri/probes/measure-host.mjs` | Đo cold start → `ready`, RSS, shutdown |
| `.agents/plan/desktop-tauri/probes/headless-probe.sh` | Boot headless, kiểm tra 401/303/200, `__DSH_BOOT__`, mux 101 |
| `.agents/plan/desktop-tauri/probes/render.mjs` | Playwright headless: render UI, FCP, console error |
| `.agents/plan/desktop-tauri/probes/shell-probe.sh` | Single instance + cửa sổ + host sống |
| `.agents/plan/desktop-tauri/probes/quit-flow-probe.sh` | Thử bấm menu tray qua DBusMenu (chưa click được) |
| `.agents/plan/desktop-tauri/probes/close-paths-probe.sh` | Ba đường đóng: có tray / không tray chưa xác nhận / đã xác nhận |
| `.agents/plan/desktop-tauri/probes/bridge-probe.sh` | Cầu `/__dsh-desktop` trên HTTP thật: fence, device info, shortcut round trip, phase update |
| `.agents/plan/desktop-tauri/probes/bridge-page.mjs` + `bridge-page-probe.sh` | Quan sát cầu từ trong trang thật bằng Chromium |
| `.agents/plan/desktop-tauri/probes/session-read-probe.sh` | Host Tauri đọc session trên đĩa qua RPC `session/list` (đọc thật, không sửa) |
| `apps/desktop-tauri/scripts/update-probe.sh` | Feed HTTP thật → `updates-status` = `available`, SSE phát đúng state |
| `apps/desktop-tauri/scripts/appimage-probe.sh` | AppImage headless isolated: hợp đồng `AppRun`, seed profile, Host boot từ ảnh read-only |
| `apps/desktop-tauri/scripts/update-flow-probe.mjs` + `isolated-run.sh` | N → N+1 thật: feed HTTP, prompt qua stdio, thay `$APPIMAGE`, frame `restart` — trong sandbox bwrap |
| `apps/desktop-tauri/scripts/isolated-run.sh` | Chạy bất kỳ lệnh nào không chạm phiên: PID/UTS/IPC ns, `/run` rỗng, no TTY, env sạch |
| `apps/desktop-tauri/scripts/pack-deb.sh` + `deb-probe.sh` | Đóng gói `.deb` (ar+tar+zstd) và kiểm tra headless: launcher, seed, Host boot từ bản giải nén read-only |

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
| D13 | Đóng cửa sổ **không bao giờ** kết thúc tiến trình. Có tray ⇒ ẩn im lặng; không có tray ⇒ **giải thích một lần rồi mới ẩn**, huỷ thì cửa sổ ở lại |
| D14 | Tray chỉ được coi là "có" khi session bus thật sự có `org.kde.StatusNotifierWatcher`; `TrayIconBuilder::build()` trả Ok kể cả khi không ai render |
| D15 | Mở lại app luôn là đường về (single instance focus), nên không bao giờ có tiến trình mồ côi không lối vào |
| D16 | Cầu `window.dshDesktop` **do Host phục vụ**, không phải init script + Tauri command (ADR-0011) |
| D17 | `shortcuts` chạy đúng `ShortcutPersistence` của client package trong Host — một thuật toán, hai shell |
| D18 | `keyboard`/`shortcuts` trả disposer rỗng: một cửa sổ, client tự áp snapshot từ `get`/`edit`; `updates.subscribe` là **EventSource thật** (D26) |
| D19 | `data-platform` do chính shim đặt (thay `markDocumentPlatform` của preload Electron) |
| D20 | Directory picker **không cần Tauri dialog**: `directory-picker-auto` trong web bundle đã tự chọn chooser native của Host |
| D21 | Product analytics đã cổng sẵn ở client bằng `'dshDesktop' in globalThis`; cầu của ta thoả cổng, không cần thêm gì |
| D23 | Shim của cầu **nối tiếp** mọi lời gọi (một request một lúc): client đọc shortcut một lần cho mỗi command đăng ký, không await |
| D25 | Giữ Python trong payload nhưng **prune**: bỏ pip/ensurepip/IDLE/tkinter+tcl/tk/terminfo/header và bản sao thứ hai của `libpython3.12.so`; **không** bỏ numpy/pandas (SKILL.md dạy model dùng) và **không** strip |
| D24 | Vòng đời profile nằm trong **Host** (TS, dùng lại `@deepseek-ai/dsh-app-boot`), không viết lại bằng Rust: tạo profile + bỏ link projection + giữ đúng **một app bundle** (web). Feature bundle không bị đụng |
| D28 | Update N→N+1: Host hỏi qua frame `update-prompt` (id riêng `promptId`, không dùng chung không gian `requestId` của shell→Host), shell hiện dialog native và trả lời qua stdio; restart phải chạy `$APPIMAGE` chứ không phải `current_exe()` (mount tạm), và chờ pid cũ thoát để không đua single-instance |
| D27 | Linux phát hành dạng **AppImage**: ảnh read-only nên profile seed vào `$DSH_HOME/desktop-project` ở lần chạy đầu; `AppRun` giữ hợp đồng env của tree (thêm `DSH_CLIENT_VERSION` vì telemetry đọc nó) |
| D26 | Update: manifest tự mô tả theo `platform-arch`, tải + verify sha256/size, cài bằng **rename nguyên tử** lên `$APPIMAGE`; Q3 = **AppImage self-update**, gói `.deb` để trình quản lý gói (`unsupported`) |
| D22 | Welcome/onboarding v1 = **không tới được**: cửa sổ welcome là tài liệu native riêng cần copy của `locale.ts` (TS) và luồng đăng nhập Platform. Không chép choreography `hasApiKey` của `welcome-backend.ts` sang Host (sẽ là nhà thứ hai) |

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
| Shell P1 release (tray + dialog + window-state + devtools) | **10,3 MB** | 10.339.832 byte |
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

### B19 — `open_devtools` biến mất ở release nếu thiếu feature

`cargo build` (debug) biên dịch được, `cargo build --release` **không**: `WebviewWindow::{open_devtools,close_devtools,is_devtools_open}` chỉ tồn tại khi bật feature `devtools` của crate `tauri`. Debug thấy được vì nhánh `debug_assertions`. Phải khai báo `tauri = { features = ["tray-icon", "devtools"] }`. Electron cũng ship devTools ở bản release nên đây là ngang hàng, không phải nới lỏng.

### B20 — `TrayIconBuilder::build()` trả Ok kể cả khi không ai render icon

Trên Linux, tauri dùng libappindicator; không có StatusNotifier host thì `build()` vẫn Ok nhưng icon vô hình. Phải hỏi session bus (`NameHasOwner("org.kde.StatusNotifierWatcher")`) mới biết chắc — nếu tin vào `build()` thì app sẽ ẩn cửa sổ mà người dùng không có lối về.

### B23 — Đọc cầu đồng thời làm cạn ngân sách kết nối của trình duyệt

Client gọi `shortcuts.get` **một lần cho mỗi command đăng ký và không await** (`syncDefinitions`). Đo được `peak_in_flight=33` lời gọi cầu cùng lúc; Chromium (và WebKitGTK) chỉ cho ~6 kết nối mỗi host, nên 5 request chết với `ERR_INSUFFICIENT_RESOURCES` → `console_errors=5`, và phía client mỗi lần lỗi rơi vào `failRead()` (shortcut bị đánh dấu `unreadable`). Sửa bằng cách **nối tiếp** trong shim (`let queue = Promise.resolve()`), giữ nguyên thứ tự nên revision cũng nhất quán hơn. Sau khi sửa: `peak_in_flight=1`, `failed=0`, `console_errors=0`.

### B24 — Làn đóng gói Electron không phục vụ Linux

`prepare-dsh` không chạy được trên Linux vì hai lý do độc lập, và chúng được xử lý khác nhau:

1. **`desktop-build-paths.mjs` chặn Linux — đã ăn (ngoại lệ L2 duy nhất vào `apps/desktop/`).**
   `SUPPORTED_TARGETS` thêm `linux-x64`/`linux-arm64`, `desktopTargetPlatform` ánh xạ `platform: 'linux'`, `desktopTargetBuildPaths` giữ nguyên. Sáu dòng, chỉ thêm, không đổi hành vi mac/win.
   Nhờ đó làn Linux dùng chung `resolveDesktopTargetBuildPaths()` và ghi vào `apps/desktop/.desktop-build/targets/<target>/` — cùng layout với làn Electron, hết cảnh hai bên tự đặt đường dẫn.
   `package-target.ts` vẫn giữ bộ target riêng (mac/win) và từ chối Linux bằng thông điệp rõ: target set đặt tên **payload**, carrier nào lấp cũng được.
   *Đây là chỗ duy nhất trong toàn bộ migration sửa file Electron của upstream; `AGENTS.md` §8 cấm điều đó trừ khi có yêu cầu riêng, và yêu cầu đó là của người dùng ("ăn luôn desktop-build-paths"). Nếu upstream đổi file này, conflict nằm ở đúng dòng `SUPPORTED_TARGETS`.*
2. **`prepare-dsh.ts` chạy pnpm bằng binary Electron — không ăn.** Đây là script của carrier Electron (ADR-0001 giữ Electron cho mac/win); kéo Node vào đó là sửa đường phát hành của upstream mà chỉ fork dùng. Phần dùng lại được đã dùng hết: primary-runtime builder, `release:pack`, package set, runtime project metadata, runtime patch policy, profile init, và build paths.

### B22 — Shell Tauri truyền thiếu argv ⇒ thao tác gói rơi về `pnpm` trên PATH

`DesktopHostProcess` của Electron truyền 5 tham số vị trí:

```
argv[2] runtimeDir   argv[3] projectDir   argv[4] primaryRuntime
argv[5] pnpm.mjs     argv[6] nodeBin
```

`tauri.ts` đọc đúng argv[5]/argv[6] để dựng `packageManager`, nhưng `host_command()` trong `main.rs` **chỉ đẩy argv[2] và argv[3]**.

**Đính chính quan trọng:** đây *không* phải "tắt im lặng". `PluginManager` có `pnpmCommand: z.string().default('pnpm')` và dùng `profile.packageManager ?? { command: this.pnpmCommand }` (index.ts:451, 762), nên thiếu bộ ba thì Host rơi về **`pnpm` bất kỳ trên PATH**. Với app đóng gói, đó là phụ thuộc ngoài không kiểm soát được — hỏng khi máy người dùng không có pnpm, và chạy sai phiên bản khi có.

Đã sửa: `host_command()` đẩy đủ 5 slot, bộ ba package-manager là all-or-nothing (thiếu một cái là lỗi to, không suy diễn). Kiểm chứng bằng shim `pnpm` từ chối trong `$PATH` — xem bảng M1.

Kiểm chứng đầy đủ ở M1 (xem bảng làn đóng gói): cài từ registry, cài từ tarball cục bộ, và plugin **chạy** (`fiberPhase: active`).

### B21 — Typecheck fail vẫn emit rác vào `src/`

Khi `tsc` báo TS6059/TS6307 (file ngoài `rootDir`), nó **vẫn ghi** `.js`/`.d.ts`/`.map` cạnh source: `packages/client/shortcuts/src/{binding,configuration,native,persistence,protocol}.*` xuất hiện và làm `lint` đỏ ở file không ai sửa. `git clean -f -- packages/client/shortcuts/src/` là cách dọn; đừng commit chúng.

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
| M1 | Đo `dsh` production tree thật (`prepare-dsh`) | P0 | **xong, qua làn fork** — `prepare-dsh` không chạy được trên Linux (B24) nên đã dựng làn riêng dưới `apps/desktop-tauri/scripts/` |
| M2 | Đo Tauri release binary (Linux) | P0 | đang chạy |
| M3 | Đo RSS idle + cold start | P0 | **xong** (Host Node: 0,9–1,2 s; 266–268 MiB) |
| M4 | Xác nhận `node:sqlite` trên Bun | P0 | **xong** — chạy được |
| M5 | Chạy `dsh plugin` + load plugin trong compiled binary | P0 | **bỏ** — Bun bị bác (`B8`) |
| M6 | So sánh DACL/token provider Rust vs koffi (Windows) | P5-win32 | chưa |
| M7 | Đo lại cold start/RSS với Node 24.21 đóng gói | P1 | **xong** — 1,27–1,31 s; 309–318 MiB (cây production thật) |
| M8 | Đo RSS + cold start của shell Tauri có webview | P0 | chưa |

## Câu hỏi mở

| # | Câu hỏi | Ảnh hưởng |
|---|---|---|
| Q1 | Có cần embedded browser (sidebar) + Platform view trên Tauri không? | Nếu có: +4–6 pw, không parity đầy đủ |
| Q2 | Windows có bắt buộc trong 12 tháng tới? | Quyết định P5-win32 (5–7 pw) |
| Q3 | Auto-update Linux dùng AppImage self-update hay repo apt? | **Đã chốt: AppImage self-update** (D26); `.deb` do trình quản lý gói |
| Q4 | Có cần ký/notarize macOS ngay v1 không? | +3–5 pw |
| Q5 | Có chấp nhận Office/Python tải sau lần dùng đầu? | −273 MB nếu có |

## Ghi chú môi trường

- Bun 1.4.3, Node host 26.8.2; repo pin Node 24.21.0 cho runtime đóng gói.
- `node_modules` 4,5 GB (Bun isolated layout).
- Rust 1.98.1; webkit2gtk 2.54.1 + gtk3 + libsoup3 + appindicator **đầy đủ**, Tauri build được.
- `CI=1` bỏ qua Lefthook postinstall.
- Host `desktop` dùng được profile `~/.dsh/profiles/web` cho thí nghiệm: bỏ `skill-office`, đặt `runtimeDir=$HOME/.dsh/profiles`, `projectDir=$HOME/.dsh/profiles/web`.
- `strip` lên binary Python của python-build-standalone làm hỏng interpreter (mất symbol versioning); đừng strip, hãy prune file.
- `/tmp` là tmpfs 7,7 GB và chỉ còn ~1,6 GB: copy Node (120 MB) hay wheel vào đó chết với `Unknown system error -122` (EDQUOT). Đặt `TMPDIR` sang ổ lớn (`apps/desktop/.desktop-build/tmp` hoặc `~/.cache/…`) khi dựng primary runtime và khi chạy `apps/desktop-host/tests` (spec `office.spec.ts` copy Node).

## Lịch sử cập nhật

| Ngày | Thay đổi |
|---|---|
| 2026-10-19 | Tạo bộ tài liệu; chốt D1–D10; ghi B1–B7 |
| 2026-10-19 | P0: bác bỏ D2 (Bun) bằng chứng `B8`; sửa `B4`; thêm `B9`–`B12`; đo cold start/RSS Host Node; dựng `apps/desktop-tauri` + Tauri 2.12.4 build được |
