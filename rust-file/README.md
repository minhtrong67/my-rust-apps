# Rust File

Trình quản lý tệp **Rust thuần, chạy native** – viết lại hoàn toàn từ Fluent File (Tauri + WebView).
Không WebView, không Electron, không Node, không JavaScript: một file `.exe` duy nhất.

A pure-Rust, native file manager – a full rewrite of Fluent File (Tauri + WebView).
No WebView, no Electron, no Node: one single executable.

- GUI: [`egui`/`eframe`](https://github.com/emilk/egui) (OpenGL, immediate mode) – khởi động nhanh, RAM thấp
- Mọi thao tác file chạy trên **thread riêng** (sao chép, di chuyển, tìm kiếm, quét trùng lặp, thống kê dung lượng, giải nén…) nên giao diện không bao giờ bị đứng
- Quét trùng lặp & thống kê dung lượng **đa luồng** trên mọi nhân CPU

## Build (Windows)

```powershell
# cần Rust >= 1.85 (https://rustup.rs) và Visual Studio Build Tools (C++)
cargo build --release
# -> target\release\rust-file.exe
```

Chạy thử: `cargo run --release` hoặc `rust-file.exe D:\Duong\Dan` (mở thẳng một thư mục).
Cũng build được trên Linux/macOS (để dev), nhưng một số tính năng chỉ có trên Windows (xem bên dưới).

## Tính năng

**Duyệt tệp**
- Nhiều tab, 1 hoặc 2 khung (F9), lịch sử Back/Forward (kể cả nút chuột bên), breadcrumb có menu thư mục con
- Danh sách chi tiết (ảo hóa – mượt với hàng trăm nghìn tệp) và lưới biểu tượng nhỏ/vừa/lớn có **thumbnail ảnh**
- Sắp xếp tự nhiên (`file2` < `file10`), lọc theo tên/loại/thẻ màu, nhớ kiểu xem + sắp xếp **theo từng thư mục**
- Tự làm mới khi thư mục thay đổi (file watcher), có debounce
- Sidebar: This PC, Truy cập nhanh (ghim thư mục), ổ đĩa có thanh dung lượng, thẻ màu

**Thao tác**
- Sao chép / cắt / dán có thanh tiến trình + nút Hủy, hộp thoại trùng tên (Thay thế / Giữ cả hai / Bỏ qua)
- Kéo-thả giữa các khung, vào thư mục, sidebar; thả tệp từ Explorer vào cửa sổ để sao chép (giữ Ctrl khi kéo = sao chép)
- Xóa vào Thùng rác / xóa vĩnh viễn, **Hoàn tác** (Ctrl+Z) cho đổi tên, di chuyển, tạo mới, xóa vào Thùng rác
- Đổi tên inline (F2), **đổi tên hàng loạt** (mẫu `{name} {n:3} {date}`, tìm & thay có regex, ngẫu nhiên, đổi hoa/thường) có xem trước
- Nén **ZIP gốc bằng Rust** (không cần phần mềm ngoài), giải nén ZIP có chống zip-slip; RAR/7z dùng WinRAR/7-Zip nếu đã cài
- Gắn sao, thẻ màu, màu thư mục, ghim, đặt làm hình nền, lệnh tùy chỉnh trong menu chuột phải

**Công cụ**
- Tìm kiếm đệ quy (có thể hủy), tìm nâng cao: regex, đuôi tệp, kích thước, ngày sửa; lịch sử & tìm kiếm đã lưu
- Tìm tệp trùng lặp (kích thước → hash 64 KB → hash đầy đủ, đa luồng)
- Dung lượng ổ đĩa dạng **treemap** (nhấn để đi sâu vào thư mục)
- Bảng lệnh Ctrl+K, xem nhanh (Space: ảnh có zoom/kéo, văn bản), ngăn chi tiết (Alt+P) có xem trước văn bản
- Dọn tệp tạm, công cụ hệ thống Windows (Task Manager, Device Manager, …)
- Giao diện sáng/tối/theo hệ thống, màu nhấn (tự lấy màu nhấn Windows), cỡ chữ, Tiếng Việt / English

## Phím tắt

| Phím | Chức năng | Phím | Chức năng |
|---|---|---|---|
| Ctrl+K | Bảng lệnh | F2 | Đổi tên (nhiều tệp = đổi hàng loạt) |
| Ctrl+T / Ctrl+W | Tab mới / đóng tab | Delete / Shift+Delete | Xóa vào Thùng rác / vĩnh viễn |
| Ctrl+N | Cửa sổ mới | Ctrl+C / X / V | Sao chép / cắt / dán |
| Ctrl+L | Sửa thanh địa chỉ | Ctrl+Z | Hoàn tác |
| Ctrl+F | Ô tìm kiếm | Ctrl+Shift+F | Tìm kiếm nâng cao |
| Ctrl+Shift+N | Thư mục mới | Ctrl+Shift+C | Sao chép đường dẫn |
| Space | Xem nhanh | Alt+Enter | Thuộc tính |
| Alt+← → ↑ / Backspace | Back / Forward / Lên | F5 | Làm mới |
| F9 | Hai khung | Alt+P | Ngăn chi tiết |
| Ctrl+1 / Ctrl+2 / Ctrl+lăn chuột | Kiểu xem | F11 | Toàn màn hình |
| Ctrl+, | Cài đặt | Ctrl+A / Esc | Chọn tất cả / bỏ chọn |

## Khác biệt so với bản Tauri

Không còn WebView nên một số hiệu ứng phụ thuộc web đã được thay bằng bản native:

- Không còn hiệu ứng Mica/Acrylic/Liquid-Glass và thanh tiêu đề tự vẽ (dùng khung cửa sổ hệ thống)
- Xem nhanh hỗ trợ **ảnh + văn bản**; video/âm thanh/PDF → nút "Mở bằng ứng dụng mặc định"
- "Thiết kế thư mục" bộ logo 3000+ biểu tượng được thay bằng chọn **màu thư mục**
- Khay hệ thống (tray), phím tắt toàn cục và trình cài menu chuột phải của Explorer chưa có
- Clipboard sao chép/cắt là **nội bộ trong app** (không dán sang Explorer bằng Ctrl+V)
- Thumbnail ảnh giữ trong RAM (tối đa 400 ảnh), chưa có cache trên đĩa

## Cấu trúc mã nguồn

```
src/main.rs      cửa sổ, vòng lặp chính
src/app.rs       trạng thái, điều hướng, luồng nền, sao chép/xóa/hoàn tác
src/fsops.rs     toàn bộ thao tác file (có unit test: cargo test)
src/ui.rs        thanh tab/địa chỉ, sidebar, danh sách, lưới, ngăn chi tiết, phím tắt
src/dialogs.rs   hộp thoại, bảng lệnh, treemap
src/icons.rs     icon Fluent (font hệ thống) + icon vẽ bằng vector
src/settings.rs  cài đặt (JSON trong %APPDATA%\rust-file\settings.json)
src/i18n.rs      Tiếng Việt / English
```

MIT License.
