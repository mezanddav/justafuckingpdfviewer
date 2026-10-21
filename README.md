<p align="center"><img src="packaging/icon-256.png" width="128" alt=""></p>

# justafuckingpdfviewer

A PDF viewer that just shows the PDF. No accounts, no toolbars, no updater, no cloud.
It opens in about a tenth of a second and gets out of your way.

- **Fast.** The first page is on screen ~110 ms after you double-click. Pages render on several
  threads in the background.
- **One file.** A single self-contained executable. Nothing to install, nothing to configure.
- **Pure Rust.** PDF rendering by [hayro](https://github.com/LaurenzV/hayro), UI by
  [egui](https://github.com/emilk/egui). No C/C++ PDF engine.
- **Windows, macOS and Linux** (x64 and ARM).

## Download

Grab the file for your system from [Releases](https://github.com/mezanddav/justafuckingpdfviewer/releases).
Each one is the whole app; there is nothing to install.

| System | File |
| --- | --- |
| Windows (most PCs) | `justafuckingpdfviewer-windows-x64.exe` |
| Windows on ARM | `justafuckingpdfviewer-windows-arm64.exe` |
| macOS (Apple Silicon and Intel) | `justafuckingpdfviewer-macos.zip` |
| Linux (most PCs) | `justafuckingpdfviewer-linux-x86_64` |
| Linux on ARM (Raspberry Pi and friends) | `justafuckingpdfviewer-linux-aarch64` |

- **Windows:** run the `.exe`. To open PDFs with it by default: right-click a PDF → *Open with*
  → *Choose another app* → *justafuckingpdfviewer* → *Always*.
- **macOS:** unzip and move the app to Applications. The app isn't notarized yet, so the
  first time, right-click it → *Open*. Open PDFs by dropping them on the window, or with
  `open -a justafuckingpdfviewer file.pdf`.
- **Linux:** make it executable (`chmod +x justafuckingpdfviewer-linux-*`) and run it once.
  It adds itself to your app menu and to *Open with* for PDFs. To make it the default:
  `xdg-mime default justafuckingpdfviewer.desktop application/pdf`.

On Windows and Linux the app re-registers itself each time it starts, so if you move the file,
just run it once from its new place.

## Keys

Press **H** in the app to see these.

| Key | Does |
| --- | --- |
| Wheel / drag | Move around |
| Keep scrolling | Scrolls faster and faster |
| Flick and let go | Page keeps gliding |
| Hold right/middle button | Joystick: page flies toward the mouse |
| Shift + wheel | Scroll sideways |
| Ctrl + wheel | Zoom at the mouse |
| S | Whole page / actual size |
| Shift + S | Scroll page by page / continuously |
| Space | Reset view: whole page, this page |
| Arrows, PgUp/PgDn | Scroll |
| Home / End | First / last page |
| Drag over text | Select it |
| Ctrl + C | Copy selected text |
| F | Fullscreen on / off |
| A | Page previews on / off |
| Click a preview | Go to that page |
| I | File info |
| H | Hotkey list |
| X / Esc | Quit |
| Drop a PDF | Open it |

On macOS, use ⌘ where it says Ctrl.

## Build from source

You need [Rust](https://rustup.rs) (stable).

```sh
cargo build --release
```

The binary ends up in `target/release/`. On Linux, install the windowing headers first:

```sh
sudo apt-get install libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev
```

On Windows, the build embeds the app icon, which needs `rc.exe` from the Windows SDK (it comes
with the Visual Studio C++ build tools that Rust already asks for).

## Contributing

Issues and pull requests are welcome. The one rule: **nothing may make it slower to open or
to use.** New features stay off the hot path (the page previews, for example, only render
while they are shown).

Before sending a PR:

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

## Known gaps

- Password-protected PDFs aren't supported yet.
- On macOS, double-clicking a PDF in Finder doesn't pass it to the app yet; drop it on the
  window instead.
- hayro is young; an unusual PDF may render imperfectly. Please open an issue with the file
  if you can share it.

## License

[MIT](LICENSE): use it, copy it, change it, sell it, ship it in your fridge. Just keep the
license notice.
