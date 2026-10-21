#!/bin/sh
# Installs justafuckingpdfviewer for the current user and makes it the default PDF viewer.
# Run from the unpacked release folder: ./install.sh
set -e
cd "$(dirname "$0")"
mkdir -p ~/.local/bin ~/.local/share/applications ~/.local/share/icons/hicolor/256x256/apps
install -m 755 justafuckingpdfviewer ~/.local/bin/
install -m 644 justafuckingpdfviewer.desktop ~/.local/share/applications/
install -m 644 justafuckingpdfviewer.png ~/.local/share/icons/hicolor/256x256/apps/
command -v update-desktop-database >/dev/null && update-desktop-database ~/.local/share/applications || true
command -v xdg-mime >/dev/null && xdg-mime default justafuckingpdfviewer.desktop application/pdf || true
echo "Installed. Make sure ~/.local/bin is on your PATH."
