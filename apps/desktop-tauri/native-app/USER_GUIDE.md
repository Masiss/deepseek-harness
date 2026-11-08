# Hướng dẫn sử dụng ứng dụng Desktop native

## Tóm tắt

Ứng dụng cho phép bạn quản lý Sessions, trò chuyện với agent, xem tệp trong workspace và theo dõi các công việc gắn với từng Session thông qua Desktop Host.

Tài liệu này mô tả ứng dụng Rust/gpui trong [`src/`](src/) (`src/gpui_main.rs` và `src/shell/`), là renderer duy nhất của lane. Giao diện HTML trong [`prototype/`](prototype/) chỉ là bản mô phỏng, không phải ứng dụng đang chạy.

Tên menu và nút được giữ bằng tiếng Anh như trong ứng dụng để bạn dễ đối chiếu.

## Mục lục

- [Điều kiện và khởi động](#điều-kiện-và-khởi-động)
- [Bố cục và điều hướng](#bố-cục-và-điều-hướng)
- [Sessions](#sessions)
- [Trò chuyện, model và tệp đính kèm](#trò-chuyện-model-và-tệp-đính-kèm)
- [Phê duyệt và câu hỏi](#phê-duyệt-và-câu-hỏi)
- [Workspace Explorer và xem tài liệu](#workspace-explorer-và-xem-tài-liệu)
- [Changes, Git và lịch sử commit](#changes-git-và-lịch-sử-commit)
- [Plugins, Skills và Marketplace](#plugins-skills-và-marketplace)
- [Goals và Plan mode](#goals-và-plan-mode)
- [Terminal, Jobs, Schedules và Deliverables](#terminal-jobs-schedules-và-deliverables)
- [Preferences và cập nhật Desktop](#preferences-và-cập-nhật-desktop)
- [Bàn phím, khay hệ thống và thoát ứng dụng](#bàn-phím-khay-hệ-thống-và-thoát-ứng-dụng)
- [Dữ liệu và khắc phục sự cố](#dữ-liệu-và-khắc-phục-sự-cố)

-----

## Điều kiện và khởi động

Ứng dụng cần kết nối được với Desktop Host để tải Sessions, gửi prompt, đọc workspace và dùng các tính năng phụ thuộc Session.

1. Khởi chạy bản Desktop native đã được phân phối cho bạn.
2. Ứng dụng khởi động Host sidecar nếu bản cài đặt cung cấp cấu hình Host và mặc định tự kết nối khi Host sẵn sàng.
3. Nếu màn hình hiển thị nút **Connect**, chọn nút đó để xác thực và tải danh sách Sessions.
4. Khi trạng thái báo Host bị ngắt kết nối, chọn **Retry connection** để thử kết nối lại.
5. Chọn một Session có sẵn hoặc tạo Session mới bằng **New Session**.

Nếu Host chưa được đóng gói hoặc không khởi động được, ứng dụng vẫn mở nhưng không thể tạo Session mới hay thực hiện thao tác cần kết nối. Hãy liên hệ người quản trị bản cài đặt thay vì tự sửa các biến môi trường của Host.

Host quản lý danh mục model. Bạn có thể tạo Session khi chưa có model provider, nhưng cần cấu hình provider ở Host trước khi gửi prompt thành công.

Xem [USER_FLOWS.md](USER_FLOWS.md) để theo dõi các nhánh từ lúc mở ứng dụng đến khi hoàn tất công việc.

-----

## Bố cục và điều hướng

Thanh trên cùng cao 30 px dùng nút biểu tượng vector (menu, Search, Changes) thay cho ký tự Unicode; workspace picker gồm tên thư mục và mũi tên xuống, và không còn tiêu đề Session ở giữa. Search mở command palette, còn workspace picker liệt kê các workspace của những Session đã biết và chuyển sang Session mới nhất của workspace đó. Không có thanh điều hướng dọc: menu chia ba nhóm — **Workspace** (New session, Toggle Explorer, Toggle Changes, Show changes (diff), Open file…, Goals), **Customize** (**Preferences…**, **Extensions**, **Skills**, **Marketplace**) và **Help** (**About DSH**, Quit).

Bố cục gồm ba cột: **Explorer** bên trái, cột **Agent** ở giữa với tab Agent, transcript và composer, và **Changes** bên phải với ba tab Diff/Files/History. Thanh dưới cùng cao 26 px: bên trái hiển thị biểu tượng branch kèm tên branch hiện tại (bấm để mở menu nhánh) và số file thay đổi do Host báo, hoặc **No repository** khi workspace không phải Git; bên phải là Terminal, Problems, menu Jobs/Schedules/Deliverables/Diagnostics và chấm kết nối Host, tất cả đều dùng biểu tượng vector.

Explorer rộng mặc định 300 px (210–390), Changes 565 px (340–720); Explorer chỉ dock khi cửa sổ đủ rộng để cột Agent còn tối thiểu 420 px (khoảng 1293 px), Changes cần tối thiểu 1180 px. Trong Explorer, ở chiều rộng không quá 900 px, panel hiển thị lần lượt cây thư mục hoặc phần xem tệp.

Kéo vách phân cách giữa Explorer hoặc Changes với cột Agent để đổi chiều rộng; chiều rộng được lưu vào preferences khi thả tay. Cột **Changes** có ba tab Diff/Files/History và chỉ đọc dữ liệu Git từ Host; chi tiết từng tab, menu nhánh, menu đồng bộ, stash, xung đột và xoá tệp untracked nằm ở [Changes, Git và lịch sử commit](#changes-git-và-lịch-sử-commit).

-----

## Sessions

Session chứa transcript và workspace riêng, đồng thời là phạm vi cho Goals, terminal, jobs, schedules, skills và model đã chọn.

- Mở bộ chuyển Session bằng nút tên Session ở đáy Explorer. Popover có ô tìm kiếm và nút làm mới; chọn ra ngoài popover để đóng. Nút workspace ở thanh trên mở workspace picker riêng, không mở danh sách Session.
- Chọn một hàng trong danh sách để mở Session. Dấu chấm bên hàng và nhãn **● Running** cạnh tiêu đề Agent báo Session đang chạy; dòng phụ hiển thị thời điểm cập nhật và trạng thái trống nếu không có nội dung.
- Nội dung chưa gửi trong composer được giữ riêng cho từng Session. Chuyển sang Session khác rồi quay lại sẽ thấy lại bản nháp; gửi prompt sẽ xoá bản nháp của Session đó. Bản nháp được lưu cục bộ sau khoảng 0,75 giây ngừng nhập.
- Khi **Restore last Session** bật (mặc định), lần mở ứng dụng tiếp theo sẽ mở lại Session cuối. Nếu Session đó không còn, ứng dụng mở Session cập nhật gần nhất và ghi rõ lý do; nếu chưa có Session nào, danh sách để trống.
- Nhập truy vấn ở **Search sessions**, rồi nhấn Enter hoặc nút tìm kiếm. Để trở lại danh sách đầy đủ, xóa truy vấn và làm mới danh sách.
- Chọn **＋** ở đáy Explorer hoặc **New session** trong popover để tạo Session mới. Ứng dụng mở Session vừa tạo và tải lại danh sách.
- Mở **Session actions** khi đã chọn Session để đổi tiêu đề hoặc chọn **Fork selected** để tạo một Session nhánh từ Session hiện tại.
- Chọn nút làm mới cạnh tiêu đề Sessions để tải lại danh sách.

Nếu thao tác tạo, đổi tên hoặc fork mất kết nối, ứng dụng báo rằng kết quả có thể chưa rõ. Làm mới hoặc tìm kiếm danh sách trước khi gửi lại thao tác để tránh tạo bản trùng.

-----

## Trò chuyện, model và tệp đính kèm

Mở một Session rồi nhập nội dung ở composer phía dưới transcript. Chọn **Send** hoặc nhấn **Ctrl+Enter** để gửi; nhấn Enter để xuống dòng. Khi agent đang chạy, nút **Cancel turn** gửi yêu cầu hủy lượt hiện tại. Dòng bắt đầu bằng `/` được gửi tới command registry của Host thay vì model; nếu Host không nhận ra lệnh, dòng đó quay lại composer để bạn sửa hoặc gửi như prompt thường.

Danh sách **Model** lấy từ Host. Mở danh sách để chọn provider và model cho Session. Nếu model có các mức reasoning effort, chọn mức cần thiết từ danh sách bên cạnh model. **Use profile default** chuyển Session về model mặc định của profile; **Initialize profile default** yêu cầu Host khởi tạo giá trị mặc định khi chưa có cấu hình.

Nút làm mới trong danh sách model tải lại catalog. Nếu catalog rỗng hoặc báo lỗi, kiểm tra cấu hình provider ở Host rồi thử tải lại.

**Attach** mở hộp thoại chọn tệp của hệ điều hành. Ứng dụng đọc bytes của tệp, tải tệp lên Host và đính kèm receipt vào prompt tiếp theo. Chờ upload hoàn tất trước khi gửi; nút × bên tên tệp bỏ tệp đính kèm. Composer giữ một tệp đính kèm tại một thời điểm.

Transcript hiển thị tin nhắn của bạn, phản hồi đang stream, kết quả công cụ, thông báo quyền, jobs, Todos, Goals, chế độ plan và các deliverables mà agent công bố. Chọn **Load older events** để nạp lịch sử cũ hơn khi nút này xuất hiện.

Chọn 👍 hoặc 👎 dưới phản hồi assistant để lưu đánh giá cho phản hồi đó. Ứng dụng tải lại đánh giá của Session và hiển thị trạng thái lưu hoặc lỗi.

Nếu Host mất kết nối trong lúc gửi, không thể biết chắc yêu cầu đã được nhận hay chưa. Kết nối lại, mở lại Session để kiểm tra transcript, rồi mới quyết định gửi lại.

-----

## Phê duyệt và câu hỏi

Khi công cụ cần quyết định quyền, ứng dụng hiển thị thẻ **Permission** cùng tên công cụ và lý do do Host cung cấp.

- **Allow once** cho phép lần yêu cầu đang chờ; lựa chọn này không cấp quyền lâu dài.
- **Reject** từ chối yêu cầu đang chờ.
- **Delegate** chuyển yêu cầu cho listener tiếp theo trong chuỗi xử lý sự kiện của Host.

Nếu agent đặt câu hỏi, thẻ câu hỏi hiển thị tiêu đề, nội dung, mô tả và các lựa chọn. Chọn một hoặc nhiều đáp án theo hướng dẫn trên thẻ; câu hỏi một lựa chọn chỉ nhận một đáp án. Bạn cũng có thể nhập câu trả lời riêng ở ô văn bản.

Chọn **Answer** để gửi các lựa chọn và nội dung đã nhập. Chọn **Skip** để gửi câu trả lời rỗng cho các câu hỏi trong thẻ. Một yêu cầu đã bị hủy hoặc event stream bị đóng có thể làm thẻ biến mất; nếu chưa rõ agent đã tiếp tục hay chưa, kiểm tra transcript của Session.

-----

## Workspace Explorer và xem tài liệu

Workspace Explorer đọc cây tệp thuộc Session đang chọn. Panel chỉ đọc (ghi trong tooltip của nút làm mới); bạn có thể mở rộng thư mục, làm mới cây và chọn tệp văn bản UTF-8 để xem nội dung. Tệp không được sửa trong Explorer.

Explorer tải thư mục khi bạn mở rộng nó. Trang xem văn bản đầu tiên tối đa 300 dòng; chọn **Load next lines** để đọc tiếp. Tệp nhị phân, liên kết tượng trưng, mục không hỗ trợ hoặc giới hạn từ Host sẽ được báo thay vì hiển thị như văn bản bình thường.

Explorer theo dõi thay đổi trên đĩa cho những thư mục đang mở và tệp đang xem. Thư mục đổi thì cây tự tải lại và bỏ các mục đã biến mất; tệp đang xem đổi thì hiện dòng nhắc **changed on disk** kèm nút tải lại trong tiêu đề preview; tệp bị xóa thì giữ trang cuối và báo **no longer exists on disk**. Nếu Host không hỗ trợ theo dõi, Explorer hiện thông báo và chỉ thử lại khi bạn bấm làm mới.

Màn hình **Files** mở phần xem tài liệu theo đường dẫn Host:

- Nhập đường dẫn rồi chọn **Preview** để xem văn bản read-only. Lượt đọc này hiển thị tối đa 500 dòng.
- Các định dạng ảnh được hỗ trợ như PNG, JPEG, GIF, WebP, BMP và ICO được giải mã và hiển thị dưới dạng ảnh.
- Với DOC, DOCX, XLS, XLSX, PPT hoặc PPTX, chọn **Render Office preview** để yêu cầu Host chuyển đổi. Ứng dụng hiển thị ảnh trang đầu tiên; lỗi chuyển đổi hoặc thiếu font được báo trong phần preview.
- Chọn tệp ở **Deliverables** để tự điền đường dẫn và mở preview tương ứng.

Trong **File actions**, **Open file…** mở hộp thoại chọn đường dẫn; ứng dụng dùng đường dẫn đó để yêu cầu Host đọc tệp trong phạm vi Session đang mở. **Save file…** ghi nội dung văn bản đang preview vào đường dẫn cục bộ bạn chọn; thao tác này không sửa tệp nguồn trên Host.

Preview cần có Session được chọn và Host đang kết nối. Nội dung đã tải có thể vẫn hiển thị khi refresh hoặc gặp lỗi; dòng trạng thái sẽ đánh dấu dữ liệu đó có thể đã cũ.

-----

## Changes, Git và lịch sử commit

Cột **Changes** đọc trạng thái Git của workspace đang chọn qua Host và chỉ hiển thị kết quả Host trả về: không có bước nào báo thành công trước khi Host xác nhận. Panel chỉ dock khi cửa sổ rộng tối thiểu 1180 px; hẹp hơn thì cột hội thoại được ưu tiên.

### Diff, Files và commit

Header của cột hiển thị số file thay đổi và chip ahead/behind. Nút **Review all** và trạng thái **Reviewed** chỉ lưu trong client; chúng tự đặt lại khi trạng thái Git của file đổi.

- Tab **Files** nhóm các đường dẫn thành **Staged / Changes / Untracked**, có thống kê thêm/bớt từng phía và ô lọc theo đường dẫn. Bấm một dòng để xem diff của nó; tick nhiều dòng để stage, unstage hoặc discard hàng loạt. Mỗi dòng chỉ hiện **Stage**, **Unstage**, **Discard** hay **Delete** khi thao tác đó hợp lệ với trạng thái Git hiện tại.
- Tab **Diff** có công tắc **Unstaged/Staged** và từng hunk với **Stage hunk**, **Unstage hunk** hoặc **Discard hunk** qua `git/apply`. Patch bị Host cắt bớt, tệp nhị phân, tệp untracked và tệp đang xung đột đều hiện lý do thay vì nút thao tác.
- Thanh commit ở cuối panel gồm ô **Commit message** (kèm **Recent** để điền lại thông điệp gần đây), **Amend** và **Sign-off** khi Host cho phép, rồi **Commit** và **Commit & Push**. Mọi thao tác xoá đều mở hộp xác nhận trước khi gọi Host.

### Lịch sử commit

Tab **History** đọc `git/log` theo trang 50 commit và có nút **Load older commits**. Graph swimlane do client tự vẽ: làn giữ cột cho tới khi nhánh merge lại, merge có đường cong cho cha phụ, ref hiện thành chip và HEAD có vòng tròn. Chọn commit bằng click hoặc phím **↑/↓** để xem chi tiết từ `git/show`; oid cha bấm được khi cha nằm trong trang đã tải, còn **Cherry-pick…** và **Revert…** hỏi xác nhận trước khi gọi `git/cherryPick` hoặc `git/revert`.

### Menu nhánh

Bấm branch ở thanh dưới cùng để mở menu nhánh: danh sách nhánh cục bộ (nhánh hiện tại có dấu ●), **Checkout**, **Merge**, **Rebase**, **Cherry-pick** cho nhánh khác và **Revert tip** cho nhánh hiện tại. HEAD tách rời được nêu rõ. Khi một verb Git đang chạy, mọi dòng bị tắt kèm lý do; checkout, merge, rebase, cherry-pick và revert đều hỏi xác nhận trước khi gọi Host.

### Menu đồng bộ

Bấm biểu tượng sync ở thanh dưới cùng để mở menu đồng bộ: **Fetch**, **Pull (fast-forward only)**, **Push**, **Push and set upstream** (publish nhánh chưa có upstream lên remote) và **Stash…**. Menu hiển thị branch, upstream và số commit ahead/behind; kết quả hoặc lỗi của Host hiện ngay dưới các dòng lệnh. Mọi dòng bị tắt khi Host offline, chưa chọn Session hoặc một thao tác Git khác đang chạy. **Push** và **Push and set upstream** hỏi xác nhận trước khi chạy và không bao giờ force-push; **Fetch** và **Pull** gửi thẳng tới Host.

### Stash

**Stash…** mở panel stash: nhập message tuỳ chọn, bật **Include untracked** nếu muốn mang cả tệp chưa theo dõi, rồi chọn **Save stash**. Danh sách entry có **Apply**, **Pop** và **Drop**; mỗi thao tác hỏi xác nhận và chỉ chạy khi entry vẫn còn trong danh sách Host trả về. Host giới hạn số entry nên panel ghi rõ khi danh sách bị cắt bớt.

### Xung đột

Khi Host báo merge, rebase, cherry-pick hoặc revert đang mở, banner trong panel nêu tên thao tác, số xung đột còn lại và hai nút **Continue**/**Abort**. **Continue** chỉ bật khi không còn xung đột. Bấm một đường dẫn xung đột để đọc ba stage **Base** (chỉ đọc), **Ours** và **Theirs**, rồi chọn **Accept ours** hoặc **Accept theirs** sau khi xác nhận; stage bị Host cắt bớt được ghi rõ để không bị nhầm là toàn bộ tệp.

### Xoá tệp untracked

Dòng untracked có nút **Delete**; thao tác này mở hộp xác nhận nêu đúng đường dẫn trước khi Host xoá tệp. Client chỉ xoá tệp: một đường dẫn kết thúc bằng `/` (thư mục untracked) bị từ chối kèm lý do.

-----

## Plugins, Skills và Marketplace

### Plugins

Màn hình **Plugins** tải inventory từ Host và hiển thị các plugin đã cấu hình, trạng thái bật/tắt, phase, bundles và thông tin agent preset.

Chọn **Enable** hoặc **Disable** để thay đổi plugin/bundle do Host quản lý. Một số mục không thể sửa ở đây và được đánh dấu read-only cùng lý do. Sau thay đổi, ứng dụng tải lại inventory để hiển thị trạng thái Host trả về.

### Skills

Màn hình **Skills** liệt kê các skill khả dụng cho Session đang chọn. Nhập từ khóa để lọc theo tên hoặc mô tả; chọn một skill để xem hướng dẫn áp dụng, trạng thái model-invocable và đường dẫn nguồn nếu có.

Màn hình này chỉ xem và lọc danh sách; nó không cài đặt, bật/tắt hoặc gọi skill trực tiếp. Agent có thể dùng skill trong composer nếu skill được đánh dấu khả dụng cho model.

### Marketplace

Màn hình **Market** tải các index đã cấu hình ở Host. Nhập từ khóa, chọn **Search** để xem kết quả và mô tả. Nếu không có index, việc tìm kiếm hoặc cài đặt không khả dụng cho đến khi Host được cấu hình.

Chọn **Install** ở mục có gói cài đặt để ứng dụng yêu cầu Host kiểm tra đích cài đặt rồi cài bundle vào profile. Host quyết định có chấp nhận cài đặt hay không; khi hoàn tất, ứng dụng hiển thị kết quả và làm mới danh sách Plugins. Cài đặt có thể thay đổi profile đang dùng.

-----

## Goals và Plan mode

Màn hình **Goals** làm việc với Goal của Session đang chọn. Chọn **Refresh** để tải trạng thái mới nhất.

- Khi chưa có Goal, nhập mục tiêu và có thể nhập số vòng tối đa; chọn **Create goal** để tạo.
- Với Goal đang tồn tại, sửa mục tiêu hoặc số vòng rồi chọn **Save changes**. Số vòng phải là số nguyên dương.
- Chọn **Pause** để tạm dừng; chọn **Resume** để tiếp tục Goal đang paused hoặc blocked.
- Chọn **Complete** để đánh dấu hoàn tất.
- Chọn **Clear** rồi xác nhận để gỡ Goal hiện tại khỏi Session. Lịch sử Goal vẫn nằm trong session log.

Phần **Recent tasks and plan changes** chiếu tối đa 12 hàng gần nhất thuộc nhóm Todo, Goal và Notice từ transcript; đây là phần xem lại, không phải trình chỉnh sửa Todo.

Chọn **Enter plan mode** hoặc **Leave plan mode** để gọi lần lượt lệnh `/plan` hoặc `/plan off` qua Host. Các nút này không hoạt động khi chat hoặc một lần đổi chế độ plan khác đang chạy.

-----

## Terminal, Jobs, Schedules và Deliverables

Các công cụ này nằm ở thanh tiện ích dưới cùng và gắn với Session đang chọn. Chọn biểu tượng một lần để mở panel, chọn lại hoặc nhấn × để đóng panel.

### Terminal

Chọn **Refresh terminals** để tải danh sách terminal; chọn **Create terminal** để mở shell mới trong Session. Chọn terminal rồi **Attach** để theo dõi snapshot và output trực tiếp.

Mặc định ô nhập gửi cả dòng khi bạn nhấn Enter. Bật **Raw keys** rồi bấm vào ô nhập của pane để gửi từng phím: ký tự, Enter, Backspace, Delete, ↑/↓/←/→, Home/End, PageUp/PageDown, Insert, F1–F12, **Esc**, và Ctrl+C khi không bôi đen chữ nào. Phím Esc cũng đóng lớp phủ đang mở như trước. Tab và các tổ hợp Ctrl/Alt khác do giao diện xử lý nên không đi tới terminal. Tắt Raw keys để quay lại chế độ gửi theo dòng.

**Add pane** mở thêm pane trong cùng Session, tối đa 4 pane; **Remove pane** chỉ hiện khi pane đang focus không phải pane đầu. Kéo vạch 4 px giữa hai pane để đổi độ rộng. Mỗi pane có Attach, Detach, Close và Rename riêng; bấm vào tiêu đề pane hoặc ô nhập của pane để chuyển focus.

Chọn output bằng cách bấm một dòng, giữ **Shift** và bấm dòng cuối để chọn cả khoảng; dòng đã chọn được tô nền. **Copy** sao chép đúng khoảng đã chọn vào clipboard; nếu chưa chọn dòng nào, pane báo cần chọn trước. **Detach** ngắt kết nối giao diện nhưng shell tiếp tục chạy và màn hình cuối được giữ lại; **Close** kết thúc shell của terminal đó. Nhãn lifecycle cho biết `idle`, `starting`, `attached`, `detached` hoặc `error`. Nếu terminal báo read-only, chờ trạng thái writable hoặc chọn Attach lại sau khi khôi phục kết nối.

### Jobs

**Jobs** liệt kê background jobs của Session. Chọn một job để mở luồng output và trạng thái trực tiếp; chọn refresh để tải lại danh sách. Panel này chỉ theo dõi output, không có nút dừng job.

### Schedules

**Schedules** liệt kê các lịch đã gắn với Session và thời gian chạy kế tiếp. Chọn refresh để tải lại. **Delete** xóa lịch được chọn; panel này không tạo hoặc chỉnh sửa lịch.

### Deliverables

**Deliverables** liệt kê các đường dẫn mà agent đã công bố là tệp đầu ra, kèm mô tả nếu có. Chọn một đường dẫn để mở trong Files preview. Chỉ tệp được agent công bố mới xuất hiện trong danh sách này.

-----

## Preferences và cập nhật Desktop

**Preferences** mở thành lớp phủ ngay trên shell đang chạy: mở bằng **Preferences…** trong menu hoặc bằng **Ctrl+,**. Lớp phủ rộng 910 px và cao 680 px (thu theo UI scale và giới hạn cửa sổ), có thanh tiêu đề với nút **Close**, và đóng bằng **Esc** hoặc bấm ra ngoài. Shell phía sau giữ nguyên trạng thái và vẫn hiển thị hai cột hai bên. Mỗi thay đổi được ghi ngay vào tệp tùy chọn cục bộ, nên không có nút Save riêng.

Cột điều hướng bên trái chia bảy mục: **General**, **Appearance**, **Keymap**, **Extensions**, **Skills**, **Marketplace** và **About**.

### General

**LAYOUT** bật/tắt **Workspace Explorer** và **Restore last Session**; nháp composer chưa gửi được giữ theo từng Session và lưu cùng client.

**WORKSPACE APPEARANCE** đặt ngoại lệ cho workspace đang chọn: bật/tắt Explorer và mật độ riêng, kèm **Clear override**/**Clear density** để quay lại giá trị chung. Giá trị trống nghĩa là workspace đó dùng cấu hình chung.

**DESKTOP UPDATES** và **HOST SETTINGS** nằm ngay dưới hai nhóm trên.

### Appearance

**COLOR** chọn **Theme** System/Light/Dark. **PACKS** liệt kê các pack đã cài: pack built-in (DSH, Harness, High contrast, Solarized), palette **Custom** cũ, pack người dùng đọc từ `$DSH_HOME/themes/*.json` và pack tải từ GitHub đọc từ `$DSH_HOME/themes/cache/*.json` (mặc định `$HOME/.deepseek-harness/themes`). Mỗi dòng ghi nguồn `builtin`, `user` hoặc `github`, trạng thái tương phản, nguồn gốc GitHub (URL và ref) và các nút **Use**, **Duplicate**, **Edit JSON**, **Export…**, **Remove**. **Add theme file…** chép một tệp JSON đã chọn vào thư mục themes dưới tên `<id>.json`; id trùng pack built-in bị từ chối, còn nếu đã có tệp người dùng cùng id thì phải bấm **Replace theme file** để xác nhận ghi đè. **Duplicate** tạo `<id>-copy.json` với đầy đủ mọi role và accent của pack nguồn. **Edit JSON** mở trình soạn thảo cho pack người dùng: **Save** kiểm tra schema, id và màu rồi ghi nguyên tử, **Cancel** bỏ thay đổi và giữ nguyên tệp. **Export…** lưu bản JSON đầy đủ ra vị trí bạn chọn. **Remove** xoá pack người dùng hoặc pack GitHub đã cache và cần bấm xác nhận lần hai; pack built-in luôn chỉ đọc. Nút bị tắt có chú thích nêu lý do. **ACCENT** chỉ áp dụng cho pack có bộ accent riêng (DSH và Harness); pack khác giữ accent của chính nó nên các nút Accent bị tắt kèm chú thích. **DENSITY** chọn Comfortable hoặc Compact. **SCALE** đổi UI scale từ 0.8 đến 1.5 theo bước 0.05 bằng hai nút − và +.

**ADD FROM GITHUB** nhận một URL `https` trỏ thẳng tới tệp `.json`, hoặc dạng `owner/repo`, `owner/repo@ref` cùng ô **Ref** và **Path** tuỳ chọn. Khi thiếu, ref mặc định là `main` và đường dẫn mặc định là `theme.json`; ví dụ `owner/repo@dev` với path `themes/forest.json` tải `https://raw.githubusercontent.com/owner/repo/dev/themes/forest.json`. Chỉ chấp nhận `https`, từ chối URL có thông tin đăng nhập nhúng và chuyển hướng sang giao thức khác; mỗi lần tải giới hạn 64 KiB và 10 giây. **Check** chỉ tải và kiểm tra, không ghi cache. **Import** tải, kiểm tra rồi mới ghi `cache/<id>.json` cùng `cache/<id>.meta.json` (sourceUrl, ref, etag, fetchedAt, sha256) bằng ghi nguyên tử. Không có lần tải nào lúc khởi động.

Pack đã cache hiển thị offline và xếp dưới tệp người dùng nhưng trên pack built-in cùng id. **Check for updates** gửi `If-None-Match` bằng ETag đã lưu: `304` là không đổi, còn tài liệu mới chỉ thay cache sau khi vượt qua kiểm tra. Nếu lần cập nhật lỗi hoặc tài liệu mới không hợp lệ, bản cache cũ vẫn dùng được và lý do hiện trong dòng trạng thái. Pack giao diện chỉ chứa màu: pack không đổi được metric, font, layout, kích thước điều khiển hay chạy mã. Pack người dùng và pack GitHub đều là nguồn không tin cậy: thiếu role, sai id/màu, hoặc không đạt ngưỡng tương phản bắt buộc sẽ hiện lý do và client quay về pack Harness thay vì vẽ một phần pack. Một tệp không đọc được chỉ hiện lỗi, không cản ứng dụng khởi động. File schema 5 dùng `themePack` vẫn được chuyển khi đọc: `default` giữ bảng màu Harness cũ, còn `high-contrast`/`solarized`/`custom` giữ nguyên tên.

### Keymap

**PRESET** chọn bộ phím DSH, Zed hoặc VS Code. Bên dưới là bốn tổ hợp phím đổi được: mở Settings, Marketplace, Goals và tạo Session. Dùng dạng `Ctrl+Shift+S`; nhập `none` để tắt. Khi lưu, client kiểm tra cả bốn phím tùy chỉnh lẫn các phím cố định của menu và command palette, và báo lỗi nếu hai chức năng trùng tổ hợp.

Tùy chọn giao diện được lưu ở tệp `native-ui-preferences.json` trong thư mục cấu hình của ứng dụng; pack người dùng nằm trong thư mục con `themes/` và cache GitHub nằm trong `themes/cache/` cạnh tệp đó. Khi `DSH_HOME` không được đặt, thư mục mặc định là `$HOME/.deepseek-harness`.

### Host settings

Trong **General**, mở **Host settings (agent and plugin configuration)** rồi chọn **Refresh settings** để đọc namespace, schema, revision và các giá trị đã che thông tin bí mật.

Nếu Host cho phép ghi, chọn **Edit user overrides** để sửa đối tượng JSON của user overrides rồi **Save settings**. Chỉ user overrides có thể được sửa từ editor này; các giá trị bí mật được che và không thể chỉnh trong editor. **Reset to defaults** trong editor đặt bản nháp overrides thành `{}`; bạn vẫn phải chọn Save để ghi thay đổi. **Discard draft** bỏ bản nháp.

Nếu lần lưu bị conflict hoặc không rõ kết quả, editor giữ lại bản nháp và chặn lần ghi tiếp theo. Làm mới Settings, xem giá trị hiện tại, rồi bỏ bản nháp cũ trước khi bắt đầu chỉnh sửa lại. Ứng dụng không tự gửi lại thao tác ghi.

### Cập nhật Desktop

Trong **General**, mở mục **Desktop updates** và chọn **Check for updates** khi Host được kết nối. Nếu Host yêu cầu xác nhận tải xuống hoặc cài đặt, đọc phiên bản và chọn **Download** hoặc **Install and restart**; chọn **Not now** để từ chối yêu cầu hiện tại. Host điều phối kiểm tra và thay thế bản chạy; ứng dụng native không tự cài một gói chưa được Host xác nhận.

-----

## Bàn phím, khay hệ thống và thoát ứng dụng

- **Ctrl+Enter** gửi prompt khi composer đang focus; Enter thêm dòng mới. Khi ô **Commit message** đang focus, **Ctrl+Enter** commit phần đã stage nếu Host cho phép.
- **Ctrl+P** mở **Quick open** (command palette). Gõ để lọc theo tên hoặc nhóm lệnh; **↑/↓** đổi mục đang chọn, **Enter** chạy mục đó, bấm chuột cũng chạy được. Palette còn liệt kê các lệnh `/` do Host đăng ký cho Session hiện tại.
- **Ctrl+N** tạo Session mới; **Ctrl+O** mở file; **Ctrl+Shift+E** ẩn/hiện Explorer; **Ctrl+Shift+C** ẩn/hiện Changes (giữ tab đang đọc); **Ctrl+Shift+D** mở Changes và chuyển thẳng sang tab **Diff**; **Ctrl+Shift+T** ẩn/hiện Terminal.
- Trong cột Changes: **Alt+↑/↓** chọn hunk trước/sau ở tab Diff (hunk đang chọn có viền màu accent). **s** stage, **u** unstage và **d** discard: ở tab Files áp dụng cho nhóm đã tick, nếu không có thì cho file đang chọn; ở tab Diff áp dụng cho hunk đang chọn. Discard vẫn mở hộp xác nhận như khi bấm nút.
- Các phím **s**/**u**/**d** và **Alt+↑/↓** không chạy khi con trỏ đang ở ô nhập văn bản hoặc composer, và chỉ chạy khi cột Changes đang mở; riêng **Ctrl+Enter** chỉ tác dụng ở ô **Commit message**.
- **Ctrl+,** mở/đóng **Preferences**, **Ctrl+Shift+M** mở/đóng Marketplace, **Ctrl+Shift+G** mở/đóng Goals, **Ctrl+Shift+X** mở Extensions và **Ctrl+Shift+K** mở Skills. Chỉ bốn phím đầu nhóm này (Preferences, Marketplace, Goals, New session) đổi được trong Preferences; các phím còn lại cố định.
- **Esc** đóng lớp phủ trên cùng: hộp xác nhận, thông báo cập nhật, popover Sessions, menu chính, Quick open, rồi hộp About — mỗi lần một lớp, không đóng hết cùng lúc.
- Menu chính hiển thị tổ hợp phím hiện hành của từng lệnh, nên sau khi đổi phím trong Preferences, menu và Quick open luôn khớp với bộ điều phối bàn phím.
- Phím mũi tên lên/xuống điều hướng danh sách Sessions trong popover khi không nhập văn bản.

Nếu bản cài đặt có khay hệ thống, đóng cửa sổ sẽ ẩn ứng dụng xuống khay thay vì thoát. Chọn **Show/Hide** để hiện/ẩn cửa sổ; chọn **Quit** để thoát hoàn toàn. Nếu khay hệ thống không khả dụng, nút đóng cửa sổ dùng hành vi đóng cửa sổ thông thường.

-----

## Dữ liệu và khắc phục sự cố

Sessions, transcript, model selection, workspace, plugin state, Goals, terminal, jobs và schedules thuộc Host hoặc Session đang chọn. Tùy chọn giao diện thuộc tệp cục bộ của client. Tệp đính kèm được đọc trên máy rồi tải lên Host khi bạn chọn Attach; nội dung prompt tham chiếu đến bản upload đó.

| Tình trạng | Việc nên làm |
|---|---|
| Host disconnected hoặc không có nút tạo Session | Chọn **Retry connection**. Nếu Host không khởi động, liên hệ người quản trị bản cài đặt. |
| Không có model provider hoặc catalog lỗi | Chọn tải lại catalog; nếu vẫn rỗng, nhờ người quản trị cấu hình provider ở Host. |
| Tìm kiếm Session không có kết quả | Xóa hoặc thu hẹp truy vấn rồi tìm lại; refresh danh sách để thoát chế độ tìm kiếm. |
| Upload tệp thất bại | Kiểm tra kết nối và chọn lại tệp; chỉ gửi prompt sau khi upload hoàn tất. |
| Explorer không đọc được tệp | Xác nhận tệp là văn bản UTF-8 và thuộc workspace của Session; dùng Files preview cho định dạng ảnh hoặc Office được hỗ trợ. |
| Office preview thất bại | Thử lại khi Host kết nối; nếu báo thiếu font hoặc lỗi chuyển đổi, yêu cầu người quản trị kiểm tra dịch vụ preview và công cụ chuyển đổi. |
| Settings báo conflict hoặc kết quả không rõ | Refresh, kiểm tra giá trị hiện tại, discard bản nháp cũ rồi chỉnh sửa lại. Không gửi lại mù quáng. |
| Terminal không cho nhập | Kiểm tra trạng thái writable, chọn Attach hoặc refresh danh sách; Detach không dừng shell. |
| Marketplace không có index hoặc từ chối cài đặt | Host cần có index phù hợp và phải chấp nhận đích cài đặt; refresh index rồi hỏi người quản trị nếu vẫn lỗi. |
| Cửa sổ thứ hai không xuất hiện khi mở ứng dụng lần nữa | Ứng dụng chỉ chạy một instance; kiểm tra khay hệ thống để hiện cửa sổ đang chạy. |

Các giới hạn giao diện và trạng thái được kiểm thử ở [native-app tests](tests/) và giao thức ở [`client-core/API.md`](../client-core/API.md). Tài liệu giao thức dành cho người phát triển, không thay thế hướng dẫn này.

**Phạm vi xác minh:** Tài liệu này được đối chiếu với mã nguồn và tests; nó không xác nhận cấu hình của một bản cài đặt cụ thể. Người quản trị Desktop profile cần kiểm tra provider, quyền công cụ, registry Marketplace, dịch vụ Office preview và update feed trong môi trường triển khai.
