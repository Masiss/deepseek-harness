# Bàn giao — lane Desktop Tauri (fork `Masiss/deepseek-harness`)

Cập nhật 2026-10-22. Mục đích: session mới đọc file này là biết đang ở đâu, cái gì đã xong, cái gì nên làm tiếp và bẫy nào đã mắc.

## 1. Trạng thái repo

| Mục | Giá trị |
|---|---|
| Nhánh | `chore/web-bun-migration` (upstream `origin/master` không đụng) |
| HEAD | `ee93ab64e8` — đã **push hết** lên `fork` ✓ |
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

## 3. Việc nên làm tiếp (theo thứ tự giá trị)

### (A) Sửa 7 phát hiện của audit bảo mật — **ưu tiên cao nhất**
Bản đầy đủ: `~/security-audit-skill/deepseek-harness/run-1/` (`REPORT.md`, `FINDINGS-DETAIL.md`, `findings.json`).
Mỗi phát hiện có sẵn đường truyền, điều kiện, cách tái hiện trong sandbox và bản vá đề xuất (trong `findings.json` ⇒ `remediation.code_changes`).

| # | Mức | File cần sửa | Sửa gì |
|---|---|---|---|
| 1 | **high** | `apps/desktop-host/src/components/manifest.ts`, `apps/desktop-host/src/components/office.ts`, `apps/desktop-tauri/scripts/publish-release.ts` | Ký `components.json` bằng **cùng key** của update feed + client verify trước khi tải bất cứ byte nào; chặn scheme không phải https/loopback. **Đây là cái quan trọng nhất**: archive tải về được **thực thi**. |
| 2 | medium | `apps/desktop-host/src/components/tar.ts` (+ `manifest.ts`) | Chặn *khi ghi* luồng giải nén (amplification 31.000×), và giới hạn `bytes` mà manifest khai. |
| 3 | medium | `native/dsh-native/src/image_scan.rs` (+ test) | Đọc byte flags của chunk VP8X bằng `data.get(start)` ⇒ hết panic ⇒ hết SIGABRT. **Đây là code Rust của chính phiên này viết.** |
| 4 | medium | `apps/desktop-tauri/scripts/pack-appimage.sh` (+ `pack-rpm.sh`) | Ghim digest của `appimagetool` (và image `fedora:41` theo digest) trước khi chạy. |
| 5 | low | `apps/desktop-host/src/desktop-bridge.ts` | Gọi `parseShortcutEdit` / `parseShortcutDefinitions` ở cửa vào (Electron đã làm vậy ở `apps/desktop/src/keyboard.ts:89`). |
| 6 | low | `apps/desktop-host/src/update/appimage.ts` | Digest + size-check **chính bản staged** trước khi rename đè lên ảnh đang chạy. |
| 7 | low | `apps/desktop-host/src/tauri.ts` (+ `update/service.ts`) | Nhả admission lock khi cài thất bại (Electron có rollback ở `apps/desktop/src/main.ts:633`). |

Sau khi sửa: chạy lại `bunx vitest run apps/desktop-host`, `cargo test --manifest-path native/dsh-native/Cargo.toml --lib`, `bun run typecheck`, `bun run lint` ✓ và bổ sung **test hồi quy** cho mỗi cái (mỗi phát hiện đều có sẵn cách tái hiện trong sandbox để làm mẫu).

### (B) F14 — bỏ ~34 MB rác trong payload
Cây `dsh/node_modules` mang 21 package `dsh-experimental-*` (32 MB là `sherpa-onnx-linux-x64` — model giọng nói) trong khi profile desktop **không mount** package experimental nào. Lọc dependency closure trong `apps/desktop-tauri/scripts/stage-payload.sh` (đúng cách đã bỏ `sharp`/`koffi`/`node-pty`) ⇒ AppImage 90 MB → ~62 MB.

### (C) Bằng chứng vào CI
- Đưa `entry-parity-probe.sh` vào **lane release** (lane đó *có* cây đóng gói; lane CI thì không) ⇒ mỗi bản phát hành đều chứng minh cả hai entrypoint Host vẫn boot.
- Thêm **gate drift** cho `apps/desktop/`: chặn thay đổi ngoài `package.json` + `desktop-build-paths.mjs` (hai file đã biết).
- **Không** thêm bước build Electron vào CI: fork không sửa phần đóng gói của Electron, và bằng chứng quan trọng hơn (entry Electron boot được) đã có ở probe.

### (D) A9 — diff transcript với model sống
Đường đã có: `live-transcript-probe.sh` (lượt thật + replay + diff). Nếu cần *diff* với bản ghi sẵn của upstream thì phải dùng provider deepseek (cần `DEEPSEEK_API_KEY`); nếu không, diff live-vs-replay (hiện tại) là bằng chứng đủ cho "transcript không đổi".

### (E) F13 — đã **chốt** là khác biệt đã chấp nhận (D39)
Resample của libvips khác `fast_image_resize` (max 6/255 mỗi kênh) ⇒ byte ảnh chuẩn hoá lệch giữa hai carrier ⇒ A1/A9 chỉ đúng với phiên **không** đính kèm ảnh. Đã ghi vào PLAN/MEMORY. Nếu muốn đóng hẳn thì phải port arithmetic của libvips (nặng).

### (F) Cần môi trường ngoài máy này
P3 ký macOS/Windows (chứng thư) · P4 ba target còn lại (runner macOS/Windows) · P5-win32 phần ACL (runner Windows) · P6 brotli/ripgrep (tùy chọn) · welcome/onboarding (đã hoãn).

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
bash apps/desktop-tauri/scripts/update-probe.sh          # feed ký hợp lệ → available; sửa chữ ký → idle
bash apps/desktop-tauri/scripts/desktop-lease-probe.sh   # lease: hai carrier không ghi chung state
bash apps/desktop-tauri/scripts/image-parity-probe.sh    # shim ảnh vs sharp thật
bash apps/desktop-tauri/scripts/live-transcript-probe.sh  # model thật ⇒ replay ⇒ diff 0
bash apps/desktop-tauri/scripts/entry-parity-probe.sh    # cả hai entrypoint Host boot
```
Phát hành (làm thay đổi trên `fork`): bump version **mọi package** ⇒ tag `desktop-v<version>` ⇒ lane release tự chạy ⇒ feed mới đã ký.

## 8. Tài liệu lane
`.agents/plan/desktop-tauri/PLAN.md` (kế hoạch + bảng finding F01–F13) · `MEMORY.md` (quyết định D1–D39, số đo M1–M25, bẫy B1–B52) · `AGENTS.md` (luật lệ của lane) · `probes/` (probe cũ) — cập nhật cả hai file đầu khi có quyết định mới.
