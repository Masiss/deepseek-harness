# Bàn giao — lane Desktop Tauri (fork `Masiss/deepseek-harness`)

Cập nhật 2026-10-22. Mục đích: session mới đọc file này là biết đang ở đâu, cái gì đã xong, cái gì nên làm tiếp và bẫy nào đã mắc.

## 1. Trạng thái repo

| Mục | Giá trị |
|---|---|
| Nhánh | `chore/web-bun-migration` (upstream `origin/master` không đụng) |
| HEAD | Bump `0.2.1-rc.4` cùng các sửa lỗi Office/WebKit đã push; CI Desktop Linux run **39774116751** xanh ✓ |
| Version | cây làm việc `0.2.1-rc.4` (bump **349** package, local); **release đã publish vẫn** `0.2.1-rc.3` — xem bẫy §5 |
| Release đã publish | `desktop-v0.2.1-rc.3-linux-x64` + `-linux-arm64`, feed `desktop-feed-linux-{x64,arm64}` **có chữ ký Ed25519** ✓ |
| Lint | `bun run lint` sạch (1 warning có sẵn ở `packages/experimental/client-ui-cot-translation`) |
| Typecheck | `bun run typecheck` 5/5 ✓ |
| Lane CI | `.github/workflows/desktop-linux.yml` xanh ✓ (bước `Desktop shell tests` + `Keyless recorded-session replay` + `Desktop Host tests`) |
| Lane release | `.github/workflows/desktop-release.yml` xanh ✓ 19m54s cho 2 target |

## 2. Đã xong (đừng làm lại)

- **F04** feed update **ký Ed25519** (schema v2 + `minimumVersion`), public key ghim trong `apps/desktop-host/src/update/signing.ts`, private key trong secret `DSH_RELEASE_SIGNING_KEY` của fork.
- **F06** module Rust `dsh-native` encode PNG/TIFF/GIF + đọc/ghi metadata (EXIF/XMP/IPTC/ICC); shim `sharp.cjs` **không còn đường delegate** ⇒ payload không cần gói sharp.
- **F10** phần chung của hai entrypoint Host gom vào `apps/desktop-host/src/desktop-launch.ts`.
- **F11** budget sandbox (cgroup v2: cpu/RAM/disk/pids) trong `isolated-run.sh`.
- **F12** **lease** trên state desktop (`$DSH_HOME/desktop-host.lease`) do cả hai entrypoint giữ.
- **A9** đã chạy với **model thật**: `live-transcript-probe.sh` (lượt thật qua route Codex ⇒ session ghi ⇒ replay ⇒ hai log đã chuẩn hoá khớp từng byte).
- Route **Codex** dùng được (xem §6).
- Đã **dọn ~21 GB** rác máy (cây runtime lane, build Rust, output build, `~/.wine`, image Docker, `/tmp/opencode`).

## 2b. Goal 2 — release-grade (đang mở, 2026-10-22)

Mục tiêu và acceptance đầy đủ ở `PLAN.md` § "Cấu trúc mục tiêu". Bằng chứng mới trong session này:

- **Packaged journey** (`packaged-journey-probe.sh`, L2): cài plugin từ tarball → Host apply → restart → gỡ; tạo/rename Session → restart → đọc lại. Tất cả PASS.
- **Fault injection** (`fault-probe.sh`, L2): thiếu runtime, profile hỏng, crash -9 → takeover lease, shutdown sạch, offline update, offline component. Tất cả PASS.
- **Disk-full** (`disk-full-probe.sh` dưới `isolated-run.sh` 48 MB): tải component bị ENOSPC, `phase=error`, `failure=download`, không còn file dở/`current.json`. Source map ENOSPC/EDQUOT/EFBIG/EROFS sang `download` thay vì `download-network`; unit test mới trong `components.spec.ts`.
- **Crash leftovers**: `DesktopOfficeComponent.recover()` chạy lúc Host khởi động, xoá `download-*`/`staging-*` còn lại, không đụng `versions/`/`current.json`; `fault-probe.sh` thêm kịch bản `crash-leftovers` ⇒ PASS.
- **Kill giữa download**: `crash-download-probe.sh` phục vụ archive chậm, kill -9 sau khi file dở > 64 KiB; boot lại ⇒ `left_before=1`, `cleaned=0`, `current=absent`.
- **Kill giữa extract**: `crash-extract-probe.sh` kill -9 khi `staging-*` xuất hiện; boot lại ⇒ `staging=0`, `versions=0`, không cài dở. `recover()` cũng xoá version không được `current.json` trỏ tới.
- **Update trọn luồng** (`update-flow-probe.mjs`): sau D40 probe này **bị hỏng** (manifest schema 1 + không ký) và đã sửa: schemaVersion 2, ký Ed25519 bằng key tạm, truyền `DSH_DESKTOP_UPDATE_PUBLIC_KEY`; giờ `RESULT update-flow=OK` (prompt download/install, `restart`, ảnh được thay).
- **Profile contention**: `desktop-lease-probe.sh` vẫn `OK`.
- **Sharp fallback**: `image-parity-probe.sh` 11/11; khác byte JPEG/GIF/TIFF/resize là đã chấp nhận (D39).
- **Benchmark**: Host cold start 1,98–2,09 s (warm), idle RSS 179–182 MiB; shell 0,55 s → 0,77 s → 4,54 s trên weston headless. App **đang chạy** trên session thật: **RSS 719–728 MiB / PSS 551–559 MiB** (web 330–338, shell 138–139, Host 192–193, network 58–59), FPS 57, app nạp 67 tài nguyên / 5,2 MB. Probe mới `graphics-probe.sh` (RSS/PSS/FPS/tài nguyên/ảnh webview), `office-engine-probe.sh` (engine Office).

**L1 đã có**: Desktop Linux lane xanh tại [run 39740515325](https://github.com/Masiss/deepseek-harness/actions/runs/39740515325), gồm bước mới `Desktop shell tests` (drift gate, runtime policy, CSP).

**Bộ nhớ (đo lại 2026-10-22 trên GUI thật)**: mặc định cuối = **không chỉnh WebKit** (đường GPU + dmabuf) ⇒ **719–728 MiB RSS / 551–559 MiB PSS**, FPS 57, UI render đầy đủ. Bẫy 1: đo trong weston headless (không GPU) cho kết quả **ngược** (tắt dmabuf thấy nhẹ hơn 73 MiB) — trên máy có GPU thật, tắt dmabuf **nặng hơn ~185 MiB**. Bẫy 2: `hardware-acceleration-policy=Never` tiết kiệm ~10 MiB nhưng **cửa sổ đen** trên session (webview vẽ nội bộ 60 fps, màn hình không đổi) ⇒ **đã bỏ khỏi mặc định**; phải so *màn hình* (`grim` + `content_mean`), ảnh chụp từ webview không bắt được lỗi này.

**Engine Office trong AppImage (2026-10-22)**: cây runtime không mang `@deepseek-ai/libreoffice-kit` (engine đi theo component on-demand) ⇒ plugin `office-to-pdf` của profile không import được ⇒ app in `1 entry did not activate` và mất tính năng chuyển PDF. Sửa: loader trỏ engine về stub khi không có component lẫn engine (`native/office-engine-stub/`), nên plugin activate và lượt convert báo `unavailable`. Probe `office-engine-probe.sh` (trong release lane) chứng minh cả hai trạng thái.

**Mặc định WebKit cuối (2026-10-22)**: **không chỉnh gì** — đường GPU + renderer dmabuf, như WebKit mặc định. `DSH_DESKTOP_WEBKIT_{ACCEL,CACHE,WEBGL,DMABUF}` còn lại chỉ để **đo lại**, và `DSH_DESKTOP_WEBKIT_LIGHT=1` chỉ tắt page cache + WebGL (không tắt compositing gia tốc — cái đó làm màn đen).

**Sửa theo source audit (2026-10-22)**: 8 findings trong source audit report đã được tái hiện và sửa (chi tiết trong verdict đi kèm): gate cold-start/resident giờ hỏi **chính WebView** (`dsh: webview-auth=200`) thay vì `curl` của probe; probe không còn giết tiến trình lạ và dùng `mktemp -d`; Host báo lỗi khi chết sau READY (`ready` về false, có unit test); shell xoá cookie `dsh-auth-*` của WebKit trước lần đổi token nên cookie jar không phình nữa; retry token so **origin**; console webview chỉ bật khi `DSH_DESKTOP_WEB_CONSOLE=1`; launch token được redact trong log; quit hỏi Host ở thread riêng. Còn mở: cổng hồi quy cho `custom-protocol`/431 (chỉ có unit test), và workload typing/scroll (B4 — sau khi revert thì **không còn cặp cấu hình để A/B**, nên chuyển thành số ngân sách).

**L3 đã có (GUI thật trên máy)**: chạy app thật trên Hyprland và chụp màn hình (`grim`) ⇒ UI render đầy đủ. Ba lỗi chỉ lộ ra ở L3 đã sửa: release thiếu feature `custom-protocol` (start page trắng), cookie `SameSite=Strict` bị WebKit từ chối khi đi từ site của start page (app hiện "authentication required" ⇒ shell lặp yêu cầu token từ chính tài liệu của Host), và cookie jar phình theo port ⇒ Node trả `431` (Host xoá cookie authority cũ + `maxHeaderSize` 64 KiB).

**GUI thật (2026-10-22, session trên máy)**: app chạy trên Hyprland, UI render đầy đủ (sidebar, composer, model picker), `webview-auth=200`, FPS 57; model picker hiện **“GPT-5.6 Luna Default”** ⇒ route Codex (`gpt-5.6-luna`) **đã trả lời thật** (một lượt CLI trả đúng sentinel `LUNA_OK`). Phép so bắt buộc cho mọi đổi đồ họa: chụp **màn hình** (`grim -o eDP-1` rồi đo `content_mean`) — màn đen là `content_mean≈0`, 1 màu.

**Probe mới đã vào release lane** (`packaged-journey`, `fault-probe`, `crash-download`, `crash-extract`, **`office-engine`**); lane release chưa chạy lại từ 21/10 ⇒ bằng chứng packaged mới là L2 local. Còn lại của Goal 2: B1 phát hành (để AppImage đã sửa tới được máy) → B2 chạy lại release lane → B3 header CSP + lượt model thật trong GUI → B4 ngân sách bộ nhớ + workload → B5 portability → B6 phần deferred.

## 2c. Trạng thái máy sau session 2026-10-22 (tự dọn khi xong)

| Thứ | Vị trí / trạng thái |
|---|---|
| App đang chạy | Tiến trình `dsh-desktop-tauri` trên **workspace 3** (tiêu đề “DeepSeek Harness”), cửa sổ hiển thị UI. Kill bằng pid trong `/home/masis/.cache/dsh-gui/app.pid`; **instance cũ giữ tên D-Bus single-instance** nên app mới sẽ thoát im lặng nếu chưa kill cái cũ |
| Profile test riêng | `/home/masis/.cache/dsh-gui/project` — patch Codex + `gpt-5.6-luna` (`llm-pi-ai.providers.openai-codex`, `agent-default-model`, `agent-loop.agents[main]`) |
| Home riêng của app test | `/home/masis/.cache/dsh-gui/home` (`.dsh/.credentials.yaml` là **bản sao** credential thật, mode 600) |
| Credential Codex thật | `~/.dsh/.credentials.yaml` ⇒ grant `llm-pi-ai/openai-codex` (OAuth, hết hạn ~25 h kể từ 21/10); nguồn gốc: `~/.pi/agent/auth.json` (script `apps/desktop-tauri/scripts/codex-credential.ts`) |
| Cây build | `apps/desktop/.desktop-build/targets/linux-x64` — đã vá tay loader + stub (probe dùng cây này) |

## 2d. Hướng mới: desktop native (egui) — 2026-10-22

Sau grill với chủ dự án, lane **chuyển hẳn** sang ứng dụng native (Rust + egui, `tao`/`winit`); **Tauri rời khỏi lane**; Host Node vẫn là sidecar ✓ và client native dùng **cùng API** với web client ✓. Bản release webview `0.2.1-rc.4` là bản cuối của hướng webview ✓.

- Quyết định + mốc + điều kiện "xong": `PLAN.md` § "Goal 3 — Desktop native (egui)" và ADR-0012 ✓.
- Bốn điều kiện xong: parity đủ màn hình · **RSS < 300 MiB** (đo trên máy thật) · probe của lane chạy trên app native · AppImage ký được + update flow thật ✓.
- Số đo quyết định: app webview **719–728 MiB**; WebKit ~530 MiB kể cả trang trắng; Host ~190 MiB; app chỉ nạp **5,23 MB** mà nó dùng (**0 B** không với tới được) ⇒ đổi composition không giúp gì, chỉ bỏ webview mới giảm được ✓.
- Mốc đầu tiên: **M1 `client-core` Rust** (lớp protocol) — cổng chặn: đối chiếu với client web trên cùng Host ✓.

### Đã xong sau session 2026-10-22 (M1–M9)

| Mốc | Kết quả | Commit |
|---|---|---|
| M1 | `client-core`: envelope, auth (token→cookie), transport HTTP + cancel; inventory 73 request + 12 stream | done, API correctness re-audited 2026-10-23 |
| M2 | `session`: list/page + tool-call status; fixture recorded-session | protocol done; egui viewer partial |
| M3 | `chat` (prompt/cancel) + `follow` (Gateway mux stream) | protocol + basic egui panel, no model probe |
| M4 | `models`, `sessions`, Remote Events approvals | API clients + allow/reject UI, no Host probe |
| M5 | `terminal` (PTY RPC + mux streams) | API + basic egui panel, no ANSI renderer/probe |
| M6 | `settings`, `plugins`, `skills`, `schedules`, `workspace_files` | clients + binary decoder; settings viewer partial |
| M7 | Native egui shell, Host sidecar/auth, tray/dialog, packaging | partial; signed updater, self-contained AppImage/release wiring pending |
| M8 | `PARITY.md` checklist | in progress; native probes missing |
| M9 | `UPSTREAM.md` process | process written; no release-cycle evidence yet |

**Số đo sàn bộ nhớ:** Host base-only **≈ 180 MiB RSS / 168 MiB PSS**; Host webview ~190 MiB ⇒ bỏ web bundle chỉ ~10 MiB; webview ~530 MiB là chi phí lớn. Ước tính native **240–290 MiB** ⇒ `< 300 MiB` khả thi.

### Cập nhật tiếp theo — 2026-10-23 (working tree, chưa commit)

- Sửa wire incompatibilities do audit: `rpcId`, `/api/<method>`, named `payload.args`, typed void response; unary calls có exact-body tests.
- Native auth đổi sang `/connect?token=…&interactions=answer` để nhận API operator cookie; Gateway mux dùng `IntoClientRequest`, authenticated cookie, `open/item/end/cancel`, remote failure, and stream cancellation/drop. `$events` approval result dùng browser outcomes `allowed-once`/`rejected`.
- Thêm multipart/form-data decoder (`multer`) và integration vào unary HTTP decoder; thêm tests raw bytes/path/parts.
- Native app: Host start/auth/list/page chạy worker; Host stdio giữ open, drains output, gửi graceful `shutdown` rồi timeout/kill fallback; chat basic, approval controls, terminal panel (plain text, bounded display), optimistic settings editor, Host text preview, and session management controls.
- AppImage script prepares a self-contained AppDir by copying the packaged Host/runtime tree; structural validation passed, but no AppImage was built/launched or published.
- Auto-install executable được disable cho tới khi có signed artifact contract; HTTPS metadata check có timeout/semver, nhưng không thay file.
- Checks đã chạy: client-core 62 unit + 8 multipart tests pass; native-app 16 tests pass; native-app and Tauri `cargo check` pass; JSON and diff checks pass. `bun run test:docs` runs 20/21 gates; remaining repository-reference failure is the known `python/sdk/uv.lock:113` entry. Chưa chạy release build (build-cost control), GUI Host probe, snapshot, or RSS measurement.

**Còn lại (chưa xong):** reconnect/resume/shared physical mux; 8 stream endpoints; in-flight HTTP cancellation; prompt file-upload integration; plugin/marketplace/deliverables/feedback/goals/jobs/todo/plan UI; richer image/office preview; approval lifecycle real-Host probe; terminal ANSI/PTY rendering and input-window backpressure; signed updater; AppImage launch probe/release wiring/signing; native GUI probes and measured RSS < 300 MiB.

## 3. Việc nên làm tiếp (theo thứ tự giá trị)

**Ưu tiên: Linux trước — macOS và Windows để pending** (2026-10-22). Lane chỉ mở việc cho macOS/Windows khi có runner + chứng thư; ma trận portability trong PLAN là hồ sơ, không phải việc đang làm.

**Kế hoạch chốt Goal 2 nằm trong `PLAN.md` § "Kế hoạch chốt Goal 2 (2026-10-22)"**: B1 phát hành `0.2.1-rc.4` → B2 chạy lại release lane → B3 bằng chứng GUI (CSP + lượt model thật) → B4 chốt ngân sách bộ nhớ + workload typing/scroll → B5 portability → B6 lượt soi phần deferred.


### (A) 7 phát hiện của audit bảo mật — **ĐÃ SỬA HẾT (2026-10-22)**
Bản đầy đủ: `~/security-audit-skill/deepseek-harness/run-1/` (`REPORT.md`, `FINDINGS-DETAIL.md`, `findings.json`).

| # | Mức | Đã sửa | Test hồi quy |
|---|---|---|---|
| 1 | **high** | `components.json` ký Ed25519 bằng đúng key update feed (`signedComponentBytes`), Host verify trước khi tải byte nào; feed chỉ HTTPS/loopback (D40) | `components.spec.ts` (unsigned/tampered/bad-scheme) |
| 2 | medium | bom giải nén zstd + trần `bytes` manifest — xem §3.1 | `components.spec.ts` (bomb 2.106 B) |
| 3 | medium | `image_scan.rs` đọc flags VP8X bằng `data.get(start)` ⇒ hết panic/SIGABRT | `image_scan_tests.rs` (empty VP8X + flags) |
| 4 | medium | `appimagetool` ghim sha256 cho cả hai arch; `fedora:41` ghim digest | `bash -n` + verify thủ công digest tải về |
| 5 | low | bridge gọi `parseShortcutEdit`/`parseShortcutDefinitions`, trả 400 | `desktop-bridge.spec.ts` (2 ca) |
| 6 | low | `installAppImage` verify **bản staged** (size + digest) trước rename | `update-appimage.spec.ts` (`verifyArtifact`) |
| 7 | low | `releaseAdmission` nhả lock khi driver fail (khớp rollback Electron) | `update-service.spec.ts` |

Đã chạy: `bunx vitest run apps/desktop-host` 148/148 ✓, `cargo test --lib` 14/14 ✓, `bun run typecheck` 5/5 ✓, `bun run lint` sạch (1 warning có sẵn) ✓. `office-component-probe.sh` giờ tự ký manifest bằng key tạm và truyền public key cho Host.

### (A1) Sửa xong: bom giải nén zstd (2026-10-22)
- `apps/desktop-host/src/components/tar.ts`: `extractZstdTar` nhận `limits` và **chặn luồng giải nén khi nó đang ghi** (`boundedStream` ⇒ vượt `totalBytes` là throw) ⇒ file scratch luôn bị xoá kể cả khi thất bại.
- `apps/desktop-host/src/components/manifest.ts`: `MAX_COMPONENT_ARTIFACT_BYTES` (1 GiB) — manifest khai byte lớn hơn thì bị từ chối, nên tải về cũng bị chặn.
- Test hồi quy: `components.spec.ts` — archive 2.106 B nén ra 64 MiB bị từ chối với `size budget` và scratch bị dọn.
- Đã đo lại: bom 2.106 B ⇒ `RESULT bomb=REFUSED`, `/tmp` không còn bị chiếm (fixture cũ 1,8 GB đã xoá).
- **Archive Office thật vẫn cài được**: `office-component-probe.sh` ⇒ `status_install` ready, `python_imports=ok`, engine compose ✓.

### (B) F14 — **ĐÃ SỬA (2026-10-22)**
Cây `dsh/node_modules` mang 24 package `dsh-experimental-*` + model `sherpa-onnx-*` (dùng bởi một package experimental duy nhất) trong khi profile desktop chỉ mount `@deepseek-ai/dsh-base` + `@deepseek-ai/dsh-web-app`. `linuxRuntimeFileExclusion` lọc cả hai nhóm (D41) ⇒ `dsh/node_modules` **255 → 190 MB**, AppImage **90 → 76.446.200 B**, cây cài **290 MB** (budget 400). Đã kiểm: không package runtime nào ngoài `packages/experimental` phụ thuộc chúng (`sdk/client` chỉ devDependency). Test: `apps/desktop-tauri/tests/linux-runtime-file-policy.spec.ts`. Bằng chứng: `entry-parity=OK`, `appimage-probe` `host=OK`, `size-gate=OK`.

### (C) Bằng chứng vào CI — **ĐÃ LÀM (2026-10-22)**
- `entry-parity-probe.sh` chạy trong **lane release** ngay sau khi dựng cây runtime, trước khi đóng gói/phát hành; probe tự ký component manifest bằng key tạm (bắt buộc từ D40).
- **Gate drift** `apps/desktop-tauri/scripts/desktop-drift-gate.ts`: so `origin/<base>...HEAD`, chỉ cho `apps/desktop/package.json` + `apps/desktop/scripts/desktop-build-paths.mjs`; đã chạy thật (`2 changed file(s), all allowed`), test riêng.
- Lane CI `desktop-linux.yml` thêm `permissions: contents: read`, `fetch-depth: 0`, `persist-credentials: false`.
- **Không** thêm bước build Electron vào CI (giữ nguyên lý do cũ).

### (D) A9 — diff transcript với model sống
Đường đã có: `live-transcript-probe.sh` (lượt thật + replay + diff). Nếu cần *diff* với bản ghi sẵn của upstream thì phải dùng provider deepseek (cần `DEEPSEEK_API_KEY`); nếu không, diff live-vs-replay (hiện tại) là bằng chứng đủ cho "transcript không đổi".

### (E) F13 — đã **chốt** là khác biệt đã chấp nhận (D39)
Resample của libvips khác `fast_image_resize` (max 6/255 mỗi kênh) ⇒ byte ảnh chuẩn hoá lệch giữa hai carrier ⇒ A1/A9 chỉ đúng với phiên **không** đính kèm ảnh. Đã ghi vào PLAN/MEMORY. Nếu muốn đóng hẳn thì phải port arithmetic của libvips (nặng).

### (F) Cần môi trường ngoài máy này
P3 ký macOS/Windows (chứng thư) · P4 ba target còn lại (runner macOS/Windows) · P5-win32 phần ACL (runner Windows) · P6 brotli/ripgrep (tùy chọn) · welcome/onboarding (đã hoãn).

### (G) Khoảng trống §4 — **ĐÃ XỬ LÝ (2026-10-22)**
- Workflow CI: `permissions: contents: read` + `persist-credentials: false` + `fetch-depth: 0` (cho drift gate) ✓
- `landlock-run.rs`: 5 unit test cho `parse` + `fs_mask_for_abi` ✓
- `profile-lifecycle.ts`: `prepareProfile` chạy **sau** lease, lỗi thì nhả lease (D42) ✓
- `tauri.conf.json`: bỏ `csp: null`, bỏ feature `devtools` ở release (D43) ✓
- `lib.rs`: `spawn` từ chối pid 0; `kill` bỏ qua pid ≤ 0 ⇒ không bao giờ `killpg`/`waitpid` vào process group của Host ✓
- `office.ts#officeEngineCli`: **chấp nhận** — engine chỉ đến từ component archive đã verify chữ ký + sha256 (D40); sửa cây sau khi cài là quyền của chính user (cùng mức với việc sửa payload)
- `native/{koffi,flock}.cjs` + `execve`/`fcntl`: vẫn deferred (không có phát hiện mới; cần một lượt soi riêng nếu muốn đóng)

## 4. Khoảng trống phủ kín còn lại (chưa ai soi)
Audit để lại 13 unit deferred (ngân sách agent) — đáng để soi trước:
- **`.github/workflows/desktop-linux.yml`**: không khai báo `permissions:` và `checkout` **không** `persist-credentials: false` ⇒ token nằm trong `.git/config` khi job chạy code của người đóng góp ⇒ nên thêm `permissions: contents: read` + `persist-credentials: false`.
- `native/dsh-native/src/bin/landlock-run.rs` — launcher sandbox mới (fail-closed) chưa có unit.
- `apps/desktop-host/src/profile-lifecycle.ts` — **sửa profile trước khi lấy lease** ⇒ hai carrier có thể sửa cùng profile.
- `apps/desktop-host/native/{koffi,flock}.cjs` + `lib.rs#execve/fcntl` — FFI thay thế.
- `apps/desktop-tauri/src-tauri/tauri.conf.json` — `csp: null` + feature `devtools` trong mọi build.
- `apps/desktop-host/src/office.ts#officeEngineCli` — chạy engine lấy từ cây component **chưa ký**.
- `native/dsh-native/src/lib.rs#spawn/kill` — pid 0 trong `killpg`/`waitpid`.

## 5. Bẫy đã mắc (đừng mắc lại)

| Bẫy | Ghi chú |
|---|---|
| **Bump version phải bump MỌI package** | Một bản release dùng **một** version cho cả family ⇒ bump thiếu là `pack-linux.sh` fail (`dsh release members must share one version`) hoặc `prepare-linux-runtime.ts` fail (`@deepseek-ai/dsh@0.2.1-rc.2 does not match Desktop 0.2.1-rc.3`). Đã bump 343 file. |
| **`tsgolint` bản WASM làm hỏng gate lint** | `oxlint-tsgolint@npm:@tsgolint-rust/wasm32-wasip1` không đọc nổi stdin ở chế độ oxlint stream chương trình ⇒ `bun run lint` fail. Đã khôi phục `oxlint-tsgolint@7.0.2001` (bản native đi kèm oxlint 1.81.0). |
| **Private key ký release không được nằm trên đĩa** | Đã từng có bản sao ở `/tmp/opencode/signkey.pem` (mode 644) ⇒ đã xoá. Key chỉ ở secret `DSH_RELEASE_SIGNING_KEY` của fork. |
| **Bun không chạy được runtime** | Native addon cần Node ⇒ `node --import tsx/esm apps/cli/src/bin.ts ...` (không phải `bun`). |
| **`TMPDIR` trong repo làm hỏng lane snapshot** | Đặt `TMPDIR=/tmp/opencode/...` hoặc `apps/desktop/.desktop-build/tmp`. |
| **Cây đóng gói đã bị dọn** | `apps/desktop/.desktop-build/targets/linux-x64/` phải dựng lại (`pack-linux.sh` + `prepare:primary-runtime` + `prepare-linux-runtime.ts`) trước khi chạy `update-probe.sh`, `desktop-lease-probe.sh`, `image-parity-probe.sh`, `entry-parity-probe.sh`, `office-component-probe.sh`. |
| **Probe cần `apps/desktop/.desktop-build/tmp` writable** | Không chạy được trên `/tmp` tmpfs còn trống ít. |
| **Hai phiên Codex song song giết trẻ của nhau** | Chạy 2 CLI session cùng lúc ⇒ subagent của session cũ bị dừng giữa chừng ("stopped before it finished"), không để lại thay đổi. Grant OAuth Codex xoay vòng refresh token ⇒ session mới làm hỏng token của session cũ. **Một phiên Codex tại một thời điểm**; các vòng subagent chạy tuần tự |
| **Probe L3 phải dùng binary release của cargo** | AppDir đóng gói trong cây build có thể cũ hơn các hook đo (cờ console) ⇒ probe không thấy `webview-auth=200` và bị chặn. Các probe khác dùng `apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri` ✓ |
| **Đừng commit khi subagent còn đang sửa file** | Hook pre-commit (lint-staged) **stash phần chưa stage** rồi restore; nếu phần đó đang là thay đổi của child thì restore **fail** ⇒ commit abort **và thay đổi của child bị mất** (đã xảy ra 1 lần với `apps/desktop-tauri/src-tauri/src/main.rs`). Chỉ commit khi **không còn phiên Codex nào chạy**, và `git add` hết phần unstaged trước khi commit |
| **Probe chạy chậm khi máy bận** | `packaged-csp-probe.sh` chờ auth tới 120 s ⇒ khi còn session Codex chạy song song thì probe có thể vượt timeout của người gọi. Chạy probe khi máy rảnh |
| **Node strip-only không chịu parameter property** | Các probe boot Host *từ source* phải qua tsx (đã sửa `bridge-probe.sh`, `pty-probe.sh`, `terminal-page-probe.sh`). |
| **Đo đồ họa phải nhìn MÀN HÌNH, không chỉ webview** | `hardware-acceleration-policy=Never` làm webview vẽ nội bộ 60 fps mà cửa sổ vẫn **đen**; ảnh chụp từ API webview vẫn "0 pixel khác" vì nó chụp nội bộ. Phép so đúng: `grim -o eDP-1` rồi đo `content_mean` (đen ⇒ ≈0, 1 màu) |
| **Đo trong weston headless cho kết quả NGƯỢC** | Headless không có GPU: tắt dmabuf thấy nhẹ hơn 73 MiB, trên máy có GPU thật nó **nặng hơn ~185 MiB**. Mọi số đo đường đồ họa phải đo trên session thật |
| **Cây build trong repo resolve được engine Office** | `node_modules` của repo có `@deepseek-ai/libreoffice-kit` ⇒ probe local **không** thấy lỗi "không import được" của AppImage. Lỗi đó chỉ lộ ở bản đóng gói; probe phải chạy trên cây `.desktop-build` (đã có `office-engine-probe.sh`) |
| **Không gõ bàn phím vào session người dùng đang bận** | `wtype` gõ vào cửa sổ đang có focus; khi focus rơi sang cửa sổ khác (zen/kitty) thì phím vào việc của họ. Luôn kiểm `hyprctl activewindow` trước khi gõ, và dừng nếu họ đang dùng máy |
| **`pkill -f` giết chính shell của agent** | Mẫu lệnh khớp với dòng lệnh đang chạy ⇒ shell bị SIGKILL. Dùng `pkill -x` với tên process bị cắt 15 ký tự, hoặc pid file, hoặc `python3` + `os.kill` theo `comm` |
| **Instance cũ giữ tên D-Bus single-instance** | App mới thoát im lặng (không log) vì tên D-Bus còn bị giữ. Kill instance cũ trước khi chạy lại, và kiểm `busctl --user list | grep SingleInstance` |
| **`/tmp` có quota theo user** | `df` còn trống mà vẫn `Disk quota exceeded` khi dconf/gio ghi file. Dùng `TMPDIR` ngoài `/tmp` (ví dụ `~/.cache/dsh-probe`) cho app và probe |

## 6. Route Codex (đã cắm, dùng được ngay)

- Credential: grant record `llm-pi-ai/openai-codex` trong `~/.dsh/.credentials.yaml` (0600 ✓) — copy từ store của pi bằng `apps/desktop-tauri/scripts/codex-credential.ts` ✓ (token còn ~25 h ⇒ provider **tự refresh** ✓).
- **Route không được đặt `apiKeyEnv`** (sẽ lấy API key nền tảng ⇒ provider từ chối) — profile `web` của bạn đã sửa ✓.
- Model dùng được với tài khoản ChatGPT: `gpt-5.6-terra`, `gpt-5.6-sol`, `gpt-6.1-sol` (không phải `gpt-5.5`).
- CLI: `node --import tsx/esm apps/cli/src/bin.ts --profile headless --patch apps/desktop-tauri/scripts/codex-model.patch.yml "câu hỏi"`.
- Provider ngoài (đã chứng minh, `EXTERNAL_OK` từ OpenRouter): khóa key vào store dưới dạng **ref** ⇒ route tên nó bằng `apiKeyEnv` ⇒ `llm-pi-ai` tự dựng provider (không sửa `packages/`).

## 7. Lệnh để tiếp tục

```sh
cd /home/masis/deepseek-harness
bun run typecheck && bun run lint                 # gate (lint ~90 s; lint hẹp: bunx oxlint <path>, 3 s nhưng có thể báo giả "error type" ở file import build output)
bunx vitest run apps/desktop-host                # test Host
cargo test --manifest-path native/dsh-native/Cargo.toml --lib
cargo test --manifest-path native/dsh-native/Cargo.toml --features landlock-bin --bin landlock-run
bun apps/desktop-tauri/scripts/desktop-drift-gate.ts origin/master   # chỉ 2 file apps/desktop được đổi
bash apps/desktop-tauri/scripts/update-probe.sh          # feed ký hợp lệ → available; sửa chữ ký → idle
bash apps/desktop-tauri/scripts/desktop-lease-probe.sh   # lease: hai carrier không ghi chung state
bash apps/desktop-tauri/scripts/image-parity-probe.sh    # shim ảnh vs sharp thật
bash apps/desktop-tauri/scripts/live-transcript-probe.sh  # model thật ⇒ replay ⇒ diff 0
bash apps/desktop-tauri/scripts/entry-parity-probe.sh    # cả hai entrypoint Host boot
bash apps/desktop-tauri/scripts/office-engine-probe.sh  # engine Office: component có/không
bash apps/desktop-tauri/scripts/graphics-probe.sh        # RSS/PSS/FPS/tài nguyên + ảnh webview
bash apps/desktop-tauri/scripts/cold-start-probe.sh      # webview-auth=200 + cold_start=OK
# B1 (khi được duyệt phát hành): bump `0.2.1-rc.4` đã có sẵn trong branch history
git push fork chore/web-bun-migration                    # đẩy commit bump lên fork
git tag desktop-v0.2.1-rc.4 && git push fork desktop-v0.2.1-rc.4   # kích hoạt lane release
```
Phát hành (làm thay đổi trên `fork`): bump version **mọi package** ⇒ tag `desktop-v<version>` ⇒ lane release tự chạy ⇒ feed mới đã ký.

## 8. Tài liệu lane
`.agents/plan/desktop-tauri/PLAN.md` (kế hoạch + bảng finding F01–F13) · `MEMORY.md` (quyết định D1–D39, số đo M1–M25, bẫy B1–B52) · `AGENTS.md` (luật lệ của lane) · `probes/` (probe cũ) — cập nhật cả hai file đầu khi có quyết định mới.
