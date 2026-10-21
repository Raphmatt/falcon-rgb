#!/usr/bin/env bash
# Wrap the release binaries in a macOS .app bundle and ad-hoc sign it.
# Usage: scripts/bundle-mac.sh "<App Name>" <cargo-output-dir>
set -euo pipefail

app_name="$1"
bin_dir="$2"
version="$(cargo metadata --format-version 1 --no-deps \
    | sed -n 's/.*"name":"falcon-gui","version":"\([^"]*\)".*/\1/p')"
version="${version:-0.0.0}"

app="dist/${app_name}.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

cp "$bin_dir/falcon-gui" "$app/Contents/MacOS/falcon-gui"
# The CLI ships inside the bundle too: "…/Contents/MacOS/falcon list"
cp "$bin_dir/falcon" "$app/Contents/MacOS/falcon"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>${app_name}</string>
    <key>CFBundleDisplayName</key><string>${app_name}</string>
    <key>CFBundleIdentifier</key><string>io.github.raphmatt.falcon-rgb</string>
    <key>CFBundleVersion</key><string>${version}</string>
    <key>CFBundleShortVersionString</key><string>${version}</string>
    <key>CFBundleExecutable</key><string>falcon-gui</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# Ad-hoc signature: enough to run locally. Its identity changes with every
# build, so the Input Monitoring permission may have to be granted again after
# a rebuild.
codesign --force --deep --sign - "$app"
echo "→ $app"
