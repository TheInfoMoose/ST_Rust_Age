#!/bin/bash
set -e

echo "Packaging Simply Transfer v1.0.0 Production..."

cd "$(dirname "$0")"

if [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
fi

cargo build --release -p simply-transfer-ui

OS_TYPE="$(uname -s)"

if [ "$OS_TYPE" = "Darwin" ]; then
    APP_BUNDLE="dist/SimplyTransfer.app"
    rm -rf "$APP_BUNDLE"
    mkdir -p "$APP_BUNDLE/Contents/MacOS"
    mkdir -p "$APP_BUNDLE/Contents/Resources"

    cp target/release/simply-transfer-ui "$APP_BUNDLE/Contents/MacOS/simply-transfer"
    chmod +x "$APP_BUNDLE/Contents/MacOS/simply-transfer"

    if [ -f "assets/logo.icns" ]; then
        cp assets/logo.icns "$APP_BUNDLE/Contents/Resources/AppIcon.icns"
    fi

    cat << 'EOF' > "$APP_BUNDLE/Contents/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>simply-transfer</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>com.simplytransfer.app</string>
    <key>CFBundleName</key>
    <string>Simply Transfer</string>
    <key>CFBundleDisplayName</key>
    <string>Simply Transfer</string>
    <key>CFBundleVersion</key>
    <string>1.0.0</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

    cat << 'EOF' > "dist/entitlements.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.network.client</key>
    <true/>
    <key>com.apple.security.network.server</key>
    <true/>
    <key>com.apple.security.files.user-selected.read-write</key>
    <true/>
</dict>
</plist>
EOF

    SIGN_IDENTITY="${MACOS_SIGN_IDENTITY:--}"
    codesign --force --deep --options runtime --entitlements "dist/entitlements.plist" --sign "$SIGN_IDENTITY" "$APP_BUNDLE"
    echo "Signed macOS bundle: $APP_BUNDLE"
else
    DIST_DIR="dist/SimplyTransfer-Linux-x64"
    APPDIR="dist/SimplyTransfer.AppDir"
    rm -rf "$DIST_DIR" "$APPDIR"
    mkdir -p "$DIST_DIR"
    mkdir -p "$APPDIR/usr/bin"
    mkdir -p "$APPDIR/usr/share/applications"
    mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"

    cp target/release/simply-transfer-ui "$DIST_DIR/simply-transfer"
    cp target/release/simply-transfer-ui "$APPDIR/usr/bin/simply-transfer"

    if [ -f "assets/logo.png" ]; then
        cp assets/logo.png "$DIST_DIR/simply-transfer.png"
        cp assets/logo.png "$APPDIR/simply-transfer.png"
        cp assets/logo.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/simply-transfer.png"
    fi

    cat << 'EOF' > "$DIST_DIR/simply-transfer.desktop"
[Desktop Entry]
Version=1.0
Type=Application
Name=Simply Transfer
GenericName=P2P File Transfer
Comment=Enterprise-grade P2P encrypted file transfer and snapshot sync utility
Exec=simply-transfer %U
Icon=simply-transfer
Terminal=false
Categories=Network;FileTransfer;
StartupWMClass=simply-transfer
Keywords=transfer;p2p;encrypted;sync;share;
EOF

    cp "$DIST_DIR/simply-transfer.desktop" "$DIST_DIR/SimplyTransfer.desktop"
    cp "$DIST_DIR/simply-transfer.desktop" "$APPDIR/simply-transfer.desktop"
    cp "$DIST_DIR/simply-transfer.desktop" "$APPDIR/usr/share/applications/simply-transfer.desktop"

    cat << 'EOF' > "$DIST_DIR/install.sh"
#!/bin/sh
set -e

DIR="$(cd "$(dirname "$0")" && pwd)"
ACTION="install"

if [ "$1" = "--uninstall" ] || [ "$1" = "-u" ]; then
    ACTION="uninstall"
elif [ "$1" = "--portable" ] || [ "$1" = "-p" ]; then
    ACTION="portable"
elif [ "$1" = "--system" ] || [ "$1" = "-s" ]; then
    ACTION="system"
fi

if [ "$ACTION" = "uninstall" ]; then
    echo "Uninstalling Simply Transfer..."
    rm -f "${HOME}/.local/bin/simply-transfer"
    rm -f "${HOME}/.local/share/applications/simply-transfer.desktop"
    rm -f "${HOME}/.local/share/applications/SimplyTransfer.desktop"
    rm -f "${HOME}/.local/share/icons/hicolor/256x256/apps/simply-transfer.png"
    rm -f "${HOME}/.local/share/icons/simply-transfer.png"
    if [ "$(id -u)" = "0" ]; then
        rm -f "/usr/local/bin/simply-transfer"
        rm -f "/usr/share/applications/simply-transfer.desktop"
        rm -f "/usr/share/applications/SimplyTransfer.desktop"
        rm -f "/usr/share/icons/hicolor/256x256/apps/simply-transfer.png"
        rm -f "/usr/share/pixmaps/simply-transfer.png"
    fi
    echo "Simply Transfer uninstalled successfully."
    exit 0
fi

if [ "$ACTION" = "system" ]; then
    if [ "$(id -u)" != "0" ]; then
        echo "Error: System-wide install requires root privileges (run with sudo)."
        exit 1
    fi
    BIN_DIR="/usr/local/bin"
    APP_DIR="/usr/share/applications"
    ICON_DIR="/usr/share/icons/hicolor/256x256/apps"
    PIXMAP_DIR="/usr/share/pixmaps"
    mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR" "$PIXMAP_DIR"
    cp -f "$DIR/simply-transfer" "$BIN_DIR/simply-transfer"
    chmod +x "$BIN_DIR/simply-transfer"
    cp -f "$DIR/simply-transfer.png" "$ICON_DIR/simply-transfer.png"
    cp -f "$DIR/simply-transfer.png" "$PIXMAP_DIR/simply-transfer.png"
    cp -f "$DIR/simply-transfer.desktop" "$APP_DIR/simply-transfer.desktop"
    chmod 644 "$APP_DIR/simply-transfer.desktop"
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$APP_DIR" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -f -t /usr/share/icons/hicolor || true
    fi
    echo "Simply Transfer system-wide installation complete!"
    exit 0
fi

if [ "$ACTION" = "portable" ]; then
    APP_DIR="${HOME}/.local/share/applications"
    ICON_DIR="${HOME}/.local/share/icons/hicolor/256x256/apps"
    FALLBACK_ICON_DIR="${HOME}/.local/share/icons"
    mkdir -p "$APP_DIR" "$ICON_DIR" "$FALLBACK_ICON_DIR"
    cp -f "$DIR/simply-transfer.png" "$ICON_DIR/simply-transfer.png"
    cp -f "$DIR/simply-transfer.png" "$FALLBACK_ICON_DIR/simply-transfer.png"
    cat << EOF2 > "$APP_DIR/simply-transfer.desktop"
[Desktop Entry]
Version=1.0
Type=Application
Name=Simply Transfer
GenericName=P2P File Transfer
Comment=Enterprise-grade P2P encrypted file transfer and snapshot sync utility
Exec=$DIR/simply-transfer %U
Icon=$DIR/simply-transfer.png
Terminal=false
Categories=Network;FileTransfer;
StartupWMClass=simply-transfer
Keywords=transfer;p2p;encrypted;sync;share;
EOF2
    chmod +x "$APP_DIR/simply-transfer.desktop"
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$APP_DIR" || true
    fi
    echo "Portable shortcut registered in ~/.local/share/applications/ pointing to $DIR/simply-transfer"
    exit 0
fi

# Default: User installation to ~/.local
BIN_DIR="${HOME}/.local/bin"
APP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/256x256/apps"
FALLBACK_ICON_DIR="${HOME}/.local/share/icons"

mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR" "$FALLBACK_ICON_DIR"
cp -f "$DIR/simply-transfer" "$BIN_DIR/simply-transfer"
chmod +x "$BIN_DIR/simply-transfer"
cp -f "$DIR/simply-transfer.png" "$ICON_DIR/simply-transfer.png"
cp -f "$DIR/simply-transfer.png" "$FALLBACK_ICON_DIR/simply-transfer.png"
cp -f "$DIR/simply-transfer.desktop" "$APP_DIR/simply-transfer.desktop"
chmod +x "$APP_DIR/simply-transfer.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APP_DIR" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "${HOME}/.local/share/icons/hicolor" || true
fi

echo "Simply Transfer successfully installed to ~/.local!"
echo "You can launch Simply Transfer from your application launcher or run 'simply-transfer' in your terminal."
EOF

    cat << 'EOF' > "$DIST_DIR/README.txt"
Simply Transfer - Linux Release Package
=======================================

Usage options:

1. One-Click User Install:
   Run: ./install.sh
   Installs the binary to ~/.local/bin, desktop entry to ~/.local/share/applications,
   and icon to ~/.local/share/icons.

2. Run From Archive / Portable:
   Run: ./simply-transfer
   To add to your application menu without moving files:
   Run: ./install.sh --portable

3. Manual Desktop Integration:
   Copy 'simply-transfer' to your PATH (e.g. ~/.local/bin/ or /usr/local/bin/)
   Copy 'simply-transfer.png' to ~/.local/share/icons/hicolor/256x256/apps/
   Drop 'simply-transfer.desktop' into ~/.local/share/applications/

4. System-Wide Install (all users):
   Run: sudo ./install.sh --system

5. Uninstall:
   Run: ./install.sh --uninstall
EOF

    cat << 'EOF' > "$APPDIR/AppRun"
#!/bin/sh
HERE="$(dirname "$(readlink -f "${0}")")"
exec "${HERE}/usr/bin/simply-transfer" "$@"
EOF

    chmod +x "$DIST_DIR/simply-transfer" "$DIST_DIR/simply-transfer.desktop" "$DIST_DIR/SimplyTransfer.desktop" "$DIST_DIR/install.sh" "$APPDIR/AppRun" "$APPDIR/usr/bin/simply-transfer"

    if command -v appimagetool >/dev/null 2>&1; then
        ARCH=x86_64 appimagetool "$APPDIR" "dist/SimplyTransfer-x86_64.AppImage"
        chmod +x "dist/SimplyTransfer-x86_64.AppImage"
        echo "Generated AppImage: dist/SimplyTransfer-x86_64.AppImage"
    fi

    sha256sum "$DIST_DIR/simply-transfer" > "$DIST_DIR/simply-transfer.sha256"
    echo "---------------------------------------------------"
    echo "Packaging Complete!"
    echo "Native Linux package: $(pwd)/$DIST_DIR"
    echo "AppDir bundle:        $(pwd)/$APPDIR"
    echo "---------------------------------------------------"
fi
