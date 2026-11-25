#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
set -euo pipefail

project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$project_root"

case "${VORTEX_CATALOG_API_URL:-}" in
    https://?*) ;;
    *) printf 'Set VORTEX_CATALOG_API_URL to the HTTPS catalog endpoint before building.\n' >&2; exit 1 ;;
esac

case "$(uname -s):$(uname -m)" in
    Darwin:arm64) artifact=vortex-macos-arm64 ;;
    Darwin:x86_64) artifact=vortex-macos-x64 ;;
    Linux:x86_64) artifact=vortex-linux-x64 ;;
    MINGW*:x86_64|MSYS*:x86_64) artifact=vortex-windows-x64 ;;
    *) printf 'Unsupported build host. See README.md for supported targets.\n' >&2; exit 1 ;;
esac

target_dir=${CARGO_TARGET_DIR:-$project_root/target}
output_dir=$project_root/target/packages
stage=$project_root/target/package-staging/$artifact

cargo test --locked --release --target-dir "$target_dir"
cargo build --locked --release --target-dir "$target_dir"

mkdir -p -- "$stage" "$output_dir"
cp -- LICENSE README.md assets/geist-OFL.txt "$stage/"

case "$artifact" in
    vortex-windows-x64)
        payload=vortex-desktop.exe
        cp -- "$target_dir/release/$payload" "$stage/$payload"
        ;;
    vortex-linux-x64)
        payload=vortex-desktop
        install -m 755 "$target_dir/release/$payload" "$stage/$payload"
        ;;
    vortex-macos-*)
        payload=Vortex.app
        contents=$stage/$payload/Contents
        mkdir -p -- "$contents/MacOS" "$contents/Resources" "$stage/Vortex.iconset"
        install -m 755 "$target_dir/release/vortex-desktop" "$contents/MacOS/vortex-desktop"

        version=$(awk -F\" '/^version = / { print $2; exit }' Cargo.toml)
        cat > "$contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>vortex-desktop</string>
  <key>CFBundleIdentifier</key><string>me.watchvortex.desktop</string>
  <key>CFBundleName</key><string>Vortex</string>
  <key>CFBundleDisplayName</key><string>Vortex</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version%%-*}</string>
  <key>CFBundleVersion</key><string>${version%%-*}</string>
  <key>CFBundleIconFile</key><string>Vortex.icns</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
        plutil -lint "$contents/Info.plist"
        for size in 16 32 128 256 512; do
            double=$((size * 2))
            sips -z "$size" "$size" assets/vortex-icon.png --out "$stage/Vortex.iconset/icon_${size}x${size}.png" >/dev/null
            sips -z "$double" "$double" assets/vortex-icon.png --out "$stage/Vortex.iconset/icon_${size}x${size}@2x.png" >/dev/null
        done
        iconutil -c icns "$stage/Vortex.iconset" -o "$contents/Resources/Vortex.icns"
        codesign --force --sign - "$stage/$payload"
        codesign --verify --strict "$stage/$payload"
        ;;
esac

archive=$output_dir/$artifact.tar.gz
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$stage" "$payload" LICENSE README.md geist-OFL.txt
tar -tzf "$archive"
printf '\nCreated %s\n' "$archive"
