# 发布、签名与公证（macOS arm64）

> 目标：本机开发/自用可先发未签名包；对外分发必须 Developer ID 签名 + 公证。

## 1. 产物

```
src-tauri/target/aarch64-apple-darwin/release/bundle/macos/NestTerm.app
src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/NestTerm_<version>_aarch64.dmg
```

## 2. 本地打包

```bash
./scripts/package-macos.sh
```

- 无签名环境变量：生成未签名 `.app`/`.dmg`。
- 受限沙箱里 `hdiutil` 可能被拒绝；在普通终端运行脚本，或只用 `--bundles app`。

## 3. 签名 + 公证

需要 Apple Developer Program 的 **Developer ID Application** 证书。

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="app-specific-password"   # appleid.apple.com 生成
export APPLE_TEAM_ID="TEAMID"
./scripts/package-macos.sh
```

脚本会：`codesign`（hardened runtime + timestamp）→ 签名 DMG → `notarytool submit --wait` → `stapler staple/validate`。

验证：

```bash
codesign --verify --deep --strict --verbose=2 ".../NestTerm.app"
spctl --assess --type execute --verbose=2 ".../NestTerm.app"
xcrun stapler validate "...dmg"
```

## 4. GitHub Actions

- `.github/workflows/ci.yml`：push/PR 跑 Rust 测试、前端构建、生成未签名 `.app`/`.dmg` artifact。
- `.github/workflows/release.yml`：手动触发或 `v*` tag，导入证书后签名；有 Apple 凭据时公证 DMG。

需要的 Repository Secrets：

| Secret | 用途 |
| --- | --- |
| `APPLE_CERTIFICATE` | Developer ID `.p12` 的 base64 |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` 密码 |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: ... (TEAMID)` |
| `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` | 公证 |
| `KEYCHAIN_PASSWORD` | CI 临时 keychain 密码 |

生成证书 base64：

```bash
base64 -i DeveloperID.p12 | pbcopy
```

> 未配置 secrets 时，release workflow 会失败；日常 CI 使用 `ci.yml` 的未签名产物。

## 5. 当前边界

- 仅 arm64 macOS；Windows/Linux 打包按用户要求暂不适配。
- CI 需要 macOS runner（`macos-14`）；本地 `hdiutil` 受限时 DMG 只能在普通终端/CI 生成。
- 公证需要 App Store Connect 权限；没有凭据时只能验证到“已签名但未公证”。
