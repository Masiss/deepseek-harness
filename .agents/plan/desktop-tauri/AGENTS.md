# AGENTS.md — Desktop Tauri migration

Hướng dẫn bắt buộc cho agent làm việc trên nhánh desktop Tauri. Đọc cùng:

- [`PLAN.md`](PLAN.md) — ADR + kế hoạch theo phase + điều kiện nghiệm thu.
- [`MEMORY.md`](MEMORY.md) — quyết định đã chốt, số đo, bẫy đã gặp.
- [`../../../AGENTS.md`](../../../AGENTS.md) — quy tắc chung của repo.
- [`../../../docs/architecture.md`](../../../docs/architecture.md) — kiến trúc harness.

## 1. Sứ mệnh

Chuyển desktop app từ Electron sang **Tauri (Rust)** và giảm kích thước/runtime footprint, **không làm mất gốc** của sản phẩm:

- giữ nguyên plugin ecosystem (Cordis, `dsh plugin`, bundle composition),
- giữ nguyên session format, model-visible transcript, CLI/config,
- giữ khả năng merge từ `deepseek-ai/deepseek-harness`.

## 2. Bất biến — không được vi phạm

| # | Bất biến |
|---|---|
| 1 | Session log format + `SESSION_FORMAT_VERSION` + replay |
| 2 | `SessionEventMap` required-on-read, SDK expected outputs |
| 3 | Model-visible transcript: prompt, tool schema, thứ tự, tool result |
| 4 | Plugin API: tên package, `exports`, Service Definition |
| 5 | Bundle/profile composition, `$DSH_HOME` layout |
| 6 | CLI contract, config schema `cordis.yml` |
| 7 | Token/cookie auth + WebSocket |
| 8 | PTY semantics: resize, signal, console mode |
| 9 | Sandbox semantics: Windows ACL, Landlock |
| 10 | Update policy: mandatory update, task lock, quit inspection |

**Luật vàng:** không sửa expected output, snapshot hay fixture để rewrite pass. Nếu lệch, đó là regression.

## 3. Ba lớp thay đổi

| Lớp | Được phép | Ví dụ |
|---|---|---|
| **L0** | Không sửa | `packages/core/*`, `llm/*`, `session/*`, `bundle/*`, `apps/desktop/src/*` |
| **L1** | Thêm file/package mới | `apps/desktop-tauri/`, `native/dsh-native/`, provider packages |
| **L2** | Glue tối thiểu, ghi vào `MEMORY.md` | `package.json`, profile patch, tài liệu |

**Cấm:** rewrite in-place một implementation upstream. Mỗi lần upstream sửa file đó sẽ thành conflict vĩnh viễn.

## 4. Quy trình thay provider

1. Tạo package provider mới (L1) dưới `packages/`, giữ nguyên tên Service Definition và kiểu TS.
2. Export một Cordis plugin đăng ký đúng `ctx.<key>`.
3. Thêm fork patch YAML (L1) tắt row cũ, mount row mới:
   ```yaml
   - id: subprocess
     disabled: true
   - id: subprocess-rust
     name: '@deepseek-ai/dsh-subprocess-native'
   ```
4. Feature flag, mặc định **tắt**.
5. Chạy parity suite trên cả provider cũ và mới.
6. Chỉ bật mặc định khi xanh trên nền tảng mục tiêu.
7. Không xoá code cũ trước một chu kỳ phát hành.

## 5. Thứ tự nền tảng

```
Linux MVP → Linux production → macOS → Windows
```

- Trong lúc Linux dùng Tauri, Windows/macOS vẫn dùng Electron cũ.
- Crate native dùng `#[cfg(target_os)]` **từ ngày đầu**, kể cả khi chưa build Windows.
- Không hardcode đường dẫn Linux ngoài module platform.

## 6. Gate bắt buộc

Chạy trước khi báo hoàn thành một phase:

```sh
bun run build
bun run build:check
bun run lint
bun run test:web:built
bun run test:snapshot
bun run test:expected
```

Quy ước hiện có của repo:

- `test:docs` còn 1 failure có sẵn (`verify-repository-references`, `python/sdk/uv.lock:113`). Không sửa trong phạm vi này.
- `scripts/release/pack.spec.ts > does not broaden an empty parallel family` fail có sẵn ở HEAD.
- Kiểm tra trước khi push theo `.agents/skills/dsh-pre-push-checks/SKILL.md`.

## 7. Điều kiện dừng

Dừng ngay, không cố, khi gặp:

| Điều kiện | Hành động |
|---|---|
| `dsh web` không boot dưới Bun | Bỏ Bun-compiled, fallback Node |
| Snapshot/expected lệch | Điều tra; không cập nhật fixture |
| Web e2e fail > 0 | Dừng |
| `dsh plugin` không cài/chạy được plugin | Dừng mọi nhánh native rewrite |
| Windows sandbox test fail | Giữ koffi |
| Cài máy sạch không mở được | Dừng phát hành |

## 8. Những điều cấm

- Không dùng Electron cho desktop mới.
- Không sửa `apps/desktop/` (Electron) trừ khi có yêu cầu riêng; nó là cầu tạm cho Windows/macOS.
- Không thêm dependency nếu crate/stdlib làm được.
- Không dùng `as unknown`; tuân thủ rule của repo.
- Không commit binary build vào repo.
- Không chạy `package:desktop*`/`upload:*` khi chưa được yêu cầu.
- Không đổi `packageManager` nếu chưa xử lý `scripts/primary-runtime/prepare.spec.ts:168`.

## 9. Cấu trúc dự kiến

```
apps/desktop-tauri/          # L1 — shell Rust
  src-tauri/                 # Rust: window, tray, menu, updater, sidecar
  src-tauri/tauri.conf.json
native/dsh-native/           # L1 — crate napi-rs, cfg theo OS
packages/.../*-native/       # L1 — provider packages
.agents/plan/desktop-tauri/  # tài liệu này
```

## 10. Định nghĩa "xong"

Một phase chỉ được coi là xong khi:

1. Acceptance criteria của phase trong `PLAN.md` đạt.
2. Gate ở mục 6 chạy pass (trừ failure có sẵn đã ghi).
3. `MEMORY.md` được cập nhật số đo và quyết định mới.
4. Không phát sinh file L2 mới ngoài danh sách đã ghi.
