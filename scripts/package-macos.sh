#!/bin/bash
# Package the already-built release executable; never access the user's task data.
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
    echo "App packaging requires macOS." >&2
    exit 1
fi
repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
executable="$repo_dir/target/release/hodoq"
bundle="$repo_dir/target/release/HodoQ.app"
if [[ ! -x "$executable" ]]; then
    echo "Build first: cargo build --locked --release --target-dir target" >&2
    exit 1
fi
version_output="$("$executable" --version)"
version="${version_output#hodoq }"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Unexpected executable version: $version_output" >&2
    exit 1
fi

mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$executable" "$bundle/Contents/MacOS/hodoq"
cp "$repo_dir/assets/app-icon/HodoQ.icns" "$bundle/Contents/Resources/HodoQ.icns"
cp "$repo_dir/assets/app-icon/Info.plist" "$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$bundle/Contents/Info.plist"
plutil -lint "$bundle/Contents/Info.plist"
# Local ad-hoc signing is not Developer ID signing or notarization for distribution.
codesign --force --sign - "$bundle"
codesign --verify --strict "$bundle"
echo "Created $bundle"
