# Agent instructions — short and evidence-first

## D0 — inspect current product

```text
Dùng frontend-design/QA workflow để audit DSH GPUI thật so với reference/dsh-ui-prototype-v2.html và HAMBURGER_AND_NAVIGATION.md. Đọc runtime-capability-map.md nhưng kiểm lại source/Host hiện tại. Chụp native UI 1366×768 + 1920×1080, liệt kê mismatches + capability gaps kèm file/line. Chưa sửa code.
```

## D1 — fix hamburger/menu parity

```text
Sửa hamburger DSH: mở Application Menu đúng V2 (không phải palette), workspace picker riêng; menu/shortcut chung command registry. Giữ shell 30/34/26, Explorer–Agent–Changes và session ở chân Explorer. Test click/keyboard/Esc/focus + native screenshot; báo Host gaps, không mock success.
```

## D2+ — one feature slice

```text
Triển khai [FEATURE ID] theo DSH_FEATURE_TRACEABILITY.md + spec V2. Tái dùng GPUI/Host có sẵn, không thêm UI chung chung. Verify state/focus/IME/error, screenshot before/after, đối chiếu ZED_ALIGNMENT.md. Báo file, capability, tests và gaps.
```

## Tooling

Use available native desktop screenshot and input tools for GPUI; don't depend on gpui-agent or force an upstream GPUI patch. Browser Playwright is for validating HTML previews only; native screenshots/test journeys are independent acceptance gates.
