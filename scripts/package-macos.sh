#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# 本地/CI 通用打包脚本；不传签名环境变量时生成未签名 .app/.dmg。
# 签名与公证变量见 docs/release.md。

if [ ! -d node_modules ]; then
  npm ci
fi

npm run build
npx tauri build --target aarch64-apple-darwin --bundles app,dmg

APP="src-tauri/target/aarch64-apple-darwin/release/bundle/macos/NestTerm.app"
DMG=$(ls src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/*.dmg | head -n 1)

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
  codesign --force --deep --options runtime --timestamp \
    --sign "$APPLE_SIGNING_IDENTITY" "$APP"
  codesign --verify --deep --strict --verbose=2 "$APP"
  codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$DMG"
fi

if [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  xcrun notarytool submit "$DMG" \
    --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl --assess --type execute --verbose=2 "$APP" || true
fi

echo "APP: $APP"
echo "DMG: $DMG"
