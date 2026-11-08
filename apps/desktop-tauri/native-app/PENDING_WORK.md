# Việc còn lại — Desktop native UI

Cập nhật: 2026-11-08. Mọi slice trong `DSH_Design_System_v1.2/IMPLEMENTATION_PLAN.md` đã được
triển khai trên nhánh làm việc (commit local, chưa push khi tài liệu này được viết):
A1, A2, A3, B, C, D, G0, G1, G2, G3, G4a, G4b, P1, P2.

Trạng thái kiểm chứng tại thời điểm đó:

- `cargo test --release` (native-app): 361 passed, 0 failed, 1 ignored.
- `cargo test` (client-core): 114 + 8 + 11 passed.
- `vitest packages/api/workspace-controller`: 227 passed; `bun run typecheck`: 5/5.
- Screenshot binary release dưới weston headless + Xwayland:
  `/tmp/dsh-final-shots/{light,dark}-{1366x768,1440x900,1920x1080}.png` (chỉ trạng thái
  disconnected/empty, chưa có Host session thật).

## Việc còn lại

| # | Việc | Ước lượng | Phụ thuộc / ghi chú |
|---|---|---|---|
| 1 | Screenshot trạng thái connected (Session thật, Changes có thay đổi, History, approval) | 0.5–1 ngày | Cần `DEEPSEEK_API_KEY`, project và Host ổn định; nên script hoá một session ngắn rồi chụp |
| 2 | Editor màu từng role trong Preferences | 1.5–2.5 ngày | 18 role × 2 scheme + accents, preview, validate, undo. Hiện đã có Edit JSON + Duplicate nên không chặn sử dụng |
| 3 | `git/show` cho merge commit (diff so với first parent) | 0.25–0.5 ngày | Host: thêm `--diff-merges=first-parent`, test, ghi chú UI |
| 4 | `+N −M` cho từng commit trong History | 0.5–1 ngày | `git log --numstat` + cap + client + UI |
| 5 | Parent navigation xuyên trang (lookup commit theo oid) | 0.5 ngày | Thêm `git/log { commit }` hoặc `git/show` parent |
| 6 | Branch create/delete/rename | 1–1.5 ngày | 3 verb mới; cần guard: không xoá branch chưa merge, không rename khi dirty |
| 7 | Approval "Delegate" (nhường quyết định cho answerer khác trên Host) | 0.25–0.5 ngày sau khi chốt | Hiện đã bỏ theo plan (chỉ Allow once/Deny). Nếu giữ, đặt trong menu **More…** và nói rõ có thể fail-closed |
| 8 | 4 finding `cramped-padding` của detector trên tab strip V2 | 0–0.5 ngày | Đang là pattern cố ý của V2; khuyến nghị để nguyên |

Tổng: khoảng 4.5–7 ngày làm việc, ~3–5 ngày lịch nếu chạy lane tuần tự và merge/review như các
slice trước.

## Cần chốt trước khi làm

1. **Editor role** (mục 2) có cần trong app hay JSON editor là đủ?
2. **Delegate** (mục 7) giữ lại hay bỏ hẳn? Nếu giữ, môi trường nào có answerer thứ hai (TUI,
   policy plugin) để tính năng có nghĩa?
3. **Branch create/delete/rename** (mục 6) có nằm trong phạm vi sản phẩm không, hay chỉ cần
   checkout/merge/rebase như hiện tại?

## Giới hạn đã biết (không phải việc mới)

- `git/show` trả patch rỗng cho merge commit; `git/log` không trả thống kê dòng theo commit.
- Parent link trong History chỉ bấm được khi commit cha nằm trong các trang đã tải.
- **Reviewed** key theo `git/status` (kind, side flags, stats), chưa theo content hash; sửa file mà
  thống kê không đổi thì dấu Reviewed không tự mất.
- Ref chỉ có tên: branch local chưa load và tên có `/` được hiển thị như remote; `tag:` là chuẩn.
- Lane màu History dùng 4 token (`accent`, `success`, `warn`, `error`); không có bảng màu riêng.
- Pending approval bị bỏ khỏi UI ngay sau khi dispatch (hành vi cũ, chưa sửa thành chờ ACK).
- Screenshot/verification trên máy này: GPUI 0.2.2 panic nếu Wayland compositor không có seat, nên
  dùng weston headless + `--xwayland`, chụp bằng `magick import -window <id>`; input không inject
  được nên chỉ chụp được trạng thái tĩnh.

## Cách chạy lại kiểm chứng

```sh
cd apps/desktop-tauri/native-app
CARGO_BUILD_JOBS=6 cargo test --release
CARGO_BUILD_JOBS=6 cargo build --release
```

Ghi chú môi trường: mỗi worktree `target/` chiếm 2–8 GB; chỉ nên chạy một lane build tại một thời
điểm, và xoá `target/debug` nếu chỉ dùng `--release`. Lane dài nên đặt `CARGO_BUILD_JOBS=6` để tránh
OOM khi máy còn mở trình duyệt.

## Tài liệu liên quan

- `DSH_Design_System_v1.2/IMPLEMENTATION_PLAN.md` — kế hoạch và trạng thái từng slice.
- `DSH_Design_System_v1.2/GIT_SURFACE_V1.3.md` — inventory surface Git và bảng verb Host.
- `DSH_Design_System_v1.2/prototype-git-v1.3.html` — prototype tham chiếu (dữ liệu mẫu).
- `USER_GUIDE.md`, `USER_FLOWS.md` — hướng dẫn và luồng người dùng hiện tại.
