#!/bin/bash
set -e

echo "============================================="
echo " Simply Transfer V2 - Windows Cross-Compiler"
echo "============================================="

# 1. Check for Mingw-w64 Toolchain
if ! command -v x86_64-w64-mingw32-gcc &> /dev/null; then
    echo "❌ Error: mingw-w64 compiler not found."
    echo "Please install it by running:"
    echo "   sudo apt update && sudo apt install mingw-w64"
    exit 1
fi

# 2. Add Rust Windows Target
echo "➔ Ensuring x86_64-pc-windows-gnu Rust target is installed..."
rustup target add x86_64-pc-windows-gnu

# 3. Configure Cargo Linker for Windows
mkdir -p .cargo
if ! grep -q "x86_64-pc-windows-gnu" .cargo/config.toml 2>/dev/null; then
    echo "➔ Configuring Cargo to use Mingw-w64 linker..."
    cat <<EOF >> .cargo/config.toml
[target.x86_64-pc-windows-gnu]
linker = "x86_64-w64-mingw32-gcc"
EOF
fi

# 4. Build Release
echo "➔ Building Release Binary for Windows..."
cargo build --release --target x86_64-pc-windows-gnu

echo "✅ Build Complete!"
echo "Your Windows executable is located at:"
echo "   target/x86_64-pc-windows-gnu/release/simply-transfer.exe"
