#!/usr/bin/env bash
# build-deb.sh — Build ImpHash and produce a .deb package.
#
# Usage:
#   ./build-deb.sh            # build with all features (heif + pdq + avif)
#   ./build-deb.sh --no-heif  # skip HEIC/HEIF (no C++ toolchain needed)
#
# Requirements (auto-installed by this script if missing):
#   apt: build-essential cmake clang libdav1d-dev
#   cargo: cargo-deb

set -euo pipefail

FEATURES="heif,pdq"
DEB_ARGS=""

for arg in "$@"; do
    case $arg in
        --no-heif)  FEATURES="pdq" ;;
        --no-pdq)   FEATURES="heif" ;;
        --minimal)  FEATURES="" ;;
    esac
done

echo "==> Installing system build dependencies..."
sudo apt-get update -qq
sudo apt-get install -y --no-install-recommends \
    build-essential \
    cmake \
    clang \
    libdav1d-dev \
    libgtk-4-dev \
    libadwaita-1-dev \
    libgdk-pixbuf-2.0-dev \
    pkg-config

echo "==> Installing cargo-deb..."
cargo install cargo-deb --locked 2>/dev/null || true

echo "==> Building imphash (features: ${FEATURES:-none})..."
if [ -n "$FEATURES" ]; then
    cargo build --release --features "$FEATURES"
else
    cargo build --release
fi

# cargo-deb needs a placeholder icon if you don't have one yet
if [ ! -f assets/icons/imphash.png ]; then
    echo "==> No icon found at assets/icons/imphash.png — creating placeholder..."
    mkdir -p assets/icons
    # Create a minimal 256x256 placeholder PNG using Python (no ImageMagick needed)
    python3 - << 'PYEOF'
import struct, zlib

def make_png(width, height, r, g, b):
    def chunk(name, data):
        c = zlib.crc32(name + data) & 0xffffffff
        return struct.pack('>I', len(data)) + name + data + struct.pack('>I', c)
    sig = b'\x89PNG\r\n\x1a\n'
    ihdr = chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
    raw = b''
    for _ in range(height):
        raw += b'\x00' + bytes([r, g, b] * width)
    idat = chunk(b'IDAT', zlib.compress(raw))
    iend = chunk(b'IEND', b'')
    return sig + ihdr + idat + iend

with open('assets/icons/imphash.png', 'wb') as f:
    f.write(make_png(256, 256, 99, 144, 234))  # Adwaita blue
print("Placeholder icon written.")
PYEOF
fi

echo "==> Creating .deb package..."
if [ -n "$FEATURES" ]; then
    cargo deb --no-build --features "$FEATURES"
else
    cargo deb --no-build
fi

DEB=$(ls target/debian/imphash_*.deb 2>/dev/null | head -1)
if [ -n "$DEB" ]; then
    echo ""
    echo "==> Package ready: $DEB"
    echo "    Install with:  sudo apt install ./$DEB"
else
    echo "ERROR: .deb not found in target/debian/" >&2
    exit 1
fi
