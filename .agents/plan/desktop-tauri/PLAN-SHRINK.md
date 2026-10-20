# PLAN-SHRINK — Cắt nhỏ phần mềm (kế hoạch nâng cấp, chưa implement)

Kế hoạch này **nằm ngoài** [`PLAN.md`](PLAN.md) (kế hoạch chính của lane Tauri). Nó gom các cơ hội **không có trong plan chính** nhưng đã lộ ra trong lúc làm P5/P6/A3: những phần payload/size có thể bỏ đi mà không mất gốc sản phẩm.

- Trạng thái: kế hoạch, chưa implement.
- Ngày: 2026-10-20.
- Quy tắc làm việc: [`AGENTS.md`](AGENTS.md). Số đo và bẫy: [`MEMORY.md`](MEMORY.md).
- Thứ tự làm: **quay lại [`PLAN.md`](PLAN.md) và đóng nó trước**; tài liệu này chỉ mở sau khi plan chính xanh.

---

# 1. Bối cảnh

Sau A3, artifact phát hành đã dưới ngân sách:

| Artifact | Size | Ngân sách |
|---|---|---|
| AppImage | 86,7 MiB | ≤ 105 MiB ✓ |
| .deb | 66,9 MiB | ≤ 105 MiB ✓ |
| .rpm | 72,9 MiB | ≤ 105 MiB ✓ |
| Cây cài đặt | 337 MiB | ≤ 400 MiB ✓ (A3′) |

Nhưng payload vẫn mang những phần **không phục vụ profile Desktop** và những phần **chỉ dùng cho một route/tính năng**. Đây là danh sách cắt được, không phải danh sách "nice to have".

## 1.1 Bố cục payload sau A3 (đo trên cây `linux-x64`)

| Nhóm | Size | Ghi chú |
|---|---|---|
| `@deepseek-ai/*` (closure của gói `dsh`) | 118 MB | trong đó có 32 MB gói *experimental* không profile nào compose |
| `sherpa-onnx-linux-x64` | 32 MB | onnxruntime + model của speech-to-text *experimental* |
| `@modelcontextprotocol` | 16 MB | SDK MCP (client + server) |
| `openai` | 11 MB | SDK provider |
| `@opentelemetry` | 8,9 MB | telemetry (mặc định tắt) |
| `@google` | 5,8 MB | SDK provider |
| `@anthropic-ai` | 5,6 MB | SDK provider |
| `@vscode/ripgrep*` | 5,5 MB | `rg` cho công cụ tìm kiếm |
| `fontkit` | 2,9 MB | phụ thuộc của `sharp` — đã bị provider Rust thay ⇒ chết |
| JS/CSS của client | 96,9 MB | phục vụ cho trình duyệt |
| `runtime/` (Node 103,5 + pnpm 15) | 127 MB | trần của bài toán runtime |

---

# 2. Mục tiêu

| # | Mục tiêu | Đo bằng | Ước lượng |
|---|---|---|---|
| S1 | Cây cài đặt ≤ 270 MiB | `size-gate.sh` (mục tiêu mới) | 1–2 pw |
| S2 | AppImage ≤ 80 MiB | `size-gate.sh` | 1–2 pw (phụ thuộc S1 + S3) |
| S3 | Client assets phục vụ dạng nén | so kích thước `.br` với gốc | 0,5–1 pw |
| S4 | Một nguồn duy nhất cho chuỗi của shell | generator Rust từ `locale.ts` | 0,5 pw |
| S5 | Không còn bản bootstrap trùng giữa Electron/Tauri | `git diff` không còn `tauri.ts` duplicate | 1 pw (PR upstream) |

Nguyên tắc giữ nguyên: **không bỏ gốc sản phẩm** — mọi thứ bị cắt phải là thứ profile Desktop không dùng, hoặc phải cài lại được theo một cơ chế đã có (component on-demand, plugin cài lúc chạy, hoặc phụ thuộc của hệ điều hành). Không đổi public API, không sửa `packages/` upstream.

---

# 3. Hạng mục

## S1-A — Thu hẹp core set của Desktop theo profile (không cần cơ chế component mới)

**Vì sao.** Core set của lane Tauri là **closure của gói `dsh`** nên mọi gói *experimental* đều được cài, kể cả gói profile Desktop không compose. Đây là phần cắt rẻ nhất: không đụng cơ chế lúc chạy, chỉ đổi cách tính tập package.

| Gói | Size | Lý do cắt |
|---|---|---|
| `dsh-experimental-inspector` (+ `-profile`) | 25 MB | không package nào trong sản phẩm khai báo; là bundle devtools riêng |
| `sherpa-onnx-linux-x64` | 32 MB | chỉ `dsh-experimental-speech-to-text-sensevoice` dùng |
| `dsh-experimental-client-ui-cot-translation` | 6,7 MB | experimental, không compose trong profile web |
| `fontkit` | 2,9 MB | phụ thuộc của `sharp`; provider Rust đã thay `sharp` |
| các `dsh-experimental-*` còn lại | ~1 MB | experimental |

**Cách làm.** `readDesktopCorePackageSet` (lane Tauri, L1) tính closure từ **profile thực sự compose** thay vì từ gói `dsh`; gói *experimental* chỉ vào khi profile khai báo.

**Rủi ro.** Profile động cập nhật có thể cần gói mà core set không có → phải khai báo phụ thuộc tường minh, và lỗi phải nói rõ tên gói.

**Acceptance.** `size-gate.sh` báo cây cài đặt ≤ 270 MiB; profile Desktop vẫn boot, `dsh plugin` vẫn cài+chạy được; không gói experimental nào trong payload.

## S1-B — Provider SDK theo nhu cầu

**Vì sao.** `openai` 11 MB + `@anthropic-ai` 5,6 MB + `@google` 5,8 MB + `@smithy` 3,3 MB ≈ **26 MB**; người dùng chỉ cần một route. Cơ chế **đã có**: plugin manager cài được package lúc chạy (`dsh plugin`).

**Cách làm.** Biến SDK của provider thành **optional dependency** của plugin `llm-pi-ai`; khi người dùng chọn route chưa có SDK, app cài nó qua plugin manager (cùng luồng với `dsh plugin`, có progress + lỗi rõ ràng).

**Rủi ro.** Route đổi lúc chạy cần mạng; phải chịu được "chưa cài" như một trạng thái hợp lệ (không phải crash).

**Acceptance.** Payload không còn SDK của route chưa dùng; đổi route sang provider chưa cài thì app cài được rồi chạy.

## S1-C — `ripgrep` on-demand (5,5 MB)

**Vì sao.** `rg` chỉ phục vụ công cụ tìm kiếm; AppImage đóng gói nó cho mọi người.

**Hai lựa chọn.** ① Thành component on-demand (như Office) ② Bỏ khỏi AppImage và dùng `rg` của hệ thống, khai báo dependency cho `.deb`/`.rpm`.

**Rủi ro.** ② làm tính năng tìm kiếm phụ thuộc hệ điều hành — cần thông báo rõ khi thiếu.

**Acceptance.** `rg` không còn trong AppImage; công cụ tìm kiếm vẫn chạy (qua component hoặc qua hệ thống), và báo lỗi dễ hiểu khi thiếu cả hai.

## S2 — Brotli cho client assets (~38 MB trong AppImage)

**Vì sao.** 96,9 MB JS/CSS; trình duyệt tải bản gốc. Nén `.br` cạnh file gốc và cho server phục vụ bản nén khi client chấp nhận.

**Cách làm.** Server tĩnh là **upstream** (không sửa được) ⇒ **Host** của fork thêm middleware: nếu request có `Accept-Encoding: br` và file `.br` tồn tại thì phục vụ nó kèm `Content-Encoding: br`. Bước build sinh `.br` cạnh asset.

**Rủi ro.** Phải giữ `Content-Type`/`Content-Length` đúng; fallback khi client không nhận br; `.br` phải được sinh ở đúng chỗ đóng gói (deb/rpm/AppImage dùng chung cây staged ✓).

**Acceptance.** AppImage nhỏ hơn ~38 MB; client vẫn tải và render đúng; không có request nào 404 vì `.br`.

## S3 — Một nguồn chuỗi cho shell (locale) — *thuộc [`PLAN.md`](PLAN.md) P1, không làm lại ở đây*

**Vì sao.** `apps/desktop/src/locale.ts` là nguồn duy nhất; chép tay sang Rust tạo hai nhà cho cùng chuỗi (B1 của audit).

**Cách làm.** Generator đọc `locale.ts` lúc build và xuất Rust; chuỗi của shell trở thành locale-owned ✓ (đóng được mục P1 "Bản địa hoá copy của shell").

**Acceptance.** Không còn chuỗi viết tay trong Rust; đổi `locale.ts` ⇒ Rust sinh lại; kiểm tra không lệch chuỗi nào.

## S4 — Đưa bootstrap chung của Tauri/Electron lên upstream

**Vì sao.** `apps/desktop-host/src/tauri.ts` cố ý duplicate `index.ts` của Electron để không refactor upstream (F10 của audit) ⇒ fork không tự nhận bugfix behavior.

**Cách làm.** PR lên upstream tách phần bootstrap dùng chung; lane Tauri dùng lại phần đó. Trong lúc chờ PR, giữ bản trùng **có ghi chú đồng bộ** (đã có ✓).

**Rủi ro.** PR upstream có thể bị trì hoãn ⇒ giữ bản trùng cho tới khi merge.

**Acceptance.** `tauri.ts` không còn là bản trùng của `index.ts`; hai lane chạy cùng một bootstrap.

---

# 4. Thứ tự làm (sau khi `PLAN.md` xanh)

| Bước | Hạng mục | Cỡ | Ghi chú |
|---|---|---|---|
| 1 | S1-A (core set) | 1–2 pw | cắt 67 MB, không cần cơ chế mới ⇒ làm trước |
| 2 | S2 (brotli) | 0,5–1 pw | đo lại AppImage sau S1-A |
| 3 | S1-C (ripgrep) | 0,5 pw | chọn component hay hệ thống |
| 4 | ~~S3 (locale)~~ | — | thuộc plan chính (P1) |
| 5 | S1-B (provider SDK) | 1–2 pw | cần cơ chế "cài lúc chạy" cho SDK |
| 6 | S4 (PR upstream) | 1 pw | phụ thuộc upstream |

Đo sau mỗi bước bằng `size-gate.sh` và `apps/desktop-tauri/scripts/measure-linux-tree.sh` ✓; artifact phát hành phải vẫn boot + probe xanh ✓.

---

# 5. Ngoài phạm vi

- Đổi runtime Node (Bun đã bị bác ở P0; Node SEA không giảm size).
- Bỏ `pnpm` khỏi payload (cần cho `dsh plugin`; đổi backend cài plugin là việc riêng).
- Cắt `@modelcontextprotocol`/`@opentelemetry` (chưa rõ phạm vi sử dụng thật của profile ⇒ cần đo trước).
- Bỏ `@deepseek-ai/*` nào khỏi payload mà không thuộc profile (S1-A đã xử lý phần lớn).
- Bất kỳ thay đổi nào sửa `packages/` upstream.
