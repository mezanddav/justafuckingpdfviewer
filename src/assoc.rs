//! Registers the viewer as a PDF handler for the current user, so it shows up under
//! "Open with" and in Settings > Default apps, with the app icon on PDF files.
//!
//! Windows doesn't let programs make themselves the default; the user picks it once.
//! Registering again on each launch keeps the entries pointing at wherever the exe now lives.

use std::ffi::c_void;

const PROG_ID: &str = "justafuckingpdfviewer.pdf";
const APP_KEY: &str = r"Software\justafuckingpdfviewer";

// HKEY_CURRENT_USER is the sign-extended constant 0x80000001.
const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
const REG_SZ: u32 = 1;
const RRF_RT_REG_SZ: u32 = 0x2;
const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;

#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegSetKeyValueW(
        key: isize,
        sub_key: *const u16,
        value: *const u16,
        ty: u32,
        data: *const c_void,
        len: u32,
    ) -> i32;
    fn RegGetValueW(
        key: isize,
        sub_key: *const u16,
        value: *const u16,
        flags: u32,
        ty: *mut u32,
        data: *mut c_void,
        len: *mut u32,
    ) -> i32;
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHChangeNotify(event: i32, flags: u32, item1: *const c_void, item2: *const c_void);
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn set(key: &str, value: &str, data: &str) {
    let data = wide(data);
    // SAFETY: all pointers come from live, NUL-terminated buffers.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            wide(value).as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        );
    }
}

fn get(key: &str, value: &str) -> Option<String> {
    let mut buf = vec![0u16; 2048];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: `len` holds the size in bytes of `buf`, which RegGetValueW won't exceed.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(key).as_ptr(),
            wide(value).as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    (status == 0).then(|| String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]))
}

pub fn register() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let exe = exe.display().to_string();
    let command = format!("\"{exe}\" \"%1\"");
    let command_key = format!(r"Software\Classes\{PROG_ID}\shell\open\command");
    if get(&command_key, "").as_deref() == Some(command.as_str()) {
        return;
    }

    set(&format!(r"Software\Classes\{PROG_ID}"), "", "PDF Document");
    set(
        &format!(r"Software\Classes\{PROG_ID}\DefaultIcon"),
        "",
        &format!("\"{exe}\",0"),
    );
    set(&command_key, "", &command);
    set(r"Software\Classes\.pdf\OpenWithProgids", PROG_ID, "");
    set(
        r"Software\Classes\Applications\justafuckingpdfviewer.exe\shell\open\command",
        "",
        &command,
    );
    set(
        r"Software\Classes\Applications\justafuckingpdfviewer.exe\SupportedTypes",
        ".pdf",
        "",
    );
    set(
        &format!(r"{APP_KEY}\Capabilities"),
        "ApplicationName",
        "justafuckingpdfviewer",
    );
    set(
        &format!(r"{APP_KEY}\Capabilities"),
        "ApplicationDescription",
        "Just a PDF viewer",
    );
    set(
        &format!(r"{APP_KEY}\Capabilities\FileAssociations"),
        ".pdf",
        PROG_ID,
    );
    set(
        r"Software\RegisteredApplications",
        "justafuckingpdfviewer",
        &format!(r"{APP_KEY}\Capabilities"),
    );

    // SAFETY: SHCNE_ASSOCCHANGED takes no items.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, 0, std::ptr::null(), std::ptr::null()) };
}
