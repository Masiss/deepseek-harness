# Bàn giao — lane Desktop Tauri (fork `Masiss/deepseek-harness`)

Cập nhật 2026-10-22. Mục đích: session mới đọc file này là biết đang ở đâu, cái gì đã xong, cái gì nên làm tiếp và bẫy nào đã mắc.

## 1. Trạng thái repo

| Mục | Giá trị |
|---|---|
| Nhánh | `chore/web-bun-migration` (upstream `origin/master` không đụng) |
| HEAD | `fbe6ea0fe7` (D41–D43) + 3 commit Goal 2 (2026-10-22) — **chưa push**; xem `git log -3` |
| Version | `0.2.1-rc.3` (đã bump **toàn bộ** 343 package — xem bẫy §5) |
| Release đã publish | `desktop-v0.2.1-rc.3-linux-x64` + `-linux-arm64`, feed `desktop-feed-linux-{x64,arm64}` **có chữ ký Ed25519** ✓ |
| Lint | `bun run lint` sạch (1 warning có sẵn ở `packages/experimental/client-ui-cot-translation`) |
| Typecheck | `bun run typecheck` 5/5 ✓ |
| Lane CI | `.github/workflows/desktop-linux.yml` xanh ✓ (có thêm bước so shim ảnh với sharp thật) |
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
- **Update trọn luồng** (`update-flow-probe.mjs`): sau D40 probe này **bị hỏng** (manifest schema 1 + không ký) và đã sửa: schemaVersion 2, ký Ed25519 bằng key tạm, truyền `DSH_DESKTOP_UPDATE_PUBLIC_KEY`; giờ `RESULT update-flow=OK` (prompt download/install, `restart`, ảnh được thay).
- **Profile contention**: `desktop-lease-probe.sh` vẫn `OK`.
- **Sharp fallback**: `image-parity-probe.sh` 11/11; khác byte JPEG/GIF/TIFF/resize là đã chấp nhận (D39).
- **Benchmark**: Host cold start 1,98–2,09 s (warm), idle RSS 179–182 MiB; shell 0,55 s → 0,77 s → 4,54 s trên weston headless. `resident-probe.sh`: idle toàn app **RSS 605–607 MiB / PSS 430–432 MiB** (shell 192 + Host 169 + WebKit 167 + network 56).

Còn lại của Goal 2: đưa các probe mới vào CI; bằng chứng GUI thật (CSP/console); crash giữa download/rename; portability matrix (Windows ACL/sandbox/Office path/update driver); đo WebView RSS.

## 3. Việc nên làm tiếp (theo thứ tự giá trị)

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
| **Node strip-only không chịu parameter property** | Các probe boot Host *từ source* phải qua tsx (đã sửa `bridge-probe.sh`, `pty-probe.sh`, `terminal-page-probe.sh`). |

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
```
Phát hành (làm thay đổi trên `fork`): bump version **mọi package** ⇒ tag `desktop-v<version>` ⇒ lane release tự chạy ⇒ feed mới đã ký.

## 8. Tài liệu lane
`.agents/plan/desktop-tauri/PLAN.md` (kế hoạch + bảng finding F01–F13) · `MEMORY.md` (quyết định D1–D39, số đo M1–M25, bẫy B1–B52) · `AGENTS.md` (luật lệ của lane) · `probes/` (probe cũ) — cập nhật cả hai file đầu khi có quyết định mới.
