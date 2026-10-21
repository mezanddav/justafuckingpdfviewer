//! Adds the viewer to the desktop's app list for the current user, with its icon and as an
//! "Open with" choice for PDFs, so the single downloaded binary needs no installer.
//!
//! Like on Windows, it doesn't make itself the default; the user picks it once. Registering
//! again on each launch keeps the entry pointing at wherever the binary now lives.

use std::path::PathBuf;

const ICON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_256.png"));

pub fn register() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(data) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    else {
        return;
    };

    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=justafuckingpdfviewer\n\
         Comment=Just a PDF viewer\n\
         Exec=\"{}\" %f\n\
         Icon=justafuckingpdfviewer\n\
         MimeType=application/pdf;\n\
         Categories=Office;Viewer;\n\
         Terminal=false\n",
        exe.display()
    );
    let apps = data.join("applications");
    let entry_path = apps.join("justafuckingpdfviewer.desktop");
    if std::fs::read_to_string(&entry_path).is_ok_and(|current| current == entry) {
        return;
    }

    let icons = data.join("icons/hicolor/256x256/apps");
    let written = std::fs::create_dir_all(&apps)
        .and_then(|_| std::fs::create_dir_all(&icons))
        .and_then(|_| std::fs::write(icons.join("justafuckingpdfviewer.png"), ICON))
        .and_then(|_| std::fs::write(&entry_path, entry));
    if written.is_ok() {
        // Refresh the desktop's MIME cache if the tool is around; harmless if not.
        let _ = std::process::Command::new("update-desktop-database")
            .arg(&apps)
            .status();
    }
}
