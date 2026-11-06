# Bàn giao — lane Desktop Tauri (fork `Masiss/deepseek-harness`)

Cập nhật 2026-11-06. Mục đích: session mới đọc file này là biết đang ở đâu, cái gì đã xong, cái gì nên làm tiếp và bẫy nào đã mắc.

> **Đọc theo mốc thời gian.** Các mục ghi ngày ≤ 2026-11-01 mô tả shell **egui** mà ngày 2026-11-06 đã bị xoá (xem mục cuối). Chúng được giữ làm lịch sử quyết định và bẫy, không phải trạng thái hiện tại.

## Native gpui port — 2026-10-31

Native GUI của lane chạy trên **`gpui` 0.2.2** (GPU-first renderer qua wgpu) với **một bin duy nhất**: `dsh-desktop-native-app`. Shell egui cũ (`src/main.rs`, `ui_harness.rs`, `settings_panel.rs`, `transcript_view.rs`, feature `legacy-egui`, `eframe`, snapshot PNG) đã **bị xoá ngày 2026-11-06**; xem mục cuối file. Module chia theo vai (client-core protocol, host, explorer, session_list, question_card, transcript, terminal_ansi, fonts, ui_preferences, layout) giữ nguyên contracts; presentation layer (`shell/`) là gpui.

Gate hiện tại (sau khi xoá legacy): `cargo test --bin dsh-desktop-native-app` = **179 pass / 0 fail**; `cargo build` xanh (đã sửa 4 test `host.rs` đỏ sẵn: gpui kéo `serde_json/preserve_order` vào làm đổi thứ tự key JSON, còn fixture so sánh chuỗi byte; giờ so từng field nên không phụ thuộc thứ tự). 48 test mới phủ preferences chord/parse, auto-connect, office/image path, text-delta budget, model picker/probe/catalog, settings error, theme metrics/blend, terminal log budget, layout bounds, `contains_cjk` và tiến trình khởi động Host.

Startup đo thật (debug, cây packaged, Home cô lập): **host=ready 2,5–3,7 s**, `auth=ok` thêm **~0,13–0,17 s**; riêng `desktop-host` packaged tới listen **1,2–1,6 s warm**. Shell in `dsh: native host=ready in <ms>` / `auth=ok in <ms>` để đo độ trễ thay vì đoán, và trong lúc chờ Host nó hiện `Starting Host sidecar… <n>s`.

Còn lại: runtime gpui cần session Wayland **thật** (headless Weston expose no seat → panic ở `gpui linux wayland client seat`) để xác nhận boot/auth/streams; headless gpui test-support cho shell mới chưa có suite riêng; visual/IME/physical-window review và RSS mới chưa đo; release build + doc gates + pre-push checks chưa chạy lại sau port.

## Native UI rollout — 2026-10-26

`native-app/src/main.rs` áp dụng các view prototype vào shell egui: Plugins và Skills có trạng thái Host, filter/chọn skill; Marketplace search/install, Goals lifecycle và Document preview giữ các RPC đang có. Dock dưới bên phải chuyển Terminal, Jobs, Schedules, Deliverables thành bốn icon accessible có tooltip; chỉ một utility panel mở, mỗi panel có nút đóng, Jobs/Schedules có Refresh và Deliverables mở file preview.

Trước điều chỉnh rail bên dưới, `cargo test --locked --offline --manifest-path apps/desktop-tauri/native-app/Cargo.toml` đạt **153 pass / 3 ignored**. AccessKit harness kiểm tra toggle/close dock, plugin enablement, skill filter/selection, goal create/pause/resume/complete, schedule listing/deletion, job output selection, và deliverable preview. Packaged `native-journey-probe.sh panels` (4 GiB), `model-default`, `update-ui`, AppImage probe, và `size-gate.sh linux-x64` đều pass.

AppImage được cài nguyên tử tại `/home/masis/.local/bin/deepseek-harness` cho owner review: **87,833,080 B**, SHA-256 `dd4c236f4965f58e5f7e81cee5898eb8457c63f080d590bc5da3c223b3d74256`. Đây là build cục bộ; không publish. Chưa mở cửa sổ trong phiên này nên manual acceptance vẫn thuộc owner; tests/probes không thay review tay.

## Headless UI audit — 2026-10-26

`native-app/src/ui_harness/audit.rs` chạy như một `#[test]` integration-style trên `Harness::builder().build_ui_state`, gọi đúng `DesktopApp::tick` + `render`. Bảy scenario (default, many-sessions, long-transcript, approval, explorer-terminal, CJK/Vietnamese, host-offline) nhân nhiều viewport/scale thành **20 run**; mỗi run đọc AccessKit `bounding_box`/`role`/`is_disabled`/`supports_action(Click)` và epaint galley (`elided`, `rect` so `clip_rect`) để kiểm tra bounds/min-size, overlap không hợp lệ, clipping và elision, rồi thực hiện tương tác thật: Reject approval, đóng/mở lại Terminal, gõ CJK/Vietnamese vào Composer.

Kết quả: **LAYOUT PASS / INTERACTION PASS / VISUAL REVIEW PENDING** (0 layout failure, 0 interaction failure). Report máy đọc được ở `target/dsh-kittest/ui-audit.json` (`schemaVersion: 1`, `layout`/`interaction`/`visual`, `runs[]`); ảnh review theo scenario ở viewport nhỏ nhất và lớn nhất nằm trong `target/dsh-kittest/audit/`.

Hai lỗi có bằng chứng từ dump đã sửa: footer sidebar Sessions tràn khỏi viewport (chuyển thành `egui::Panel::bottom("sessions-footer")` và để `session_list` dùng trọn `available_height`) và approval card đè lên composer (`attention_reserved_height` + `estimate_approval_height`, truyền qua `TranscriptViewState`). Golden screenshot chỉ cập nhật **một lần** sau khi chép `.diff.png`/`.new.png` cũ sang `target/dsh-kittest/review/`; ảnh review đó là thứ owner dùng để duyệt thay đổi thị giác, không phải baseline tự làm xanh test. Clippy về 0 warning, gồm ba cảnh báo có sẵn ở `explorer.rs` và vùng `session_title`.

## Visual review follow-up — 2026-10-26

Ảnh GUI owner gửi cho thấy các nhãn rail như “Sessions”, “Explorer” và “Settings” bị bẻ giữa từ. Rail 62 pt cùng nút tối thiểu 50 pt không giữ đủ chỗ cho nhãn ở scale thực tế. Rail tăng lên 82 pt, nút lên 68 pt; audit đọc số hàng galley và báo `nav-label-wrap` nếu nhãn bị tách dòng. Ma trận vẫn đạt **20 run, 0 layout failure, 0 interaction failure**; ảnh mới ở `.artifacts/ui-audit-review/audit/` cho thấy nhãn một dòng.

Không cập nhật golden. Lần chạy đầy đủ sau sửa đạt **147 pass / 6 snapshot failures / 3 ignored**; sáu ảnh `.diff.png`/`.new.png` nằm trong `.artifacts/ui-audit-review/nav-rail-change/` để owner duyệt. `cargo clippy --locked --offline --bin dsh-desktop-native-app --all-targets -- -D warnings` pass. AppImage native được dựng lại; `appimage-probe.sh` và `size-gate.sh linux-x64` pass. **Visual review và chấp thuận golden vẫn pending**.

## Host startup connection — 2026-10-26

The native shell now authenticates with the bundled Host automatically after its ready frame. `DSH_DESKTOP_AUTO_CONNECT=0` disables the initial attempt for troubleshooting; provider configuration is not required for Host authentication. An AccessKit harness test verifies that the default path issues the real `/connect` handshake and surfaces a rejected response. `native-journey-probe.sh terminal` also passes under isolated Weston with no auto-connect variable set: Host ready, auth succeeds, and the terminal journey completes. Full native tests report **148 pass / 6 snapshot mismatches / 3 ignored**; the mismatches are the pending nav-rail golden review above and were not rewritten. Clippy, AppImage probe, and size gate pass. The local AppImage is installed at `~/.local/bin/deepseek-harness`; physical-window review remains pending.

## GUI launcher name — 2026-10-26

The Local Codex runtime registry includes a `deepseek-harness` agent whose executable is `dsh`; its availability scan runs `dsh --version`. The user’s `~/.local/bin/dsh` was the Desktop AppImage, whose native executable ignores CLI arguments and opens the GUI, so each version probe briefly launched the window and Host. The process watcher captured two launches with `--version`, parented to the Local Codex daemon, followed by process exit. The AppImage was renamed to `~/.local/bin/deepseek-harness`, and `~/.local/share/applications/dsh.desktop` now points to that name. The AppImage bytes and SHA-256 are unchanged. `dsh` is currently not installed as a CLI on PATH; install a real CLI before enabling the Local Codex agent.

## Packaged native-panel evidence — 2026-10-25

`native-journey-probe.sh panels` chạy trong `isolated-run.sh` với giới hạn RAM 4 GiB: cài Office component vào Home tạm, cấu hình index và npm registry loopback trong profile tạm, rồi dùng `egui_kittest` AccessKit trên transport/cookie/WebSocket nối packaged Host. Test click Marketplace Search/Install, đọc lại bundle đã cài qua Host, preview `probe.docx` bằng Office converter thật + `pdftoppm`, kiểm tra `missing.docx` lỗi nhưng giữ preview trước đó, tạo/attach PTY qua panel, chạy TUI trong workspace và xác nhận TUI nhận bracketed paste + SGR mouse click. Test cũng ghi `llm-deepseek-account.modelsMaxAgeMs` qua Settings UI, đọc revision/giá trị mới từ Host, rồi dừng Host và xác nhận Retry khởi động lại, xác thực và nạp lại Session. Đây là L2 Host đóng gói + renderer, không phải cửa sổ OS. Giới hạn 2 GiB làm Host thoát 137 khi dựng Office; lượt 4 GiB pass.

`native-journey-probe.sh model-default` dùng grant giả và server `/models` chỉ nghe loopback trong Home/profile cô lập; picker khởi tạo profile default rồi đọc lại catalog từ packaged Host. `native-journey-probe.sh update-ui` dùng fake AppImage executable và feed loopback ký bằng Ed25519 key tạm. AccessKit click Settings → Check for updates → Download → Install and restart; test xác nhận image bytes được thay, file còn executable, Host gửi restart và fake image chạy sau khi test process thoát. Không dùng key production, AppImage phát hành, profile thật, Hyprland hay cửa sổ vật lý. Native harness hiện có **34 test thường + 3 test ignored**.

Lượt kiểm tra trong phiên này: native-app **141 pass / 3 ignored** (`cargo test`, snapshots gồm shell 3 viewport và Explorer); client-core **97 + 8 + 11 pass**; `native-protocol-probe` pass; packaged `panels` pass ở 4 GiB; packaged `model-default` và `update-ui` pass; `bun run build`, `build:check`, `lint`, `test:docs` (21/21), `appimage-probe.sh`, `size-gate.sh linux-x64`, và `git diff --check` pass. `doc-sync` có 40 pass, 3 lỗi catalog đã biết (Cordis API, config catalog, event graphs). `test:expected` có 107 pass / 15 fail trên 7 file; `test:snapshot` có 38 pass / 173 fail / 2 skipped trên 4 file. Các transcript mismatch chứa thêm `system-reminder` với nội dung AGENTS/skill catalog và sequence id tăng; test DSH badge còn lỗi vì mock Agent không hỗ trợ Inbox mutation. Không ghi lại expected/snapshot. Dừng các gate tiếp theo theo quy tắc lane.

Packaged panel journey xác nhận Session search được gửi tới Host; profile phát hành giữ `session-query-sqlite.openAt: never`, nên UI báo đúng search-disabled policy. Nút Refresh sau đó đọc lại Session inventory thành công. Kittest search dùng nhãn AccessKit và chọn được kết quả fixture. Không bật full-text search trong profile mặc định.

Native AppImage mới dựng và cài để owner thử (chưa có manual GUI acceptance): `/home/masis/deepseek-harness/apps/desktop/.desktop-build/targets/linux-x64/appimage/DeepSeek-Harness-native-0.2.1-rc.4-x86_64.AppImage` → `/home/masis/.local/bin/dsh`, **87,747,064 B**, SHA-256 `2448babb2339f158449cbce5b19347e1f0c97c187471d1f9644cd5e7ca75e898`. Thay thế bằng rename nguyên tử sau khi xác minh checksum; mode `0755`. `appimage-probe.sh` xác nhận seed Home, Host boot từ extraction chỉ đọc và bridge idle; `size-gate.sh linux-x64` pass (download 87,747,064 / 110,100,480 B, installed tree 289,930,082 / 419,430,400 B). Instance cũ vẫn chạy từ mount trong `/tmp`; không dừng instance này, nên cần đóng nó rồi mở lại để chạy bản native mới. Đây là build cục bộ, không phải release-qualified artifact: expected/snapshot gates còn đỏ, physical-window review và feed production chưa được kiểm tra. Không publish.

Goal 3 còn mở: kiểm tra visual/manual trên cửa sổ desktop vẫn thuộc owner; Settings conflict chỉ có fixture coverage; update qua feed phát hành và release/AppImage qualification chưa xác minh. Native test/probe thành công không thay L3 acceptance hay điều kiện release của Goal 3.

## Native release and Host integration — working tree 2026-10-24

### Appearance editor: role colors and font families — 2026-10-24

Preferences moved to schema 2. A version-1 file is still read: every new field defaults, the loaded version is raised, and the stored choices survive (test `reads_a_version_one_file_and_keeps_its_choices`). New fields are a `customTheme` with a palette per color scheme, `fonts`, and `radius`.

The palette uses role names rather than widget fields — background, surface, text, muted, accent, selection, border, warn, error — the same vocabulary as the OpenDesign "Neutral Modern" token model, and the Settings editor exposes a swatch plus hex field per role with a reset button. Defaults follow that model (cobalt accent, near-black background instead of pure black). The Settings page now groups Appearance (theme, scale, density, corner radius), Colors (pack and per-role editor), Fonts, shortcuts, Sessions width, the JSON draft, and the actions into labeled sections instead of one flat list.

Fonts are real choices, not a fixed fallback. `fonts.rs` scans `fc-list` on a background thread, keeps single-face `.ttf`/`.otf` files (font collections cannot be loaded), resolves a configured family exactly then case-insensitively then by substring, and builds the egui chains with the chosen family first, the bundled faces next, and the system fallbacks last, so uncovered scripts still render. A configured family that disappeared is reported in the editor and in the log instead of failing startup.

Verified on the physical desktop with an isolated Home: a green custom palette painted the panels (`#0F1A12` sampled from the screenshot), radius 4 flattened the controls, `Noto Sans` shaped the interface, `JetBrainsMono Nerd Font Mono` shaped code, and the preview line rendered `việt`. Native tests are 95, clippy keeps its 9 pre-existing warnings, and the AppImage was rebuilt (87,652,856 B) and reinstalled.

Two probe lessons: `fcitx5` composing ASCII while testing fonts rewrites words (`fonttest` → `fontòtét`), so deactivate the IME for Latin checks and re-activate it afterwards; and a keystroke that activates a focused egui button can open the portal file dialog, which blocks the UI thread long enough for Hyprland to show "Application Not Responding". That path is pre-existing (the file dialog is called synchronously), not a regression from the appearance work, but the next GUI probe should check it.

### Physical-desktop pass (L3) and four defects it found — 2026-10-24

Every earlier GUI check ran under headless Weston. This pass drove the installed AppImage (`~/.local/bin/dsh`, 87,607,800 B) on the real Hyprland session with an isolated `DSH_HOME`, injecting keys with `wtype` and capturing windows with `grim`. It found four real defects, all fixed in the working tree:

- **Composer discarded every keystroke.** The message `TextEdit` used an auto id that shifts as transcript rows grow, so the focused id pointed at a widget that no longer existed; egui reported focus while the typed text was dropped. Fixed with `.id_salt("composer")`; `composer_identity::composer_focus_survives_a_changing_transcript` fails with `the composer lost focus` when the salt is removed.
- **`session/follow` failed on a fresh Home** with `gateway/input-invalid: wire field "request" failed boundary validation` because the client sent `assistantStream: false` and the Host schema is `assistantStream?: true`. The field is now a `bool` that is omitted when false.
- **Vietnamese text rendered as missing-glyph boxes.** egui's bundled faces have no precomposed Vietnamese letters. `install_font_fallbacks` appends the first available system face (Liberation, Noto, DejaVu) to the proportional and monospace chains.
- **No keyboard path to a first message.** `Ctrl+N` now creates a Session and focuses the composer, which also made real typing verifiable without pointer injection.

Verified on the physical session: boot/auth/streams, session creation, typing with a visible caret, clipboard paste (`wl-copy` + `Ctrl+V`), Vietnamese IME through fcitx5 Bamboo (`vieejt` → `việt`), Solarized and high-contrast packs, and a rebound Settings chord (`Ctrl+Shift+S` opens Settings while `Ctrl+Comma` does nothing). Killing the Host pid produced a clear disconnected state without a crash, kept the session list, and hid the composer. RSS/PSS on the shipped image: 241/228 MiB after start, 253/238 connected, 260/240 with a Session and Settings open, 261/245 with composer text — under the 300 MiB gate on a real GPU.

Not verified here: pointer-driven controls (Retry after Host death, terminal panel create/select, a signed update confirmation) and a full-screen TUI inside the terminal panel, because this compositor exposes no pointer injection (`ydotool`/`wlrctl` absent; `hyprctl` only moves windows). Read screenshots cropped and upscaled; the 1x thumbnails misrender small text badly enough to invent bugs.

### Terminal screen model and appearance packs — 2026-10-24

`terminal_ansi.rs` keeps a bounded screen (5,000 rows) instead of assembling lines: cursor moves (`H`/`f`, `A`/`B`/`C`/`D`, `G`, `d`), save/restore (`s`/`u`), erase-in-line/display (`K`/`J`), wrapping at the terminal's column count, and scrolling all land where a full-screen program expects. The panel re-parses when the stream or the column count changes. `ui_preferences.rs` gained a `themePack` (default / high contrast / Solarized) applied over both color schemes and client-local `keybindings` (open settings, marketplace, goals) that the shell consumes; invalid or duplicate chords fail validation and surface in the preferences editor.


### Packaged journey wave (A1) — 2026-10-24

`packaged-journey-probe.sh` now boots a local Messages provider (`mock-model-server.mjs`, `DSH_MOCK_MODEL_MODE=plain`) and drives five more journeys against the packaged tree: complete a turn, fork the Session, read the fork's transcript after a Host restart; the full Goal lifecycle (`create`/`get`/`pause`/`resume`/`edit`/`complete`/`clear`) plus a goal that survives a restart; and a marketplace-style install of the same fixture through a loopback npm registry (`installBundle` with an explicit `registry`). `office-component-probe.sh` installs the Office component, creates a Session, writes a `docx` with the component's Python, and renders it through `officeToPdf/render` (multipart PDF, 16,999 bytes).

`scripts/native-journey-probe.sh [terminal|timed-question|panels|update-ui]` runs packaged journeys in an isolated Home. `terminal` and `timed-question` launch the native app under headless Weston with env-gated Host probes; `panels` and `update-ui` use the existing egui AccessKit harness against the packaged Host. The terminal service journey **found and fixed a real packaging defect**: the subprocess runner resolves `node-pty` with its own `createRequire`, which does not consult the launcher's resolve hooks, so every `terminal/create` failed with `Cannot find module 'node-pty'`. `prepare-linux-runtime.ts` now materializes a `node-pty` package that re-exports the Rust provider, and the terminal journey is in the release lane. The timed-question journey cannot run on the shipped composition: the preset composes the blocking tool, and enabling `mode: timed` means overriding the preset's `config.plugins`, i.e. changing the model-visible tool surface.

Release source separates `manifest-native.json` from the webview `manifest.json`, signs both with the existing Host canonical bytes, and embeds shell-safe feed defaults in AppRun. Native Host startup consumes framed `ready`/`fatal` messages, forwards update prompts to egui for explicit confirmation, and schedules restart only after installation. The restart helper selects the outer `$APPIMAGE`, waits for the old process to exit, and refuses an invalid image path rather than falling back to the mounted binary. Native app headless tests are included in the Linux CI source.

Validation: native-app 75/75; native release build; AppImage probe; and native GUI boot/auth/stream probe under isolated Weston pass. The UI preference file has round-trip, validation, failure-preservation, default-loading, and theme-application tests. The AppImage is 87,587,320 bytes at that point; the physical-desktop pass below supersedes its open GUI items. `test:docs` remains 20/21 because of the known `python/sdk/uv.lock:113` repository-reference failure. No production key, publication, tag, or active-desktop GUI interaction was used. Goal 3 remains open for release and the remaining live Host journeys.

## Native functional controls — working tree 2026-10-24

The egui layout presents Host update confirmations, approval delegation, older-page navigation, profile-default model initialization, Goal lifecycle controls, and `/plan` commands through existing Host APIs. Native UI preferences are stored separately in versioned `native-ui-preferences.json`; theme, scale, density, and default Sessions width can be previewed and saved without a Host connection. Invalid files leave their bytes unchanged and show a Settings error; the JSON editor validates drafts before applying them. The current AppImage passes its package probe, and the native app boots, authenticates, and reads streams under isolated Weston; Settings interactions still need GUI verification.

## Cập nhật native UI — 2026-10-23

`native-app` có navigation rail/sidebar, session-list renderer riêng, composer multiline với Ctrl+Enter, loading/disconnected/retry states, transcript renderer riêng, Goal lifecycle panel, older-page navigation, profile-default model initialization, approval delegation, và update confirmation. Eguis headless tests dựng layout ở bốn viewport và giữ transcript 1.000 dòng trong vùng cuộn; `cargo test` hiện có 69 tests và release build xanh. Physical-desktop IME/clipboard và visual review vẫn chờ.

`bun run build` xanh (9/9); `bun run test:docs` đạt 20/21, lỗi `python/sdk/uv.lock:113`; `bun run doc-sync` đạt 39/43, còn ba generated catalogs stale (`api-catalog.ts`, config catalog, event graphs) và cùng repository-reference error. Không sửa các generated artifact ngoài phạm vi.

## Dọn build và release artifacts — 2026-10-23

`bun run clean` đã xóa 359 build/runtime paths; `cargo clean` cho `native-app`, `client-core`, `src-tauri`, và `native/dsh-native` đã thu hồi tổng cộng khoảng 24,3 GB. Runtime tree Linux sau đó được dựng từ các archive đã cache và xác minh hash; `.desktop-build` hiện khoảng 3,3 GB. Native AppImage `0.2.1-rc.4` được đóng gói lại (87.521.784 B) với URL feed native và component feed. `.artifacts` (23 MB), `.cache` (416 MB), và tracked `apps/desktop-tauri/dist/index.html` vẫn được giữ nguyên.

Probe AppImage có thư mục `squashfs-root` chỉ đọc nên lần đầu `bun run clean` dừng với `EACCES`; trước khi chạy lại, mở quyền ghi cho riêng cây generated bằng `chmod -R u+w apps/desktop/.desktop-build`. Runtime preparation lần đầu timeout vì dùng cache path rỗng; rerun với `/home/masis/.cache/dsh-primary-runtime` dùng các archive đã xác minh và hoàn tất.

Remaining: successful Retry after a recovered live Host, timed-question composition, production feed signing/publication, physical-window input and visual review, and the release lane. Packaged Marketplace installation, Office preview, PTY TUI input, and the signed fake-image GUI swap are covered by the 2026-10-25 probes above; see [`PARITY.md`](PARITY.md) for evidence levels.

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

## 2b. Goal 2 — release-grade (đang mở, 2026-10-22)

Mục tiêu và acceptance đầy đủ ở `PLAN.md` § "Cấu trúc mục tiêu". Bằng chứng mới trong session này:

- **Packaged journey** (`packaged-journey-probe.sh`, L2): cài plugin từ tarball → Host apply → restart → gỡ; tạo/rename Session → restart → đọc lại. Tất cả PASS.
- **Fault injection** (`fault-probe.sh`, L2): thiếu runtime, profile hỏng, crash -9 → takeover lease, shutdown sạch, offline update, offline component. Tất cả PASS.
- **Disk-full** (`disk-full-probe.sh` dưới `isolated-run.sh` 48 MB): tải component bị ENOSPC, `phase=error`, `failure=download`, không còn file dở/`current.json`. Source map ENOSPC/EDQUOT/EFBIG/EROFS sang `download` thay vì `download-network`; unit test mới trong `components.spec.ts`.
- **Crash leftovers**: `DesktopOfficeComponent.recover()` chạy lúc Host khởi động, xoá `download-*`/`staging-*` còn lại, không đụng `versions/`/`current.json`; `fault-probe.sh` thêm kịch bản `crash-leftovers` ⇒ PASS.
- **Kill giữa download**: `crash-download-probe.sh` phục vụ archive chậm, kill -9 sau khi file dở > 64 KiB; boot lại ⇒ `left_before=1`, `cleaned=0`, `current=absent`.
- **Kill giữa extract**: `crash-extract-probe.sh` kill -9 khi `staging-*` xuất hiện; boot lại ⇒ `staging=0`, `versions=0`, không cài dở. `recover()` cũng xoá version không được `current.json` trỏ tới.
- **Update trọn luồng** (`update-flow-probe.mjs`): sau D40 probe này **bị hỏng** (manifest schema 1 + không ký) và đã sửa: schemaVersion 2, ký Ed25519 bằng key tạm, truyền `DSH_DESKTOP_UPDATE_PUBLIC_KEY`; giờ `RESULT update-flow=OK` (prompt download/install, `restart`, ảnh được thay).
- **Profile contention**: `native-lease-probe.sh linux-x64` proves a second packaged Host is refused and a new Host can take over after the owner dies.
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
| M1 | `client-core`: envelope, auth (token→cookie), HTTP + cancel, shared mux; inventory 82 requests + 12 streams, including Goal and command RPCs | done, exact request coverage and Goal/command wire behavior re-audited 2026-10-24 |
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

### Đã chốt — 2026-10-23 (đã commit)

- Khối working tree của session 2026-10-23 đã chia thành 3 commit: `fix(desktop-client)` (wire protocol), `feat(desktop-native)` (egui panels), `docs(desktop)` (mốc).
- **Huỷ HTTP trong lúc bay**: transport chạy `reqwest` async trên runtime tokio riêng, `select!` với cờ huỷ ⇒ request đang chờ bị drop (test với server treo).
- **Terminal ANSI**: `native-app/src/terminal_ansi.rs` render SGR (16/256/RGB + bold/dim/italic/underline/inverse), `\r` ghi đè, tab, `ESC[K`; LayoutJob monospace; cache theo revision. 9 test.
- **Probe native**: `apps/desktop-tauri/scripts/native-gui-probe.sh` chạy binary release + cây đóng gói, Home/project tách riêng, `DSH_DESKTOP_AUTO_CONNECT=1`, chờ `dsh: native host=ready` / `auth=ok`, in RSS/PSS của cả cây.
- **Số đo thật (Hyprland)**: **263 MiB RSS / 240 MiB PSS** khi idle sau auth ⇒ **đạt ngân sách < 300 MiB** (điều kiện "xong" #2).
- **AppDir tự chứa đã chạy**: `package-appimage.sh --appdir-only` rồi `AppRun` với `DSH_HOME` tạm ⇒ `host=ready` + `auth=ok`.
- Test: `client-core` 63 unit + 8 multipart; `native-app` 25 unit. `cargo build --release` xanh (app 40 MB).
- Bẫy mới: cửa sổ trên workspace `special:` + `silent` ⇒ egui không chạy frame ⇒ không đọc `host_receiver`; probe phải mở cửa sổ thật. Và Host orphan giữ lease khiến lần chạy sau không `ready` — luôn dùng work dir mới.

### Tiếp — 2026-10-23 (đợt 2, đã commit)

- **Mux dùng chung + reconnect**: `RemoteStreamMux` giữ một WebSocket cho mọi logical stream; mất carrier → stream đang mở fail có kiểu, `open` mới xếp hàng chờ socket kế tiếp (backoff 200 ms→5 s). Bỏ mô hình một socket mỗi stream.
- **Đủ 12 stream endpoint** trong `client-core/src/streams.rs` + `stream.rs` (typed frame, shared mux): session/control, workspace/follow, job/list, job/follow, workspaceFiles/changes, account/watch, account/watchExpiry, remoteAccess/follow, userQuestions/attachWait (cộng 4 đã có). Probe thật xác nhận 7/9 không cần id sống; `job/follow` và `attachWait` cần job/câu hỏi sống.
- **Cửa sổ input terminal**: `TerminalWriter` theo `maxInputBytes`, `maxBufferedBytes`, `inputWindow` trong snapshot; queue + ack, chặn frame quá lớn, clamp resize.
- **Đính kèm tệp**: `POST /api/session/uploadFileBinary` + `PromptContentPart::File{receiptId}`; nút Attach file… trong panel chat.
- **Panel Jobs**: `job/list` + `job/follow`, output 128 KiB.
- **Probe mở rộng**: chờ `$events=ready`, in từng stream, và `pluginInventory/list`; số đo mới **265 MiB RSS / 243–245 MiB PSS**.
- **Preview ảnh** (PNG/JPEG/GIF/WebP/BMP/ICO, trần 16 MiB/8192 px), **tab Plugins** read-only, **model picker** (catalog + selectModel), **panel Schedules** (list/delete), nút **Cancel turn**, và **session list theo title/tuổi/running**.
- **Transcript**: snapshot + event sống → message/tool/approval/job + draft streaming, trần 2000 row/256 KiB.
- **Feedback**: 👍/👎 cho assistant message theo `messageId`, list hiện có khi mở session, lưu qua `messageFeedback/put`.
- **Settings ghi bằng mutate** (xoá field, bảo vệ secret slot) và **bật/tắt plugin/bundle** trong tab Plugins.
- **Office preview** (Host `officeToPdf/render` → pdftoppm → image), **Marketplace** tab (search + install), **feedback** 👍/👎.
- **Updater**: Host verifies the Ed25519 feed and artifact before installation; native egui asks before download and install, then schedules the outer AppImage after the old process exits. Packaged signed swap/restart remains unverified.
- **Native AppImage**: `DSH_DESKTOP_SHELL=native bash apps/desktop-tauri/scripts/pack-appimage.sh linux-x64` (87,6 MB), `appimage-probe.sh` PASS (AppRun contract + Host boot, headless); release workflow source now packages/verifies the native image and publishes a separate signed `manifest-native.json`, but no release run has validated it.
- **Skills tab** (list theo session), **Deliverables panel** (link mở preview), **transcript card** cho todo/goal/plan + reasoning.
- **Question card**: `ask_user_question` hiện card chọn đáp án (một/nhiều + free text), gửi qua waterfall `$events`; trước đó app delegate mọi event lạ nên câu hỏi thật bị `NO_PROVIDER`.
- **Approval + job + câu hỏi thật**: `scripts/approval-probe.sh [allow|reject|delegate]` + `scripts/mock-model-server.mjs` chứng minh ask → allow (tool chạy, marker ngoài workspace) / reject / delegate (audit đúng, không chạy), `job/follow` đọc output job nền, và `ask_user_question` được trả lời qua waterfall `$events`.
- Lệnh: `bash apps/desktop-tauri/scripts/approval-probe.sh` (cần display; log trong `~/.cache/dsh-probe/approval.*`).
- Test: `client-core` **79 unit + 8 multipart**, `native-app` **25 unit**.

**Còn lại (2026-10-24):** packaged Host journeys for session fork, PTY execution, settings writes/persistence, Goal mutations, marketplace-registry installation, PDF rendering, and timed-question waits; native GUI-driven update acceptance/restart; interactive IME/clipboard; UI fault injection and independent visual review; release-lane signing/publication. The packaged tree was rebuilt from verified cached runtime archives; no release was published.

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
| **Native runtime tree** | `apps/desktop/.desktop-build/targets/linux-x64/` currently contains the rebuilt package tree and native AppImage. Use `native-lease-probe.sh` and `update-flow-probe.mjs` for egui-era checks; `update-probe.sh`, `desktop-lease-probe.sh`, `graphics-probe.sh`, and `cold-start-probe.sh` still require the retired Tauri shell. |
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
 bun run typecheck
 bun run build
 bun run lint
 bunx vitest run apps/desktop-tauri/tests/native-release.spec.ts
 bash apps/desktop-tauri/scripts/native-protocol-probe.sh
 cargo test --locked --offline --manifest-path apps/desktop-tauri/native-app/Cargo.toml
 cargo build --locked --offline --release --manifest-path apps/desktop-tauri/native-app/Cargo.toml
 bash apps/desktop-tauri/scripts/entry-parity-probe.sh linux-x64
 bash apps/desktop-tauri/scripts/native-lease-probe.sh linux-x64
 bash apps/desktop-tauri/scripts/packaged-journey-probe.sh linux-x64
 bash apps/desktop-tauri/scripts/fault-probe.sh linux-x64
 bash apps/desktop-tauri/scripts/crash-download-probe.sh linux-x64
 bash apps/desktop-tauri/scripts/crash-extract-probe.sh linux-x64
  DSH_ISOLATION_DIR=/home/masis/.cache/dsh-probe/office-component-current bash apps/desktop-tauri/scripts/isolated-run.sh bash apps/desktop-tauri/scripts/office-component-probe.sh
  bash apps/desktop-tauri/scripts/office-engine-probe.sh linux-x64
  DSH_ISOLATION_DIR=/home/masis/.cache/dsh-probe/update-flow-current bash apps/desktop-tauri/scripts/isolated-run.sh node apps/desktop-tauri/scripts/update-flow-probe.mjs linux-x64
 bash apps/desktop-tauri/scripts/appimage-probe.sh apps/desktop/.desktop-build/targets/linux-x64/appimage/DeepSeek-Harness-native-0.2.1-rc.4-x86_64.AppImage
 DSH_DESKTOP_SHELL=native bash apps/desktop-tauri/scripts/pack-appimage.sh linux-x64
 # The native GUI probe requires a display; run only on an idle physical desktop or isolated compositor.
# B1 (khi được duyệt phát hành): bump `0.2.1-rc.4` đã có sẵn trong branch history
git push fork chore/web-bun-migration                    # đẩy commit bump lên fork
git tag desktop-v0.2.1-rc.4 && git push fork desktop-v0.2.1-rc.4   # kích hoạt lane release
```
Phát hành (làm thay đổi trên `fork`): bump version **mọi package** ⇒ tag `desktop-v<version>` ⇒ lane release tự chạy ⇒ feed mới đã ký.

## 7b. Kiểm thử UI không cửa sổ (gpui kittest / harness)

Không dùng toạ độ pixel, không chạm Hyprland: harness dựng **cùng** `DesktopApp::tick`/`render` mà cửa sổ dùng, trong `egui::Context` không có OS window.

```sh
cargo test --locked --offline --manifest-path apps/desktop-tauri/native-app/Cargo.toml ui_harness
```

- `src/ui_harness.rs` (chỉ `cfg(test)`): `FixtureHost` là HTTP Host trong tiến trình, trả lời `session/list`, `session/create`, `session/prompt`, `session/page`, `model/catalog`, `settings/describe|mutate` (tự áp path op vào document của nó rồi tăng revision), `pluginMarket/listIndexes|search`, `pluginManager/inspect|installBundle`, `pluginInventory/list`, `officeToPdf/render`, và `POST /__dsh-desktop/updates-open` (204 hoặc lỗi); `GET /connect` trả 503 để test attach thất bại. Nó **ghi lại request** để assert payload; `RemoteStreamMux::loopback` (feature `test-fixtures` của client-core) phát frame follow, replay snapshot cho stream mở muộn.
- Widget được định danh bằng AccessKit: `harness.get_by_label("New Session")`, `"Composer"`, `"Send"`, `"Allow once"`. `TextEdit` không có tham số label nên `accessible_label()` gắn tên ổn định (Composer, Model, Session title, Terminal command, Marketplace search, Document path, Host settings JSON); nút "Search" của Marketplace có tên riêng `Search marketplace` vì panel Sessions cũng có nút "Search"
- Test hiện có (20 test thường): luồng semantic (New Session → chọn session → gõ → Send → payload prompt đúng → event follow hiện lên → Allow once phát `ApprovalCommand::Answer`), mở session cũ có tool events + deliverable trong lịch sử, snapshot chen lúc load, chuyển session, 2.100 event cắt còn 2.000, phân trang 120 event, Settings (ghi + đọc lại + xung đột revision), Marketplace (search + install + nhánh từ chối), Retry connection (bấm nút, Host nhận `GET /connect`, bị từ chối ⇒ vẫn disconnected), preview Office→PDF→raster (và nhánh PDF hỏng), update check Host bridge (thành công + lỗi), terminal TUI (AccessKit gửi bracketed paste, click SGR trên màn hình, wheel được xếp và phát sau ack), layout 3 viewport + report JSON ở `target/dsh-kittest/layout.json`, 3 snapshot vỏ rỗng ở `tests/snapshots/`. Hai test ignored trong cùng harness chạy bằng `native-journey-probe.sh panels|update-ui` để gắn renderer vào Host đóng gói.
- **Fixture PASS ≠ live PASS**: reply của fixture chứng minh UI phát đúng request và render đúng reply, không chứng minh Host thật ghi settings, cài bundle hay reconnect. Retry chỉ chứng minh *attach attempt* + nhánh từ chối; reconnect thành công cần Host thật vì harness không có WebSocket mux. Settings test kiểm tra read-back bằng revision mới do fixture áp op, nên vẫn là fixture-side effect.
- Terminal TUI compatibility: mode tracker chạy tăng dần theo output frame và giữ DECSET state sau khi output buffer bị trim; paste/mouse được encode bằng cùng `TerminalWriter`, ack giải phóng hàng đợi theo thứ tự, refusal xoá phần còn lại và hiện lỗi. Native unit + kittest assert input/output bytes. Một phép chạy thủ công trong PTY cô lập với Neovim `mouse=a` xác nhận Neovim bật `2004`, `1002`, `1006`, lưu chính xác paste hai dòng được bọc bằng `ESC[200~`/`ESC[201~`, và đổi vị trí chèn theo SGR mouse click. Đây là kiểm tra giao thức với TUI thật, không phải probe Host/AppImage hay GUI OS.
- Packaged native-panel journeys: `native-journey-probe.sh panels` cài Office component trong Home cô lập, cấu hình loopback market/registry rồi chạy `egui_kittest` với transport và mux thật; AccessKit cài bundle và kiểm tra `listBundles`, preview `probe.docx` qua Office Host + `pdftoppm`, tạo/attach terminal, chạy TUI qua PTY và xác nhận bracketed paste + mouse report tới TUI. `native-journey-probe.sh update-ui` dùng key Ed25519 sinh theo lượt, fake AppImage executable, và cùng harness để bấm Download/Install and restart; test kiểm tra byte ảnh, permission executable, `HostEvent::Restart`, rồi probe chờ fake image chạy sau khi test process thoát. Đây là L2 packaged Host + UI renderer, không phải physical-window L3 hay feed production.
- Bẫy: sau mỗi click AccessKit phải `harness.run_steps` trước khi `host.wait_for`, vì `wait_for` chỉ chờ phía fixture và không đẩy frame UI. Bẫy thứ hai: `Modifiers::CTRL` **không** khớp select-all của egui; dùng `Modifiers::COMMAND`, và assert lại `value()` của editor sau khi gõ thay thế.
- Snapshot chỉ là baseline chống hồi quy; **chất lượng thị giác vẫn chờ review độc lập**. IME, tray, drag & drop, hành vi cửa sổ vẫn do probe Weston cô lập lo.
- Cập nhật baseline: `UPDATE_SNAPSHOTS=1 cargo test ... ui_harness`, rồi xem lại ảnh trước khi commit.
- Đo bộ nhớ trên desktop thật: dựng binary release, chạy với `DSH_HOME` cô lập trên một workspace, rồi đọc `VmHWM`/`VmRSS` từ `/proc/<pid>/status` và `Pss:` từ `/proc/<pid>/smaps_rollup` sau mỗi thao tác (boot → tạo Session → dán chữ CJK → mở Settings). Số 2026-10-24: 92,7 MB peak (79,6 PSS) lúc boot, 133,5 MB (120,5 PSS) khi có CJK + Settings. **Bẫy**: cửa sổ DSH của người dùng cũng có class `DeepSeek Harness`, nên `hyprctl -j activewindow` theo class là chưa đủ — phải so `pid` với pid app vừa lao động trước khi gõ phím; và `DSH_HOME` cô lập chỉ tách session, còn số liệu usage trong Settings vẫn đọc từ dữ liệu chung nên đừng nhầm đó là session của mình.
- Lịch sử Session: `session/page` và `session/follow` cùng đổ vào `Transcript::merge` (sort theo `seq`, mỗi event project một lần, trang cũ chèn lên trước, snapshot cửa sổ không xoá lịch sử cũ). Danh sách event thô (seq/type/text) đã bỏ để chỉ còn một renderer; nút "Load older events" chuyển lên đầu khung chat. Test: mở session cũ (kèm tool events), snapshot chen lúc load, chuyển session, 2.100 event (cắt còn 2.000), phân trang 120 event; ordering của live stream được test riêng ở tầng wire frame (`live_follow_frames_project_in_wire_order`), không dựa vào replay của loopback.

## 8. Tài liệu lane
`.agents/plan/desktop-tauri/PLAN.md` (kế hoạch + bảng finding F01–F13) · `MEMORY.md` (quyết định D1–D39, số đo M1–M25, bẫy B1–B52) · `AGENTS.md` (luật lệ của lane) · `probes/` (probe cũ) — cập nhật cả hai file đầu khi có quyết định mới.
## Release measurement — 2026-11-01

- `cargo build --release --bin dsh-desktop-native-app`: 9m42s, 44.5 MB binary.
- Real Hyprland run, packaged runtime tree, isolated Home: `host=ready` 5.0–14.4 s, `auth=ok` +0.2–0.9 s — measured while the host machine had load average 20.4 from unrelated `cc1plus` processes; quiet debug runs were 2.5–3.7 s.
- Idle process-tree RSS after auth ≈ 290–297 MiB (app ~114 MB + Host ~184 MB), under the 300 MiB gate but close to it.
- The eframe/epaint link was removed on 2026-11-01 (see the split section below): release binary 43.99 MB, debug 646 MB, idle RSS unchanged at about 299 MiB.

## Tách `eframe`/`egui` khỏi binary gpui — 2026-11-01

`eframe` giờ là dependency **optional** sau feature `legacy-egui` (`default = []`), và bin `dsh-desktop-egui-legacy` khai báo `required-features = ["legacy-egui"]`. Nhờ vậy binary gpui không còn link egui/epaint; kiểm chứng bằng `strings` trên binary build ra: 0 ký hiệu `egui::`/`eframe::` (chỉ còn chuỗi `prepaint` của gpui).

Phần trình bày egui trong các module dùng chung được gate theo feature thay vì bị xoá: `fonts::install`, `terminal_ansi::{format_for, layout}`, `explorer` (ui + render + source preview), `question_card::render`, `session_list::render`, `document_preview` (ImagePreview + DocumentPreview::ui), `ui_preferences` (ThemeMode→ThemePreference, KeyChord::consume, key_by_name, apply/apply_theme_pack/accent_fill, UiPreferencesPanel + editor helpers), `layout` tests. Trạng thái/model/protocol vẫn dùng chung: `KeyChord` giờ là dữ liệu thuần (`ctrl/alt/shift/cmd` + tên phím chuẩn hoá) nên `UiPreferences::validate` kiểm trùng phím cho cả hai bin, còn `KeyChord::consume` chỉ tồn tại ở bản egui. `Palette`/`AnsiStyle` chuyển sang màu 8-bit thuần (`Rgba8`, `AnsiColor`) và chỉ convert sang `Color32` trong code egui.

Hai test terminal trước đây kiểm cả parse lẫn format egui được tách: phần parse ở lại bin gpui, phần format chuyển sang `formatted_spans_swap_foreground_and_background_when_inverse` (legacy). Clippy: **66 warning ở bin gpui** (đều là dead code/visibility có sẵn trong `shell/`, không phải lỗi) và **0 warning code** ở bin legacy.

## Xoá shell egui legacy — 2026-11-06

Bin `dsh-desktop-egui-legacy`, feature `legacy-egui`, optional `eframe`, `egui_kittest` dev-dependency, `src/main.rs`, `src/ui_harness.rs` + `src/ui_harness/audit.rs`, `src/settings_panel.rs`, `src/transcript_view.rs`, `tests/snapshots/*.png` và toàn bộ khối `#[cfg(feature = "legacy-egui")]` trong module dùng chung đã bị xoá. Lý do: chỉ còn một renderer cần bảo trì; hai bản UI song song buộc mọi feature phải làm hai lần và đã phát sinh đúng lỗi đó trong các slice UI V2.

**Harness thay thế (06/11):** `src/harness.rs` mở `DesktopApp` thật trong test window của gpui rồi đọc hình học renderer tạo ra (`debug_bounds`), so với baseline `tests/snapshots/shell-1366x768.txt` và `shell-1920x1080.txt`; `cargo test harness` phủ bar 30/34/26, ngưỡng dock Explorer/Changes, toggle Changes, và chuyển trang (page thay conversation, ẩn hai dock). `native-journey-probe.sh shell` chạy harness với Host đóng gói: shell kết nối, tạo Session qua đúng đường RPC của nó, và xác nhận Session có trong list (L2, chạy thật 1,8 s, `host=ready 1529ms`). gpui 0.2.2 **không có accessibility tree**, nên không thể khẳng định kiểu AccessKit; chưa có pixel.

**Mất mát phải bù:** harness `egui_kittest` là nơi duy nhất chạy headless UI (AccessKit + snapshot PNG) và là L2 cho các journey `panels`/`model-default`/`update-ui`. Vì vậy ba probe GUI đó đã bị gỡ khỏi `native-journey-probe.sh`, `office-component-probe.sh`, `model-default-probe.sh`, `update-flow-probe.mjs`; hiện **chưa có harness UI tự động cho gpui**. `native-journey-probe.sh terminal|timed-question|model-default` vẫn chạy qua probe hook env của gpui. Việc dựng lại harness tương đương trên gpui là mục mở, không được tính là đã xong.

## AppImage native gpui — 2026-11-01

`DSH_DESKTOP_SHELL=native apps/desktop-tauri/scripts/pack-appimage.sh` đóng gói lại `apps/desktop/.desktop-build/targets/linux-x64/appimage/DeepSeek-Harness-native-0.2.1-rc.4-x86_64.AppImage`: **87.181.816 byte** (bản cũ 87.833.080). Binary nhúng giống hệt bản release vừa build (**43.989.408 byte**), `strings` không còn ký hiệu `egui::`/`eframe::`.

Kiểm chứng: `scripts/appimage-probe.sh` pass cả hai chặng (AppRun contract + Host packaged boot/shutdown, `RESULT host=OK`); chạy thật bằng `APPIMAGE_EXTRACT_AND_RUN=1` trên Hyprland với Home/TMPDIR cô lập: `host=ready in 1414ms`, `auth=ok in 1495ms`, `events=ready`, cửa sổ render đủ nav rail, Sessions panel, empty state và bottom bar.

## Bàn giao phiên 2026-11-06 — C/D/E + chốt scope A

Trạng thái nhánh: `chore/web-bun-migration`, **chưa push**, working tree sạch. Tất cả commit dưới đây nằm trên nhánh này.

### Quyết định đã chốt trong phiên
- **A — `ask_user_question`:** cho **cả hai mode cùng tồn tại**. Mặc định preset vẫn là `legacy` (blocking); `timed` đã implement đầy đủ trong `packages/interaction/tool-ask-user` nhưng phải mount bằng config/overlay. **Chưa** mở đường chọn mode lúc chạy — nếu muốn có journey packaged cho timed thì phải chọn: overlay/profile riêng, hoặc Settings chọn mode (đổi model-visible tool surface ⇒ quyết định sản phẩm).
- **Verified cho plugin:** tạm hoãn. Không có cơ chế chữ ký/kiểm chứng index; Marketplace chỉ hiện trust facts + `permissions[]` do index khai báo.
- **Pixel snapshot gpui:** vẫn pending (gpui 0.2.2 không có API rasterize/accessibility tree).

### Đã xong (theo thứ tự commit, `git log --oneline` để lấy hash)
| Slice | Nội dung |
|---|---|
| git merge/revert | `git/merge` + `git/revert`; `continueOperation`/`abortOperation` phủ cả merge/revert; UI Branches có Merge, branch hiện tại có Revert tip. Refusal message nêu đủ 4 operation. |
| approval requestId | `approval/request` mang `requestId` (= id của `approval/asked`); browser contract + `PendingApproval` + Rust `ApprovalRequest.request_id`; shell bỏ card trùng id sau reconnect. Scope vẫn chỉ `allowed-once`. |
| search jump-to-line | Click kết quả content search đặt caret đúng dòng; viewport cuộn theo caret; target gắn theo path để mở file khác không dính dòng cũ. |
| quick-open paths | `workspaceFiles/searchPaths` (bounded, không đọc nội dung) + palette gộp với cây Explorer đã tải ⇒ quick-open phủ workspace chưa mở. Một request in-flight, kết quả cũ bị bỏ và hỏi lại theo query mới. |
| file rename/delete | fs seam thêm `rename`/`remove` **virtual** (backend không hỗ trợ → `FS_IO_ERROR`, không giả lập read+write); `workspaceFiles/rename|delete` có stale guard, `FS_ALREADY_EXISTS`, từ chối xoá thư mục; sandbox fence cả 2 đầu rename; document page có Rename + Delete 2 bước, Explorer refresh sau mutation; SSH khai báo `FS_ALREADY_EXISTS` nhưng chưa implement move/delete. |
| divider reset | Double-click vách Explorer/Changes reset width + xoá override workspace (divider tự đo 400 ms vì gpui không có sự kiện double-click). |
| file drop | Kéo-thả file vào cửa sổ (`on_drop::<ExternalPaths>`), đi qua đường upload sẵn có; drop nhiều file báo rõ chỉ giữ file đầu. |
| MEMORY | MEMORY.md ghi kết quả + phần chưa làm. |
| doc debt | E: PARITY.md + capability map bỏ mọi dòng trích gate egui đã xoá (`ui_harness`, `panels`, `update-ui`, PNG) và các gap đã đóng (Changes data thật, workspace override, paging/dedupe test, prompt dedupe, handoff, skill enable/disable). |

### Gate đã chạy (đều xanh ở trạng thái cuối)
- `apps/desktop-tauri/native-app`: `cargo test` **188 pass / 1 ignored** (ignored = packaged prompt→tool→approval journey).
- `apps/desktop-tauri/client-core`: `cargo test --locked --offline --manifest-path apps/desktop-tauri/client-core/Cargo.toml` → **103 + 8 + 11 pass** (client-core **không** là workspace member, phải chạy bằng `--manifest-path`).
- `npx vitest run packages/fs packages/api/workspace-files packages/ssh` → **1077 pass** (63 file).
- `packages/api/workspace-controller/tests/git.host.spec.ts` **78 pass**; `packages/util/plugin-market-index` **18**; `packages/boot/plugin-market` **102**.
- `bun run typecheck` **5/5**, `bun run test:docs` **21/21**, `verify-type-equiv` **494**, `verify-cordis-catalog` fresh, `verify-translation-pairing` **1018 pair**.

### Còn mở (đã cân nhắc, không phải bỏ sót)
1. **C3 — xoá cứng Session.** `packages/session/session-persistence` chỉ có `create/open/flush/stat/list`; provider duy nhất là `session-persistence-jsonl`. Muốn làm phải thêm `delete` vào seam + provider, chặn session đang mở/đang chạy, rồi `session/delete` + UI confirm. Đây là quyết định durability/data-destructive; archive hiện là đường an toàn được hỗ trợ.
2. **D2 — Diagnostics panel.** Không có nguồn dữ liệu: `packages/lsp/lsp-stdio/src/connection.ts` **chủ động bỏ** notification `publishDiagnostics`, và `packages/lsp` tuyên bố diagnostics ngoài scope. Cần owner state mới (accumulator + freshness/clear rules) trước khi có RPC/UI.
3. **D3 — nhiều buffer tab Agent**, **D4 — gộp Preferences thành một cửa sổ**: thay đổi layout/state lớn, nên làm PR riêng.
4. **D5 — terminal policy theo workspace.** Host chỉ có cap theo Session (`maxTerminals` trong `api-terminal-controller`); không có khái niệm workspace ⇒ cần quyết định sản phẩm.
5. **D7 — test composition IME tự động.** Harness không inject được composition của platform; hiện có test marked-text + bằng chứng tay trên desktop thật (fcitx5 Bamboo).
6. Việc cũ chưa đóng: production signing/published-feed swap, physical-window pointer + visual review, Marketplace rollback version cũ, T08 discard version/source guard, EXT-04 test lỗi extension, `request_id` phía client không retry tự động.

### File chính của phiên
- `packages/api/workspace-controller/src/{git,types}.ts`, `tests/git.host.spec.ts`
- `packages/interaction/user-approval/src/{index,types}.ts`, `packages/client/ui-approval/src/client/{index,contract/slots}.ts`
- `packages/api/workspace-files/src/{index,types}.ts`, `tests/{mutate,search-paths}.spec.ts`
- `packages/fs/fs/src/{index,types}.ts`, `packages/fs/fs-local/src/index.ts`, `packages/fs/fs-sandbox/src/index.ts`
- `apps/desktop-tauri/client-core/src/{git,approvals,workspace_files}.rs`
- `apps/desktop-tauri/native-app/src/shell/{app,text_input}.rs`, `src/shell/render/{mod,pages,sessions}.rs`, `src/ui_preferences.rs`
- Docs: `.agents/plan/desktop-tauri/{PARITY,MEMORY}.md`, `apps/desktop-tauri/native-app/DSH_UI_Handoff_v1.0/runtime-capability-map.md`, `docs/subsystems/{filesystem,approval,workspace}.md`

### Bẫy đã gặp trong phiên
- **gpui 0.2.2 không có**: accessibility tree, pixel rasterize, sự kiện double-click. Double-click phải tự đo thời gian; D1 đã làm theo cách đó.
- **`cargo test` ở `native-app` không chạy test của `client-core`** — phải chạy riêng bằng `--manifest-path`.
- **Type mới trong package dùng chung**: client/other package có thể đang đọc `lib/types` cũ ⇒ chạy `npx tsc -b packages/<...>/tsconfig.json` trước khi tin lỗi type.
- **`gen-cordis-catalog` sẽ fail nếu type mới chưa phân loại** trong `scripts/gen-cordis-catalog.ts` (`linkedTypePages` / `typeLinkExemptions`); sửa xong chạy lại generator rồi `verify-cordis-catalog`.
- **Sửa prose trong doc có bản dịch** ⇒ `pnpm run verify-translation-pairing --write <file>`, sau đó chạy corpus `pnpm run verify-translation-pairing` không tham số.
- **`verify-type-equiv`** bắt block `ts type-equiv` trong `docs/subsystems/*.md`; sửa source type phải sửa cả en + zh.
- **`pnpm-lock.yaml` drift** có thể là nợ thật từ commit trước; kiểm bằng `pnpm install --lockfile-only --frozen-lockfile` trước khi revert.
- Máy: `/` ~18 GB free; nếu linker `bus-error` thì `rm -rf apps/desktop-tauri/native-app/target/debug/incremental`.
- Packaged journey chạy bằng: `bash apps/desktop-tauri/scripts/native-journey-probe.sh shell` (cần cây packaged ở `apps/desktop/.desktop-build/targets/linux-x64`, mock model `scripts/mock-model-server.mjs` mode approval).

### Gợi ý thứ tự tiếp theo
1. C3 nếu owner chấp nhận semantics xoá cứng (viết Agent Note + upgrade guide nếu đổi surface).
2. D3 (multi-buffer) — tách state tab khỏi transcript hiện tại.
3. D2 diagnostics — cần package owner + quyết định freshness trước khi làm UI.
4. A runtime mode cho `ask_user_question` nếu owner muốn timed có journey packaged.
