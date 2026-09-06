#!/bin/bash
# Format conversion only: preserve the approved PNG artwork and its alpha channel.
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
    echo "Icon regeneration requires macOS (sips and iconutil)." >&2
    exit 1
fi
repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
assets_dir="$repo_dir/assets/app-icon"
icon_work="$(mktemp -d "${TMPDIR:-/tmp}/hodoq-icons.XXXXXX")"
trap 'rm -rf "$icon_work"' EXIT
iconset="$icon_work/HodoQ.iconset"
mkdir "$iconset"

for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$assets_dir/hodoq.png" \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    retina_size=$((size * 2))
    sips -z "$retina_size" "$retina_size" "$assets_dir/hodoq.png" \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$assets_dir/HodoQ.icns"
# Windows 11 supports PNG-compressed ICO entries and scales this for each DPI.
sips -z 256 256 -s format ico "$assets_dir/hodoq.png" \
    --out "$assets_dir/hodoq.ico" >/dev/null
echo "Generated HodoQ.icns and hodoq.ico from assets/app-icon/hodoq.png."
