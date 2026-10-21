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

### Card cài Office trong Settings (P6, đã làm)

`packages/client/ui-settings-general` có thêm `DesktopOfficeSource` và `OfficeComponentRow` đăng ký vào `settings.general.item` (order 20, chỉ render khi carrier có `window.dshDesktop.components` — browser và Electron không thấy). Trạng thái: chưa cài → **Install**; downloading/verifying/installing → nút disabled kèm % hoặc nhãn bước; lỗi → câu theo nhóm (`network`, `integrity`, `python`, còn lại generic) + **Retry**; đã cài nhưng chưa `active` → **Restart to finish**; đã cài và active → mô tả kèm version, không nút. Copy nằm trong dictionary `settings` (en/zh).

Host: `DesktopOfficeChannel.restart` + route `POST /__dsh-desktop/components-restart`, nối vào `restartOffice(false)` (người dùng chủ động, bỏ qua admission); đường tự động sau khi cài vẫn là `restartOffice(true)` (chỉ restart khi không có turn chạy). Shim có `components.status/install/restart/subscribe`.

Test: `office-component.client.spec.tsx` (20 test, gồm từng loại lỗi, dispose giữa chừng, coalescing) + `desktop-bridge-components.spec.ts` (status/install/restart/503/SSE). Coverage `office-component-source.ts` 100%. Chưa quay GIF: phiên này cấm mở cửa sổ GUI trên máy người dùng.

### Office/Python theo nhu cầu (P6, đã đo)

Bundle core **không còn** `runtime/primary-runtime/dependencies/python` lẫn `office-skills`. `prepare-linux-runtime.ts` prune + verify Python như cũ, rồi **chuyển** nó (cùng `packages/skill/skill-office/assets`) vào `components/office-<version>-<target>.tar.zst` và ghi `components/components.json` (id, version, file, bytes, sha256). `runtime.json` vẫn giữ bản ghi Python — installer chép nó vào runtime ghép.

Host (`apps/desktop-host/src/components/`): `manifest.ts` validate; `tar.ts` đọc zstd + ustar tự viết (không thêm dependency, chặn entry/link thoát thư mục); `office.ts` tải → sha256 → giải nén → ghép runtime (`runtime.json` từ primary, `node`/`pnpm` symlink) → chạy `python3 -c 'import numpy, pandas, openpyxl, docx, pptx, PIL, lxml'` → `current.json` (atomic). Cài đặt tuần tự; lỗi phân loại `feed|feed-network|download|download-network|integrity|archive|python|install`. Bridge: `POST /__dsh-desktop/components-status|components-install`, SSE `/components-events`. Cài xong Host gửi frame `restart` (chỉ khi không có turn đang chạy); lần khởi động sau `tauri.ts` thấy bản cài rồi `ctx.plugin(desktopOffice, {source, root})` và báo `active:true`.

| Số đo | Trước | Sau |
|---|---|---|
| primary runtime | 337 MB | **144 MB** |
| component archive | — | **59.789.924 byte** (57 MB; giải nén 193 MB + assets) |
| AppImage | 200.698.360 | **143.735.288** |
| .deb | 153.888.540 | **112.839.092** |

`office-component-probe.sh` (bwrap, không GUI) chạy trên **cả cây build lẫn AppImage đã giải nén**:

```
bundle_python=absent
status_initial={"installed":false,"phase":"idle"}
status_install={"installed":true,"phase":"ready","version":"0.2.1-rc.2"}
restart_frame=yes
python_imports=ok
status_restarted={"installed":true,"active":true,"phase":"ready"}
```

Bẫy: project seed **không có `cordis.yml`** — Host tạo nó khi boot; mọi check đóng gói phải nhìn `project/package.json` (đã sửa ở `pack-appimage.sh`, `pack-deb.sh`, hai probe). `bin/desktop-host` là script sinh bởi `prepare-linux-runtime.ts`, nên đổi flag Node phải sửa ở đó rồi chạy lại.

### Gói .rpm cho Fedora (P4b, đã đo)

Host build là Arch, **không có** rpm/rpmbuild, nên `pack-rpm.sh` dựng bằng `rpmbuild` trong container `fedora:41` (mount thư mục build, chown trả lại uid host). Spec tắt `__os_install_post` (đừng strip Node trong payload), `AutoReqProv: no`, payload `w19.zstdio`. Layout và launcher dùng chung `stage-payload.sh` với `.deb`.

| Số đo | Giá trị |
|---|---|
| gói | **119.926.560 byte** (114 MiB) |
| cài đặt | 558 MB, **20.857 file** |
| Requires | `gtk3`, `webkit2gtk4.1`; Recommends `libayatana-appindicator-gtk3` |

`rpm-probe.sh` trên **Fedora 41 sạch**: `dnf install` OK, `rpm -V` OK, launcher đúng hợp đồng env + seed `$DSH_HOME`, Host boot (`RESULT host=OK`). Bẫy: probe phải export **cả `DSH_CLIENT_VERSION`**, không chỉ `DSH_DESKTOP_VERSION` — thiếu nó telemetry báo `$.serviceVersion missing` và Host dừng.

### A4 — RSS idle của Host (đã đo, đạt)

Đo bằng `rss-probe.sh` trong `isolated-run.sh`: Host headless từ cây đã đóng gói, ổn định 15 s. WebKitGTK là process tree riêng nên **không** tính — đúng cách con số 309–318 MB trước đây được đo.

| Cấu hình | RSS | heapTotal / heapUsed |
|---|---|---|
| mặc định | 314–324 MB | 217 / 78 MB |
| `--max-old-space-size=128` | 281 MB | — |
| `--max-old-space-size=64` | **không boot được** | — |
| `--optimize-for-size` | **170–179 MB** | 73 / 68 MB |

V8 chọn kích thước heap theo RAM của máy (32 GB ở đây), không theo nhu cầu: gần như toàn bộ 140 MB chênh lệch là heap dự trữ. Launcher `bin/desktop-host` thêm `--optimize-for-size`; `DSH_DESKTOP_NODE_FLAGS=` (rỗng) tắt nó. Đánh đổi: JIT/GC bớt tham lam, không đáng kể với workload chờ model; heap vẫn mở rộng khi session cần (không đặt trần old-space). Trên AppImage đã giải nén: `host_rss_kib=179684`, `node_flags=--optimize-for-size`.

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
| `apps/desktop-tauri/scripts/pack-deb.sh` + `deb-probe.sh` | Đóng gói `.deb` (ar+tar+zstd) và kiểm tra headless: launcher, seed, Host boot từ bản giải nén read-only |\n| `apps/desktop-tauri/scripts/office-component-probe.sh` | Cài Office theo nhu cầu từ feed loopback, kiểm import Python, restart để Office compose |\n| `apps/desktop-tauri/scripts/rss-probe.sh` | Đo RSS idle của Host headless (A4) |\n| `apps/desktop-tauri/scripts/pack-rpm.sh` + `rpm-probe.sh` | Đóng gói `.rpm` bằng rpmbuild trong fedora:41 và kiểm tra cài đặt + Host boot trên Fedora sạch |

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
| D30 | **Engine Office đi theo component on-demand** (ADR-0008 mở rộng): `libreoffice-kit` + `libreoffice-kit-wasm` (144 MB) nằm trong archive component, không trong payload lõi. Loader trỏ `@deepseek-ai/libreoffice-kit` (và subpath `package.json`, `cli`) vào bản đã cài, đọc từ `$DSH_HOME/components/current.json`; plugin `office-to-pdf` trong profile vẫn import được. Payload Python được **di chuyển** ra khỏi primary runtime sang `.office-python` trong build tree để archive dựng lại được khi engine đổi |
| D31 | **Ngân sách kích thước được đặt lại (A3′)**: artifact tải về ≤ 105 MB (AppImage/.deb/.rpm) và cây cài đặt ≤ 400 MB. Ngưỡng cũ "cài đặt ≤ 105 MB" bất khả thi: Node 24.21 stripped một mình đã 103,5 MB, phần còn lại là chính các package của harness |
| D29 | Provider native của Desktop nằm trong **một crate napi-rs duy nhất** `native/dsh-native` (D4): PTY (`portable-pty`), flock của session lease, binding `execve`/`fcntl` của bootstrap, và pipeline ảnh (`image` + `zune-png` + `fast_image_resize` + libwebp tĩnh). Cùng crate build binary `landlock-run` (UAPI Landlock viết thẳng trong Rust). Thay provider **không sửa `packages/`**: `apps/desktop-host/native/native-loader.mjs` đăng ký resolve hook trỏ `node-pty` → `node-pty.cjs`, `@deepseek-ai/node-addon-system/flock` → `flock.cjs`, `koffi` → `koffi.cjs`, `sharp` → `sharp.cjs`; launcher Tauri nạp hook bằng `--import` trước khi profile boot. Thiếu `.node` ⇒ hook no-op ⇒ payload giữ addon gốc ⇒ lane Electron không đổi |
| D32 | **GitHub Release là nguồn feed**: một release Linux mang cả artifact (AppImage/.deb/.rpm) lẫn tài liệu feed (`manifest.json` cho updater, `components.json` + archive Office cho component on-demand). URL tải của release ổn định theo tag ⇒ artifact **baked** sẵn `DSH_DESKTOP_UPDATE_FEED`/`DSH_COMPONENT_FEED` trỏ vào thư mục tải của chính release đó, và bản sau publish ở tag mới. Script `apps/desktop-tauri/scripts/publish-release.ts` sinh tài liệu feed **từ cây build** (sha256 + size của đúng byte sẽ upload) rồi `gh release upload --clobber`; lane `desktop-release.yml` chạy thủ công (`workflow_dispatch`, nhận tag) vì phát hành là hành động có chủ đích, không tự chạy theo branch push |

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

### B25 — napi `ThreadsafeFunction` gọi callback kiểu callee-handled, và `portable-pty` trả tên tín hiệu theo locale

Hai lỗi âm thầm khi viết binding napi:

1. `ThreadsafeFunction<T>` mặc định dùng `ErrorStrategy::CalleeHandled`: callback JS nhận **`(err, value)`**, không phải `(value)`. Viết `(data) => ...` thì `data` là `err` (luôn `null`) ⇒ mọi chunk dữ liệu thành chuỗi `"null"`.
2. `portable_pty::ExitStatus::signal()` trả **`Option<&str>` từ `strsignal()`** — tên tín hiệu *theo locale* (`"Terminated"`, có thể dịch). Muốn **số tín hiệu** (service dùng `signalName(exitSignal)`) thì phải tự `waitpid()`: crate giữ `Child` nhưng ta thả nó ngay sau `spawn` và tự reap bằng `nix::sys::wait::waitpid` (`Exited` → code, `Signaled` → `128 + sig` + `signal: Some(sig)`).

Ngoài ra `kill` phải gửi cho **process group** (`killpg`) rồi fallback `kill(pid)`, vì `node-pty` giết cả nhóm; và `pause()` chỉ là cờ: thread đọc ngủ 5 ms mỗi vòng, backpressure do buffer pty đảm nhiệm.

### B26 — `pnpm pack` bỏ qua mục `files` không tồn tại

Thêm `"native/dsh-native.node"` vào `files` của `apps/desktop-host` không làm hỏng lane Electron: `pnpm pack` bỏ qua file thiếu và vẫn exit 0. Nhờ vậy payload Tauri (có `.node`) và payload Electron (không có) dùng chung một manifest.

### B27 — Đọc output của terminal phải chờ `end`, không chỉ `done`

`handle.done` resolve khi tiến trình con thoát, còn chunk cuối cùng của pty vẫn còn trong buffer của stream. Probe đọc `output` ngay sau `done` thì ra chuỗi rỗng. Phải chờ thêm `output` phát `'end'`.

### B28 — `Vec<String>` của napi không nhận phần tử `null`

`linux-execve.ts` truyền mảng C-string đã có null terminator (`[...argv, null]`). Binding `Vec<String>` của napi từ chối `null` (`ArrayExpected` / `Failed to convert napi value Null`). Shim phải lọc null trước khi gọi binding.

### B29 — `execve` đóng fd có `FD_CLOEXEC`, và điều đó là bình thường

fd 0/1/2 của tiến trình Node trong shell của agent là **socket** và mang `FD_CLOEXEC`; sau `execve` chúng biến mất nên chương trình mới không in được gì (exit 1, không có output) dù exec thành công. Đó chính là lý do `linux-execve.ts` xóa cờ `FD_CLOEXEC` trước khi exec. Probe phải làm y vậy, nếu không sẽ tưởng binding hỏng.

### B31 — `landlock-run` viết lại bằng Rust: dùng UAPI trực tiếp, không dùng crate `landlock`

Crate `landlock` có API khác hẳn (ruleset/ABI negotiation của riêng nó), còn hợp đồng CLI của launcher đã ghim (`native/system/docs/cli-contract.md`): exit 125, dòng báo cáo `landlock: fully enforced` / `partially enforced (older ABI)`, mọi lỗi chết kèm `landlock-run: ` trên stderr. Viết thẳng UAPI trong Rust (như bản C) giữ **bề mặt kiểm toán giống hệt** và bảo đảm ma trận hành vi khớp từng dòng. `nix::fcntl::flock` cũng bị deprecate (guard `Flock` **unlock khi drop** — sai với ngữ nghĩa "giữ khoá tới khi fd đóng") nên phải gọi `libc::flock` trực tiếp.

### B32 — `Vec<u8>` của napi là **Array** số, không phải Uint8Array

Binding `Vec<u8>` nhận mảng JS và trả mảng JS; truyền `Buffer` vào sẽ lỗi `ArrayExpected`, và mảng số 12 MP là rất chậm. Dùng `napi::bindgen_prelude::Buffer` cho cả tham số lẫn giá trị trả về.

### B33 — Decoder PNG của crate `image` chậm hơn sharp ~80 lần

Bộ giải mã PNG của crate `image` (dựa trên crate `png`) mất ~2,1 s cho ảnh 12 MP, còn libspng của sharp ~27 ms. Thay bằng `zune-png` (SIMD) và `fast_image_resize` cho resize: còn ~198 ms cho decode+resize+encode (sharp 99 ms). `zune-png` cần gọi `decode_headers()` trước khi đọc `get_colorspace()`/`get_info()`, và `decode()` trả `DecodingResult` (U8/U16/F32) chứ không phải buffer.

### B34 — Ảnh 16-bit và JPEG CMYK phải giữ đúng sự thật, không nâng về 8-bit

`sharp` báo `depth: 'ushort'`/`space: 'rgb16'` cho PNG 16-bit và `space: 'cmyk'` cho JPEG 4 kênh. Nếu hạ về 8-bit sRGB ngay lúc decode, ảnh 16-bit **được pass-through** (sai) và JPEG CMYK **được coi là hợp lệ** trong cache (sai). Số kênh của SOF JPEG cho biết CMYK; PNG 16-bit phải giữ buffer `u16` để báo đúng depth.

### B38 — `TMPDIR` trong repo làm hỏng lane snapshot

Harness snapshot chạy agent trong một thư mục tạm; nếu thư mục tạm nằm **trong repo** (ví dụ `TMPDIR=apps/desktop/.desktop-build/tmp`), phần dò hướng dẫn đi ngược lên tìm thấy `AGENTS.md` của repo và chèn thêm một message `agent-instructions` vào transcript ⇒ **173 test snapshot đỏ** dù không có thay đổi nào trong sản phẩm. Đặt `TMPDIR` ngoài repo (ví dụ `/tmp/…`) thì lane xanh trở lại. Đừng sửa fixture để dập triệu chứng này.

### B44 — Runner CI không có sẵn backend sandbox: lane snapshot đỏ vì "no sandbox backend is usable"

Trên máy phát triển có `bwrap` nên lane snapshot xanh. Runner `ubuntu-latest` của GitHub **không** có `bwrap`, và package npm `@deepseek-ai/node-addon-system-<platform>` **không** chứa binary `landlock-run` (chỉ có `system.node`) ⇒ backend sandbox không khả dụng ⇒ `sandbox-local` từ chối chạy lệnh ⇒ các ca snapshot gọi tool (SDK, PTC) khác transcript đã ghi. Lane CI phải cài `bubblewrap` (hoặc dựng launcher Landlock của lane Tauri).

### B46 — Hai ca replay subagent SDK chỉ đỏ trên runner 2 vCPU của CI

`replays subagent-dsh-sdk-diagnostic` và `replays subagent-dsh-sdk-dynamic-route` (snapshots/sdk) đỏ trên runner `ubuntu-latest` (2 vCPU) với `ENOENT .../.child-dsh/sessions` ⇒ tiến trình con (một `dsh` lồng nhau) không khởi động được. **Cùng hai ca xanh trên máy phát triển** (2/2 pass) và toàn lane xanh 211/211 tại đó. CI của upstream chạy lane này trên runner 4 vCPU/16 core ⇒ khả năng cao là thiếu CPU cho tiến trình con. Việc cần cho lane của fork: runner nhiều hơn, hoặc chấp nhận hai ca này là ngoại lệ của lane.

**Đã đóng (2026-10-21):** đúng là thiếu CPU, nhưng nguyên nhân là *song song*, không phải số nhân. Đo lại bằng `taskset -c 0,1` (giới hạn 2 CPU) cho thấy lane **treo 8,5 giờ** ở đúng hai ca này: tiến trình con không bao giờ khởi động xong, còn `vitest` chờ `turn/end` vô hạn. Chạy tuần tự (`DSH_SNAPSHOT_MAX_CONCURRENCY=1`, `DSH_SNAPSHOT_MAX_WORKERS=1`) trên cùng 2 CPU ấy thì **211/211 xanh trong 367 s**. Lane CI của fork đã đặt hai biến đó ⇒ mỗi ca replay có trọn runner, tiến trình con luôn kịp khởi động. Không sửa expected output.

### B47 — `renameSync` dùng mà không import ⇒ lane đóng gói trên runner sạch chết ngay
`apps/desktop-tauri/scripts/prepare-linux-runtime.ts` gọi `renameSync(pythonDir, officePython)` (chuyển payload Python ra khỏi primary runtime) nhưng **không import** hàm đó từ `node:fs`. Máy phát triển không lộ vì cây build local đã có `.office-python` sẵn (nhánh `else if` chạy, không đi qua `renameSync`); một runner sạch có Python trong primary runtime thì đi đúng nhánh đó và chết với `ReferenceError: renameSync is not defined`. Đã sửa (thêm `renameSync` vào import `node:fs`). Cùng lúc đó, `officeComponent` có thể `undefined` ở chỗ đọc `.file` ⇒ thêm một `throw` tường minh khi không có component nào.

**Bài học:** các script của lane Tauri **không nằm trong chương trình typecheck** nào (`tsconfig.host.json` chỉ liệt kê `apps/desktop/scripts/**`), nên `tsc` không bắt được cả hai lỗi trên. Đã thêm `apps/desktop-tauri/scripts/**/*.ts` vào `include` của `tsconfig.host.json` ⇒ `bun run typecheck` giờ kiểm tra cả script của lane.

### B48 — Cài workspace bằng Bun ⇒ runtime con không resolve được package của workspace

Hai ca replay `subagent-dsh-sdk-*` đỏ trên CI với `ENOENT .../.child-dsh/sessions`: tiến trình con (`dsh` lồng nhau) **không khởi động được** nên không ghi session nào. Nguyên nhân: lane cài bằng `bun install`, mà layout **isolated** của Bun giữ mỗi package workspace trong `node_modules/.bun/<pkg>@<ver>/node_modules/`, nên Node **không** tìm thấy package đó khi resolve theo tên từ thư mục profile. Layout **hoisted** của pnpm đặt mọi package workspace vào `node_modules/.pnpm/node_modules/` ⇒ Node tìm thấy.

Đo lại trên worktree sạch (cài `bun install`): `bun install` ⇒ **2 ca đỏ** với đúng `ENOENT .child-dsh/sessions`; `pnpm install` ⇒ **2/2 xanh**. CI của upstream cài bằng pnpm vì lý do này. Đã đổi lane `desktop-linux.yml` sang `pnpm install --frozen-lockfile` (kèm `pnpm/action-setup` + Node 24).

**Bẫy phụ:** nếu thư mục kịch bản nằm *bên trong* repo (ví dụ `TMPDIR=<repo>/.tmp`), thư mục làm việc của runtime con dây lên tới repo ⇒ nó đọc `AGENTS.md` của repo ⇒ lệch transcript. Kịch bản phải nằm ngoài repo (CI dùng `/tmp` nên không dính). Không sửa expected output.


### B45 — Decoder PNG của crate `image` quá chậm cho đường ảnh sản phẩm

`image` (dựa trên crate `png`) giải mã ảnh 12 MP mất ~2,1 s, còn libspng của sharp ~27 ms ⇒ ảnh đính kèm sẽ "treo" UI. Dùng `zune-png` (SIMD) cho PNG và `fast_image_resize` cho resize: còn 171 ms cho resize+encode 4000×3000 (sharp 97 ms).

### B43 — Đọc output của PTY theo chunk sẽ cắt UTF-8; và exit phải báo sau khi reader ráo hết

`String::from_utf8_lossy` trên từng chunk biến một code point bị cắt giữa hai lần đọc thành ký tự thay thế. Reader phải giữ đuôi chưa hoàn chỉnh và chỉ giải mã phần là trọn vẹn. Tương tự, frame `exit` phải báo **sau** khi reader đã ráo hết, nếu không người tiêu dùng có thể thấy exit trước output cuối. Cả hai đều là hợp đồng của `node-pty` mà provider Rust phải giữ.

### B42 — Lane CI không được gộp toàn bộ bộ test toàn repo trong một job

Chạy `bun run test` (toàn repo) trong một job của runner 2 vCPU làm ca *process-bound* `packages/workflow/workflow-ptc` vượt thời gian mặc định (upstream chạy chính ca đó với `--testTimeout 90000`). CI của upstream chia bộ test theo nhiều runner. Lane của fork chỉ chạy các package bị đụng + desktop shell, còn bộ test toàn repo thuộc CI upstream.

### B41 — `pnpm/action-setup` không được đặt `version` khi repo đã có `packageManager`

Repo khai báo `"packageManager": "pnpm@11.28.5"`; action `pnpm/action-setup` từ chối khi workflow **cũng** đặt `version` ("Multiple versions of pnpm specified"). Bỏ `version` khỏi action để nó đọc `packageManager`. Lane CI của fork còn bỏ hẳn pnpm: các lệnh đi qua Bun, và pnpm chỉ cần cho đóng gói desktop (chạy ở máy có primary runtime).

### B40 — Credential trên máy là token OAuth của Codex, không dùng được cho provider `openai-codex`

`~/.dsh/.credentials.yaml` chỉ có `OPENAI_CODEX_API_KEY` (một token OAuth của Codex) và settings trỏ model `gpt-5.6-terra` qua provider `openai-codex`. Provider đó **không** dùng được token đó ngoài luồng Codex: `PI_AI_ERROR: Failed to extract accountId from token`. Profile `headless` mặc định dùng route `deepseek-official` nên cần `DEEPSEEK_API_KEY` thật. Lưu ý: settings trong `~/.dsh` là `settings.yaml.imported` (chưa import) — copy sang `settings.yaml` trong một `DSH_HOME` tạm cũng **không** đổi model của profile headless (profile đè settings).

### B39 — Start page của Tauri: `WebviewUrl::App` + `navigate`, và Host phải spawn không chặn

Cửa sổ start: dựng bằng `WebviewUrl::App(PathBuf::from("index.html"))` (Tauri nhúng `frontendDist` vào binary ✓ nên không cần đóng gói thêm), rồi `window.navigate(url)` khi Host báo `ready`. `HostClient::spawn` phải **không chặn**: nó đọc frame `ready` trên một thread riêng và gọi callback `on_ready` (URL) — nếu `setup()` chặn chờ URL thì event loop không chạy ⇒ start page không bao giờ vẽ. Nhánh `on_page_load` phải báo **mọi** sự kiện (`Started`/`Finished`): nếu điều hướng tới URL xảy ra trước khi trang start nạp xong, sự kiện `Finished` đầu tiên thuộc về trang ứng dụng chứ không phải trang start.

### B38 — Cửa sổ start page: `WebviewUrl::App` dùng `frontendDist` đã nhúng, không cần đóng gói thêm

Tauri nhúng `frontendDist` (ở đây là `apps/desktop-tauri/dist`) vào binary, nên trang start chỉ cần một file `index.html` trong thư mục đó. `WebviewUrl::App(PathBuf::from("index.html"))` mở nó ngay, rồi `window.navigate(url)` đưa sang Host. Nhánh `HostClient::spawn` phải **không chặn**: đọc frame `ready` trên thread riêng, còn `setup()` trả về ngay để event loop vẽ được.

### B37 — Đo "first paint" của app Tauri: cần weston headless, và milestone phải có mốc thời gian thật

`registerHooks`/`LazyLock` trong Rust khởi tạo **lần dùng đầu tiên** ⇒ mốc `Instant` phải được ép (`LazyLock::force`) ở đầu `main()`, nếu không mọi milestone báo 0 ms. Weston headless tạo socket `wayland-<N>` (không phải luôn `wayland-0`) và đường dẫn socket phải **≤ 108 byte** ⇒ thư mục làm việc phải ngắn (dùng `/tmp` riêng của sandbox, không dùng đường dẫn cây build). Shell cần `DSH_DESKTOP_HOST_BIN` (không có thì spawn Host fail) và profile đã seed sẵn (không đo lần chạy đầu).

### B35 — Payload Python có symlink **tuyệt đối** trỏ vào máy build

`python-build-standalone` để lại `bin/python`, `bin/python3`, `lib/libpython3.12.so` là symlink tuyệt đối trỏ vào thư mục build. Tar reader của Host từ chối link rời khỏi thư mục giải nén ⇒ archive fail với `failure: "archive"`. Bước build phải **tương đối hoá** symlink (đổi thành tên file trong cùng thư mục) trước khi đóng gói.

### B36 — Component phải dựng lại được sau khi payload Python rời primary runtime

P6 chuyển Python khỏi primary runtime vào archive ⇒ lần prepare sau không còn Python để đóng gói lại, nên engine Office mới không vào được archive. Payload Python được giữ ở `.office-python` trong build tree (không thuộc release) và archive dựng lại từ đó mỗi lần prepare.

### B30 — `koffi` **có** dùng trên Linux, không chỉ Windows

`packages/subprocess/subprocess-local/src/linux-execve.ts` (bootstrap một lần) bind `execve` + `fcntl` của libc qua koffi. Nếu prune koffi khỏi payload Linux mà chưa thay binding, bootstrap chết và terminal không bao giờ ra `READY`. Đã thay bằng crate Rust + shim `koffi.cjs` (chỉ nhận đúng hai signature, signature lạ thì báo lỗi tường minh).

### B15 — Không chụp được cửa sổ Tauri bằng `import` trên phiên Wayland

`import -window root` fail (`missing an image filename`); `grim` chụp được toàn màn hình nhưng cửa sổ Tauri không nằm trong khung hình chụp. Cách kiểm chứng đáng tin là `ss -tnp`: thấy `WebKitNetworkProcess` mở kết nối ESTAB tới cổng Host.

## File L2 đã sửa (fork, phải ghi để upstream merge không đụng)

Theo `AGENTS.md` §3 của lane: L0 = `packages/core/*`, `packages/llm/*`, `packages/session/*`, `packages/bundle/*`, `apps/desktop/src/*` — **không đụng** (kiểm bằng `git diff --name-only origin/master...HEAD -- < các đường dẫn L0 >` → rỗng ✓).

| File | Lý do | Ghi chú |
|---|---|---|
| `apps/desktop/scripts/desktop-build-paths.mjs` | Cho phép target Linux (một dòng `SUPPORTED_TARGETS`) | Ngoại lệ L2 duy nhất vào `apps/desktop/` (B24) — conflict upstream nằm đúng dòng đó |
| `packages/client/ui-settings-general/**` (7 file + 2 test + README) | Thẻ cài Office on-demand trong Settings | L2: không thuộc danh sách L0; đã ghi ở PLAN Phần X (phát hiện #5) |
| `packages/extensions/cordis-client-runner/src/client/slot-catalog.ts` | File sinh tự động theo slot đăng ký (thẻ Office) | Sửa bằng `pnpm run gen-client-catalog`; CI gate `verify-client-catalog` bắt buộc |

Mọi thay đổi còn lại nằm ở L1 (thêm mới): `apps/desktop-tauri/**`, `apps/desktop-host/native/**`, `apps/desktop-host/src/components/**` (component manager), `native/dsh-native/**`, `apps/desktop-host/src/office.ts`.

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
| M9 | Đo parity provider Rust cho PTY | P5-posix | **xong** — `native-pty-probe.sh`: `provider=rust`, `size="33 111"`, `write_echo=2`, `signal="SIGTERM"`; 546 test (`terminal` + `tool-bash-persistent` + `subprocess-local`) xanh khi chạy qua loader |
| M10 | Đo parity provider Rust cho flock + execve | P5-posix | **xong** — `native-flock-probe.sh`: `flock_provider=rust`, `koffi_provider=rust`, `contention=EAGAIN/11/flock`, `fcntl=ok`, `execve-ok`; `lease.two-process.e2e.ts` xanh; payload Linux không còn `koffi`/`node-pty`/`system.node` |
| M11 | Đo parity launcher Landlock Rust vs C | P5-posix | **xong** — `landlock-parity-probe.sh`: `parity=identical` trên 20 dòng ma trận (usage error, exit code, thông điệp, hiệu ứng ràng buộc); `landlock-sandbox-probe.sh`: read-only chặn ghi, `/dev/null` ghi được, workspace-write ghi được, `enforcement=full` |
| M17 | Đo update check trên Linux + lane CI của fork (A6) | A6 | **Linux đạt** — `update-probe.sh`: `status_before`/`status_after` = `{"phase":"available","version":"9.9.9"}`, event SSE đến, `feed_requests=2`. Lane CI của fork: `.github/workflows/desktop-linux.yml` (build → typecheck → lint → test các package bị đụng → snapshot → cargo build shell → cargo test crate → test Host). **Không** đóng gói trong CI: payload desktop cần *primary runtime* mà lane `prepare-dsh` của upstream chỉ chạy cho mac/win (B24) ⇒ đóng gói + size gate chạy ở máy có primary runtime. Bộ test toàn repo thuộc CI của upstream (chia nhiều runner); chạy gộp trong một job làm một ca *process-bound* quá thời gian trên runner chậm |
| M18 | Provider ảnh Rust: facts + hiệu năng | P5-posix | **xong** — facts giống sharp 5/5 định dạng (kể cả depth `ushort`/`rgb16`, alpha GIF, orientation EXIF, metadata EXIF/ICC); 93 test `attachment-local` xanh trên provider Rust; decode PNG dùng `zune-png` (SIMD) + resize `fast_image_resize`: resize+encode 4000×3000 còn **171 ms** (sharp 97 ms; bản đầu 3,2 s vì decoder PNG của crate `image`) |
| M16 | Đọc session cũ + transcript diff (A8/A9) | A8/A9 | **A8 đạt**: `session-read-probe.sh` đọc `~/.dsh/sessions` thật → 7 session (5 `.v4` + 2 legacy `migration-required`), `list_ok=1 list_items=7`. **A9 đạt phần keyless**: `test:snapshot` 211/211 + `test:expected` 122/122 xanh. Phần model thật còn mở (không có key) |
| M15 | Đo cold start của shell (A5) | A5 | **đạt ngân sách "first paint"** — start page vẽ xong **406–480 ms** (weston headless trong sandbox): `window-created` 231–259 ms · lần vẽ đầu 406–480 ms ✓ · `host-url` 2669–2787 ms (runner 2 vCPU) · nội dung ứng dụng 2824–2993 ms. Trước start page: 4358–5084 ms. Bridge trả `{"phase":"idle"}` ✓ |
| M13 | Đo kích thước sau A3 (engine Office → component + strip) | A3 | **xong** — AppImage 90.966.520 B (86,7 MiB), .deb 70.182.036 B (66,9 MiB), .rpm 76.423.369 B (72,9 MiB), cây cài đặt 353.180.656 B (337 MiB); trước: 138/108/114 MB artifact, 546 MB cài đặt |
| M14 | Xác minh .deb/.rpm sau A3 | A3 | **xong** — `deb-probe.sh`: cài + Host boot OK; `rpm-probe.sh`: `rpm -V` OK, `size=381M`, Host boot OK |
| M19 | Phát hành Linux từ GitHub runner | P4 | **xong (1 target)** — lane `desktop-release.yml` chạy thật trên `ubuntu-latest`: **12 phút 24 s** từ tag push; artifact AppImage 90.917.368 B; size gate xanh; release `desktop-v0.2.1-rc.2` trên fork có 4 asset (AppImage + `manifest.json` + `components.json` + archive Office 107.236.690 B); sha256/size trong manifest tải về khớp **đúng byte** đã upload. macOS/Windows còn mở (cần chứng thư, P3) |
| M12 | Đo parity provider Rust cho ảnh vs sharp | P5-posix | **xong** — `image-parity-probe.sh`: facts giống hệt 5/5 định dạng, WebP lệch 0–6% (ví dụ 734/724 B); 93 test `attachment-local` xanh trên provider Rust; resize+encode 198 ms so với sharp 99 ms (4000×3000); AppImage 138.004.984 B (131,6 MiB) sau khi bỏ `sharp` + `@img` |

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

### Tài nguyên cần dọn (tạo trong máy này, 2026-10-21)

| Tài nguyên | Dung lượng | Cách dọn |
|---|---|---|
| Docker image `fedora:41` | 238 MB | `docker rmi fedora:41` — `pack-rpm.sh` kéo về để có `rpmbuild` |
| Docker image `ubuntu:24.04` | 117 MB | `docker rmi ubuntu:24.04` — `deb-probe.sh` kéo về để cài `.deb` sạch |
| Wine prefix `~/.wine` | 2,4 GB | `rm -rf ~/.wine` — dùng để chạy gate `scripts/wine-windows-gates.sh` (Node win-x64 dưới Wine) |
| Wine cache `.cache/wine-windows/` | 416 MB | `rm -rf .cache/wine-windows` — cache Node win-x64 + prefix dùng lại |
| `/tmp/opencode/*` | vài GB | `rm -rf /tmp/opencode` — scratch của phiên làm việc |

Ba container đang chạy (`dbx`, `his-nhidong-orthanc`, `postgres`) **không** thuộc lane này ⇒ đừng đụng. Lane dùng `docker run --rm` nên không để lại container.

## Lịch sử cập nhật

| Ngày | Thay đổi |
|---|---|
| 2026-10-19 | Tạo bộ tài liệu; chốt D1–D10; ghi B1–B7 |
| 2026-10-19 | P0: bác bỏ D2 (Bun) bằng chứng `B8`; sửa `B4`; thêm `B9`–`B12`; đo cold start/RSS Host Node; dựng `apps/desktop-tauri` + Tauri 2.12.4 build được |
| 2026-10-20 | P5-posix (1): crate napi-rs `native/dsh-native` thay `node-pty` bằng `portable-pty`; loader resolve hook `node-pty`; thêm D29, B25–B27 |
| 2026-10-20 | P5-posix (2): crate `native/dsh-native` thêm flock của session lease và binding `execve`/`fcntl` (thay koffi trên Linux); payload Linux không còn `koffi`/`node-pty`/`system.node`; thêm B28–B30, M10 |
| 2026-10-20 | P5-posix (3): launcher `landlock-run` bằng Rust (UAPI trực tiếp) thay bản C trong payload — ma trận hành vi giống hệt; thêm B31, M11 |
| 2026-10-20 | Kiểm chứng audit bên ngoài (`DSH_Tauri_Source_Audit_2026-10-20.md`): 9/12 finding đúng, 3 lỗi thời, **đã sửa F01** (quit đọc cả `scheduledTasks`), **F05** (trần byte khi tải component Office), **F06** (UTF-8 bị cắt ở ranh giới chunk + thứ tự exit trước data cuối) và **F09 một phần** (ghim Bun 1.4.3); thêm mục kiểm chứng trong PLAN |
| 2026-10-20 | A6: lane CI Linux cho fork (`.github/workflows/desktop-linux.yml`) + đo lại update check trên Linux; crate `dsh-native` gate binary Landlock theo `target_os`; thêm M17, B40 |
| 2026-10-20 | F02/F03/F07 của audit: Host spawn không chặn + deadline 60 s + hộp thoại lỗi; shutdown chờ `shutdown-complete`; tar extraction có ngân sách (3 test). Deep link: gói cài đặt chạy `update-desktop-database` (deb `postinst`, rpm `%post`) |
| 2026-10-20 | A5: thêm start page (cửa sổ mở ngay, webview chạy song song với Host, điều hướng khi `ready`) — first paint 4,4–5,1 s → **2,7–3,2 s**; thêm B39 |
| 2026-10-20 | A8/A9: đọc session cũ thật (7 session, `list_ok=1`) + transcript diff không cần key (snapshot 211/211, expected 122/122); thêm M16, B38 |
| 2026-10-20 | A5: đo cold start bằng weston headless trong sandbox (shell báo milestone JSON) — 4,4–5,1 s tới `page-load-finished`, **chưa đạt** ngân sách 1,5 s; thêm M15, B37 |
| 2026-10-20 | Audit toàn diện: A1–A10 được kiểm lại bằng bằng chứng chạy được; 3 lỗi `test:web:built` chứng minh **có sẵn** (checkout `f8d5be74c2^` → vẫn đỏ); danh sách file L2 được ghi; mục A5/A6/A9 ghi rõ điều kiện đóng |
| 2026-10-20 | A3: engine Office (144 MB) chuyển vào component on-demand; strip Node và module native; bỏ `sharp`/`@img`/`node-pty`/`koffi`/`system.node`; size gate `size-gate.sh`; artifact ≤ 105 MiB; thêm D30–D31, B35–B36, M13–M14 |
| 2026-10-20 | P5-posix (4): pipeline ảnh bằng Rust thay `sharp` (decode `zune-png`, resize `fast_image_resize`, WebP libwebp tĩnh, JPEG zune) — facts giống hệt, WebP lệch 0–6%, 93 test `attachment-local` xanh; payload không còn `sharp`/`@img`; thêm B32–B34, M12 |
| 2026-10-21 | P1: copy của shell lấy từ `apps/desktop/src/locale.ts` (generator `generate-locale.ts` → `locale.rs`); CI kiểm tra bản sinh không cũ |
| 2026-10-21 | P4: release Linux từ GitHub runner (lane `desktop-release.yml`, D32) — release `desktop-v0.2.1-rc.2` thật, 4 asset, sha256 khớp byte; thêm M19 |
| 2026-10-21 | Sửa hai lỗi chặn lane: `renameSync` thiếu import trong `prepare-linux-runtime.ts` (runner sạch chết) + script của lane Tauri nằm ngoài chương trình typecheck; lane CI cài workspace bằng pnpm (layout isolated của Bun làm runtime con không resolve được package workspace); thêm B47–B48 |
