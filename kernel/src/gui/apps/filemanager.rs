/// File Manager — lists VFS root directory
use crate::gui::wm;

pub fn populate(win_idx: usize) {
    wm::write_to_window(win_idx, b"=== File Manager ===\r\n\r\n");

    let mut entries = [crate::vfs::DirEntry {
        name: [0u8; crate::vfs::MAX_NAME], is_dir: false, size: 0,
    }; 32];
    let n = crate::vfs::readdir(b"/", &mut entries);

    if n == 0 {
        wm::write_to_window(win_idx, b"(empty)\r\n");
    } else {
        for i in 0..n as usize {
            let e = &entries[i];
            if e.is_dir {
                wm::write_to_window(win_idx, b"  [DIR]  ");
            } else {
                wm::write_to_window(win_idx, b"  [FILE] ");
            }
            let nlen = e.name.iter().position(|&c| c == 0).unwrap_or(32);
            wm::write_to_window(win_idx, &e.name[..nlen]);
            if !e.is_dir {
                wm::write_to_window(win_idx, b"  (");
                let mut buf = [0u8; 12];
                let sz = e.size;
                let mut tmp = [0u8; 10]; let mut t = 0;
                if sz == 0 { tmp[0] = b'0'; t = 1; }
                let mut v = sz;
                while v > 0 { tmp[t] = b'0' + (v % 10) as u64 as u8; v /= 10; t += 1; }
                let mut j = 0;
                while t > 0 { t -= 1; buf[j] = tmp[t]; j += 1; }
                wm::write_to_window(win_idx, &buf[..j]);
                wm::write_to_window(win_idx, b" bytes)");
            }
            wm::write_to_window(win_idx, b"\r\n");
        }
    }

    wm::write_to_window(win_idx, b"\r\nDouble-click to open.\r\n");
}
