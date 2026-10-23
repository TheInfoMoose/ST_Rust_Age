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

    cat << 'EOF' > "$APP_BUNDLE/Contents/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>simply-transfer</string>
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
        cp assets/logo.png "$APPDIR/simply-transfer.png"
        cp assets/logo.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/simply-transfer.png"
    fi

    cat << 'EOF' > "$DIST_DIR/SimplyTransfer.desktop"
[Desktop Entry]
Name=Simply Transfer
Comment=Enterprise-grade P2P encrypted file transfer and snapshot sync utility
Exec=simply-transfer
Icon=simply-transfer
Terminal=false
Type=Application
Categories=Network;FileTransfer;Utility;
EOF

    cp "$DIST_DIR/SimplyTransfer.desktop" "$APPDIR/simply-transfer.desktop"
    cp "$DIST_DIR/SimplyTransfer.desktop" "$APPDIR/usr/share/applications/simply-transfer.desktop"

    cat << 'EOF' > "$APPDIR/AppRun"
#!/bin/sh
HERE="$(dirname "$(readlink -f "${0}")")"
exec "${HERE}/usr/bin/simply-transfer" "$@"
EOF

    chmod +x "$DIST_DIR/simply-transfer" "$DIST_DIR/SimplyTransfer.desktop" "$APPDIR/AppRun" "$APPDIR/usr/bin/simply-transfer"

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
