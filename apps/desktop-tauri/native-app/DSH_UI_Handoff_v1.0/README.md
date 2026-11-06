# DSH UI Handoff v1.0

Bộ tài liệu chuyển giao cho giao diện Desktop native: đặc tả hành vi, audit khả năng runtime, backlog triển khai và một bản HTML làm chuẩn thị giác.

## Nội dung

| File | Vai trò | Tính chuẩn |
|---|---|---|
| [`DSH_UI_Functional_Spec_v1.0.md`](DSH_UI_Functional_Spec_v1.0.md) | Hợp đồng hành vi: ID chức năng, acceptance criteria, roadmap R0–R5 | Chuẩn cho *hành vi* mong muốn |
| [`runtime-capability-map.md`](runtime-capability-map.md) | Đối chiếu từng yêu cầu với Host RPC và shell gpui hiện có | Chuẩn cho *trạng thái triển khai* (có ngày audit) |
| [`DSH_UI_Implementation_Backlog_v1.0.md`](DSH_UI_Implementation_Backlog_v1.0.md) | Thứ tự thực hiện, gate nghiệm thu, format báo cáo | Chuẩn cho *thứ tự công việc* |
| [`dsh-ui-prototype-v2.html`](dsh-ui-prototype-v2.html) | Bản HTML tự chứa: layout, token màu, panel, trạng thái | Chuẩn cho *thị giác* |

## Quan hệ với phần còn lại của `native-app`

- `../src/` — ứng dụng ship thật: **Rust + gpui** (`src/gpui_main.rs`, `src/shell/`). `src/main.rs` là shell egui đã nghỉ, chỉ build với feature `legacy-egui`.
- `../prototype/` — prototype tương tác cũ (`index.html` + `docs/`) mô tả giao diện thời egui; vẫn dùng được để xem luồng, nhưng không phải chuẩn thị giác của bản chuyển giao này.
- `DSH_UI_Handoff_v1.0/` — chuẩn thị giác và hành vi cho lần port sang gpui.
- `../USER_GUIDE.md` và `../USER_FLOWS.md` — mô tả ứng dụng đang chạy; khi tài liệu này và ứng dụng lệch nhau, ứng dụng là sự thật và tài liệu này là đích đến.

Khi một hành vi trong spec chưa có capability Host, ghi `NOT_SUPPORTED` kèm fallback trung thực. Không dựng handler giả lập trạng thái thành công.

## Cách dùng

1. Đọc `runtime-capability-map.md` trước để biết cái gì nối được ngay, cái gì cần bổ sung Host.
2. Lấy task từ backlog, mỗi lần một lát cắt dọc (T01, T02, …).
3. Trả báo cáo theo mục 3 của backlog: file đã đổi, capability đã dùng, `NOT_SUPPORTED` còn lại, test đã chạy, rủi ro.

## Bảo trì

Ngày audit ghi trong `runtime-capability-map.md`; sau khi thay đổi Host RPC hoặc shell gpui, cập nhật lại bảng tương ứng và ngày đó. Bản HTML V2 là bản đóng băng cho visual regression, không chỉnh song song với code.
