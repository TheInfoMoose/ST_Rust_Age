#!/bin/bash
set -e

echo "Packaging Simply Transfer v2 for Linux..."

# Ensure we're in the workspace root
cd "$(dirname "$0")"

# Build release (already done, but good to ensure)
source $HOME/.cargo/env
cargo build --release -p simply-transfer-ui

# Create distribution directory
DIST_DIR="dist/SimplyTransfer-Linux-x64"
mkdir -p "$DIST_DIR"

# Copy the static GUI binary
cp target/release/simply-transfer-ui "$DIST_DIR/simply-transfer"

# Create a .desktop shortcut for Fedora/GNOME
cat << 'EOF' > "$DIST_DIR/SimplyTransfer.desktop"
[Desktop Entry]
Name=Simply Transfer v2
Comment=Enterprise-grade secure file transfer
Exec=sh -c '"$(dirname "%k")/simply-transfer"'
Terminal=false
Type=Application
Categories=Network;FileTransfer;
EOF

# Make executable
chmod +x "$DIST_DIR/simply-transfer"
chmod +x "$DIST_DIR/SimplyTransfer.desktop"

echo "---------------------------------------------------"
echo "Packaging Complete! 🎉"
echo "Your native Linux package is located at: $(pwd)/$DIST_DIR"
echo "To launch the UI natively on your screen, simply run:"
echo "  $(pwd)/$DIST_DIR/simply-transfer"
echo "---------------------------------------------------"
