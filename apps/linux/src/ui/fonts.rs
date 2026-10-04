//! The prompter's typeface, Atkinson Hyperlegible Next, registered with
//! fontconfig for this process so it needs no installing.

use std::ffi::{c_char, c_int, c_void, CString};
use std::os::unix::ffi::OsStrExt;

const FILES: [(&str, &[u8]); 2] = [
    (
        "AtkinsonHyperlegibleNext[wght].ttf",
        include_bytes!("../../../fonts/AtkinsonHyperlegibleNext[wght].ttf"),
    ),
    (
        "AtkinsonHyperlegibleNext-Italic[wght].ttf",
        include_bytes!("../../../fonts/AtkinsonHyperlegibleNext-Italic[wght].ttf"),
    ),
];

#[link(name = "fontconfig")]
extern "C" {
    fn FcConfigAppFontAddFile(config: *mut c_void, file: *const c_char) -> c_int;
}

/// Makes the typeface available to GTK. Call before the first window; if it
/// fails, text falls back to the system's sans-serif.
pub fn register() {
    let dir = gtk::glib::user_cache_dir().join("teleprompt/fonts");
    let _ = std::fs::create_dir_all(&dir);
    for (name, bytes) in FILES {
        let path = dir.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(bytes)
            && std::fs::write(&path, bytes).is_err()
        {
            continue;
        }
        if let Ok(path) = CString::new(path.as_os_str().as_bytes()) {
            // SAFETY: a null config means the current one; the path is a
            // valid C string that outlives the call.
            unsafe { FcConfigAppFontAddFile(std::ptr::null_mut(), path.as_ptr()) };
        }
    }
}
