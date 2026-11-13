# NestTerm

> 原生 **arm64** 跨平台 SSH / SFTP 运维客户端，对标 FinalShell。
> 技术栈锁定：**Tauri 2 + Vue 3 + TypeScript + Element Plus + xterm.js + Rust + russh**。
> 明确拒绝：Java、Electron、Wails/Qt、依赖 Rosetta 的 x86_64 产物、调用系统 `ssh` 子进程。

当前版本：`0.1.0` · **已交付 A（脚手架）+ B1（加密存储 / 连接管理 / 多标签终端）+ B2a（SFTP 浏览/编辑）+ B2b-1（文件上传下载 / 进度 / 取消）+ B2b-2a（目录递归传输）+ B2b-2b（FinalShell 本地导入）+ B2b-2c（JSON 导出/导入闭环）+ B2b-2e（known_hosts 迁移 / 下载目录选择器）+ B2b-2f（cwd 提示符）+ B3a（本地转发 / SOCKS5）+ B3b（系统监控）+ B3c（远端转发 -R / 后端自动重连）+ UX 加固（拖拽下载 / 设置 / CSP / 按需引入 / 会话恢复）+ 发布 CI**。
本机自用（macOS arm64）；跨平台适配按需求暂缓。

> 公开版已移除真实测试机信息与截图；仓库不包含任何连接配置、密码、私钥或本地数据库。

## 文档

| 文档 | 内容 | 状态 |
| --- | --- | --- |
| [00 产品范围与验收口径](docs/00-product-scope.md) | 定位、P0/P1/P2 边界、可测量验收标准、安全基线、Ubuntu 联调方案 | 完成 |
| [01 界面布局与交互规格](docs/01-ui-layout.md) | 布局、会话树/标签页/SFTP/监控、快捷键、状态矩阵 | 完成（B1 树+标签页 / B2a SFTP 面板） |
| [02 IPC 通信契约](docs/02-ipc-contract.md) | 命令/事件全量清单、TS 类型、错误码、时序 | 完成；B1/B2a 偏差见契约 §10 |
| [03 数据库与凭据存储](docs/03-database-schema.md) | SQLCipher、Keychain、DDL、迁移/备份 | 完成；B1 v1 schema / B2a/B2b 联调记录（§13） |
| [04 FinalShell 导入映射](docs/import-mapping.md) | 实测字段映射、分组重建、密码策略 | macOS 4.6.5 已实测；分组/跨平台待补 |
| [05 cwd 跟随问题复盘](docs/cwd-follow-postmortem.md) | 症状、四层根因、提示符权威方案、回归清单 | 已解决并归档 |
| [工程约定 / Agent Memory](AGENTS.md) | cwd 只用提示符、导航序号、单一事实来源 | 长期有效 |

---

## 1. 当前状态

### 1.1 已完成并验证

| 模块 | 能力 | 验证方式 |
| --- | --- | --- |
| 工程脚手架 | Tauri 2.12 + Vite 8 + Vue 3.5 + Element Plus 2.14 + xterm 6 | `npm run build` 通过；实机启动 |
| IPC 契约 | 20+ 命令、3 个事件；统一 `ApiError{code,message,detail,retryable}` | 前端 TS 类型对齐；实机 `app_info`/`profile_list` 回读 |
| 加密存储 | SQLCipher 整库加密；`user_version` 迁移；迁移前自动备份 | 7 个 Rust 单测；DB 文件头非 `SQLite format 3` |
| 凭据存储 | 密码 / 私钥口令写入系统钥匙串；只写不读回前端 | 钥匙串封装单测（MemoryStore）；实机调试模式走内存实现 |
| 连接管理 | 分组树（≤3 级）、连接 CRUD、复制、移动、搜索、右键菜单 | 仓储单测 + 实机树形渲染 |
| 多标签终端 | 每连接独立 xterm / PTY；切换不销毁；关闭确认；首屏字节缓冲 | Ubuntu 实机：密码 + 私钥连接、shell 命令、PTY 尺寸 `33 65` |
| 认证 | 密码（inline / 钥匙串）、私钥 ed25519/rsa（含口令）、ssh-agent（unix） | 密码、私钥已实机联调；agent 代码路径完成、未实机 |
| 主机密钥 | TOFU 入库、指纹变更拒绝、`strict` 模式 | 单测覆盖 learn/trusted/mismatch/strict |
| SFTP（B2a） | 同一 SSH 通道：浏览、在线编辑、chmod、重命名、删除、跟随终端 cwd（shell 提示符） | Ubuntu 实机自检通过：write/read/chmod 600/rename/list/递归删除 |
| SFTP 传输（B2b-1） | 单文件上传/下载、拖拽上传、进度/取消、冲突 ask/overwrite/skip/rename | Ubuntu 实机：8MB 双端 SHA-256 一致；覆盖上传修复；64MB 取消无残留 |
| 目录传输（B2b-2a） | 递归上传/下载、空目录、隐藏文件、嵌套目录、按文件进度 | Ubuntu 实机：嵌套树 + 2MB 二进制双端哈希一致 |
| FinalShell 导入（B2b-2b） | 扫描 `~/Library/FinalShell/conn/*.json`，保留名称/主机/端口/用户/备注；密码不迁移 | 本机 4.6.5：3 条真实连接导入成功；二次导入全部跳过 |
| JSON 导出（B2b-2c） | 导出分组路径/连接配置到 `~/Downloads/NestTerm/`，不含密码/私钥 | 本机实测 10 连接 / 4 分组，5.2KB，`credentialsExcluded=true` |
| JSON 导入（B2b-2d） | 恢复备份：按路径重建分组、按 host/port/user 去重、事务写入 | 全新空库导入 → 10 连接 / 4 分组，与导出源一致 |
| known_hosts 迁移（B2b-2e） | 启动时仅针对已有连接，从 `~/.ssh/known_hosts` 迁移公钥；支持 hashed host；冲突不覆盖 | 启动接入 + 单测；本机系统文件无匹配主机，未产生实际导入 |
| 下载目录选择（B2b-2e） | Tauri dialog 选择下载目录，取消则中止 | 构建通过；系统对话框无法在无辅助功能权限下自动化 |
| cwd 跟随（B2b-2f） | 解析 shell 提示符（`user@host:~/path$`）；不解析输入、不依赖 OSC 7 | 正式服务器：`cd www/html/../backend` 正确；错误 `cd` 不动 |
| 端口转发（B3a） | 本地 `-L` + SOCKS5 `-D` + 远端 `-R`，规则持久化，按会话启停 | Ubuntu 实机：2222→22、1080 SOCKS5、远端 9000→本地 18080 HTTP 均通过 |
| 自动重连（B3c） | 后端编排，断线后同一 sessionId 重建，最多 3 次；SFTP 重开、转发规则恢复、PTY 尺寸取最新 | 实机 `exit`：同标签重建、新登录、SFTP 恢复、2222/1080/9000 全部恢复 |
| 会话恢复（UX） | 退出/重启后按 settings KV 恢复上次打开的 profile；受 `restoreSessions` 控制 | 强杀进程后无自动连接重启，标签自动恢复并连上 |
| 拖拽下载（UX） | 文件/目录行 `⠿` 按住拖到 Finder；`DragItem::promise` 在 drop 时才下载（目录递归） | 命令构建通过；实际 Finder 拖放需鼠标，未自动化 |
| 设置 / CSP / 按需 | 设置对话框持久化并应用；CSP 收紧；Element Plus 按需引入 | 设置对话框实机渲染；JS 846KB(gzip 256KB)、CSS 159KB；dev/release UI 正常 |
| 发布 / CI | GitHub Actions 未签名 CI + 签名/公证 release workflow + 本地打包脚本 | YAML 校验通过；需 Apple 凭据才能产出公证包 |
| 系统监控（B3b） | Linux `/proc` 采样 CPU/内存/负载/磁盘/网络，2s 刷新侧边面板 | Ubuntu 实机：面板显示 16.6% 内存、6.0% 磁盘、累计网络 |
| 终端体验 | resize 同步、8ms 输出合并、UTF-8 base64、vim/tmux 兼容 | 初始 PTY 尺寸实机验证；窗口 resize/vim/tmux 待补测 |
| 设置 | `settings_get/update`，section 级 JSON 持久化 | 单测：部分更新保留默认值 |
| arm64 | `aarch64-apple-darwin` 原生产物 | `lipo -archs` = `arm64`；界面右上角 `aarch64` |

### 1.2 未完成（按顺序）

| 优先级 | 缺口 | 计划 |
| --- | --- | --- |
| P1 | 拖拽 Finder 的鼠标自动化验证；主题/面板宽度 | 评估 |
| P1 | 跳板机、批量命令、片段库、日志回放、主题自定义 | 二期 |
| P2 | Ollama AI 助手、负载告警 | 三期 |

---

## 2. 环境要求

- macOS 11+（Apple Silicon 原生；不以 Intel 为目标）
- Node.js ≥ 20.19（验证：v24.14.0）
- Rust ≥ 1.85（验证：1.99.0）
- Xcode Command Line Tools
- 首次编译 SQLCipher 需要 Perl + C 工具链（macOS 自带；约 3–5 分钟一次性成本）

## 3. 快速开始

```bash
npm install
npm run app:dev          # Vite + Rust 调试构建 + 打开窗口
npm run build            # 仅前端：vue-tsc + vite build
npm run app:build:mac-arm64   # 生成 .app / .dmg
```

### 3.1 首次启动会发生什么

1. 应用解析数据目录，向系统钥匙串读取/创建 `db-key-v1`（macOS 可能弹一次授权）。
2. 用该密钥打开 `nestterm.db`（SQLCipher），执行 `migrations/0001_init.sql`，`user_version=1`。
3. 若钥匙串被拒绝或数据库损坏：窗口正常打开，顶部显示红色「存储初始化失败」，连接配置不可用。
4. 新建连接时，密码/口令写入钥匙串，DB 只保存 `hasPassword/hasPassphrase` 布尔值。
5. 首次连接某主机时写入应用自有 `known_hosts` 表；指纹变化直接拒绝。

> 私有部署/自动化可用调试开关（**仅 debug 构建生效**）：
> `NESTTERM_DATA_DIR=<目录> NESTTERM_DB_KEY_HEX=<64位hex> NESTTERM_MEMORY_SECRETS=1 npm run app:dev`
> 后者用内存凭据库，避免钥匙串授权弹窗；重启即丢。

### 3.2 受限环境（CI / 只读 HOME 沙箱）

```bash
export CARGO_HOME="$PWD/.cargo"
export npm_config_cache="$PWD/.npm-cache"
npm install && npm run app:build:mac-arm64
```

## 4. arm64 与打包

### 4.1 验证原生 arm64

```bash
APP="src-tauri/target/aarch64-apple-darwin/release/bundle/macos/NestTerm.app"
file "$APP/Contents/MacOS/nestterm"          # Mach-O 64-bit executable arm64
lipo -archs "$APP/Contents/MacOS/nestterm"   # arm64
```

运行期右上角显示 `aarch64 · macos`（来自 `app_info`），可随时自证非 Rosetta。

### 4.2 生产构建

```bash
./scripts/package-macos.sh
# 或未签名最小构建：
npm run app:build:mac-arm64
# 产物：src-tauri/target/aarch64-apple-darwin/release/bundle/{macos,dmg}/
```

> DMG 由 `hdiutil` 生成；受限沙箱可能报 `Operation not permitted`，`.app` 不受影响。
> GitHub Actions：`.github/workflows/ci.yml` 生成未签名 app/dmg；release workflow 见 [docs/release.md](docs/release.md)。

### 4.3 macOS 签名 + 公证

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="app-specific-password"   # appleid.apple.com 生成
export APPLE_TEAM_ID="TEAMID"
./scripts/package-macos.sh
```

脚本负责 hardened runtime 签名、DMG 公证与 stapling；CI secrets 与完整步骤见 [docs/release.md](docs/release.md)。

### 4.4 Windows / Linux

| 平台 | 产物 | 依赖 |
| --- | --- | --- |
| Windows 10/11 | `.exe`（NSIS） | WebView2、MSVC Build Tools |
| Linux | `.deb` | `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf` |

Windows/Linux 适配暂缓（本机 macOS arm64 自用）；Linux 上 SQLCipher 使用 vendored OpenSSL。

## 5. 代码结构

```
src/                                   # Vue 3 前端
├─ App.vue                             # 布局 + 事件编排 + 连接流程
├─ components/
│  ├─ SessionTree.vue / ProfileItem.vue # 分组树、搜索、右键菜单
│  ├─ WorkspaceTabs.vue                 # 多标签 + 关闭确认
│  ├─ TerminalPane.vue                  # xterm + resize + 输入
│  └─ ProfileDialog.vue                 # 新建/编辑/连接（密码/私钥/agent 占位）
├─ stores/{profiles,sessions}.ts        # 轻量单例 store（不引入 Pinia）
└─ lib/{api,ssh,types,base64}.ts        # IPC 封装 + 契约类型
src-tauri/src/
├─ api/                                 # app_info、profile_*、group_*、settings_*、known_hosts_*
├─ db/
│  ├─ mod.rs                            # SQLCipher 打开、迁移、连接句柄、单测
│  ├─ models.rs                         # 领域类型（serde camelCase）
│  └─ repo/{groups,profiles,known_hosts,settings}.rs
├─ secret.rs                            # Keychain 封装 + DB 密钥 + MemoryStore
├─ state.rs                             # 全局状态；启动失败不 panic
├─ error.rs                             # ApiError / ErrorCode
└─ ssh/{commands,session,handler,events}.rs  # 建连/认证/PTY/字节泵
src-tauri/migrations/0001_init.sql      # schema v1（与 docs/03 §6 逐字一致）
```

## 6. IPC 速查

完整契约：[docs/02-ipc-contract.md](docs/02-ipc-contract.md)。

| 域 | 命令 |
| --- | --- |
| 应用 | `app_info`、`known_hosts_list`、`known_hosts_remove` |
| 分组 | `group_list`、`group_create`、`group_update`、`group_delete`、`group_move` |
| 连接 | `profile_list`、`profile_get`、`profile_create`、`profile_update`、`profile_delete`、`profile_duplicate`、`profile_move`、`profile_set_secret`、`profile_clear_secret` |
| 设置 | `settings_get`、`settings_update` |
| 会话 | `ssh_connect`、`ssh_write`、`ssh_resize`、`ssh_disconnect` |
| SFTP | `sftp_open`、`sftp_close`、`sftp_list`、`sftp_stat`、`sftp_realpath`、`sftp_mkdir`、`sftp_rename`、`sftp_delete`、`sftp_chmod`、`sftp_read_file`、`sftp_write_file` |
| 传输 | `sftp_upload`、`sftp_download`、`sftp_cancel_transfer` |
| 转发 | `forward_rules_list`、`forward_rule_create/delete`、`forward_start/stop`、`forward_running` |
| 监控 | `monitor_sample`（Linux `/proc`） |
| 导入 | `finalshell_preview`、`finalshell_import` |
| 导出/导入 | `nestterm_export`、`nestterm_import` |

事件：`ssh:status`、`ssh:data`、`ssh:hostkey`、`ssh:cwd`、`transfer:progress`、`transfer:done`（均带 `ts`，按 `sessionId` 过滤）。

错误统一为：

```ts
interface ApiError { code: string; message: string; detail?: string; retryable: boolean }
```

## 7. 关键设计决策

| 决策 | 原因 |
| --- | --- |
| `russh` 而非 `ssh2-rs` | 纯 Rust、无 libssh2 原生依赖；SFTP/转发可复用同一 Handle |
| `ring` 加密后端 | 免 CMake；RSA 私钥支持保留 `rsa` feature |
| SQLCipher（vendored OpenSSL） | 整库加密且 SQL 不变；不依赖系统 OpenSSL/LibreSSL |
| 密码不进 DB | DB 泄露也拿不到凭据；钥匙串按应用授权 |
| 前端生成 `sessionId` | 规避“事件先于 invoke 返回”的竞态；多标签天然隔离 |
| 输出 base64 + 8ms 合并 | JSON IPC 安全传输字节流；避免高频重绘打爆 IPC |
| TOFU 入应用 DB | 不污染 `~/.ssh/known_hosts`；可做 UI 管理 |
| 单写连接 `Mutex<Connection>` | 单人桌面应用足够；避免 WAL 多连接可见性心智负担 |

## 8. 验证记录（本机实测）

```bash
cargo clippy           # 0 warning
cargo test --lib       # 26 passed（DB 7 + SFTP 6 + 传输 2 + FinalShell 2 + 导入导出 4 + known_hosts 1 + 转发 1 + 监控 2 + 重连 1）
npm run build          # vue-tsc + vite：js 846KB(gzip 256KB)，css 159KB(gzip 24KB)
file .../nestterm      # Mach-O 64-bit executable arm64（13MB 可执行文件 / 13MB .app）
head -c16 nestterm.db  # 非 "SQLite format 3"（SQLCipher 已加密）
```

### 8.1 Ubuntu 24.04 实机联调（2026-10-26）

测试机：Ubuntu 24.04 物理机（已脱敏）。

| 项 | 结果 |
| --- | --- |
| 系统 ssh 密码登录 | ✅ |
| 一次性 ed25519 公钥安装 + 免密登录 | ✅ |
| 应用内**密码认证**连接（auto-connect 注入 inline password） | ✅ |
| 应用内**私钥认证**连接 | ✅ |
| 真实 PTY + shell + MOTD + 命令执行 | ✅（`stty size` = `33 65`） |
| shell 提示符 cwd → SFTP 跟随 | ✅（正式服务器 `cd` 多级目录正确） |
| SFTP 命令层自检 | ✅ `mkdir/write/read/chmod 600/stat/rename/list/delete recursive` |
| 自检报告 | 服务器 `~/nestterm-selftest.json` → `"ok": true` |
| SFTP 8MB 上传 → 冲突 ask → 覆盖 → 下载 | ✅ 双端 SHA-256 一致；报告 `~/nestterm-transfer-selftest.json` → `"ok": true` |
| 目录递归（嵌套 + 空目录 + 隐藏文件 + 2MB 二进制） | ✅ 远端树结构完整，4 个文件双端 SHA-256 全部一致 |
| FinalShell 4.6.5 本地导入 | ✅ 3 条测试连接（已脱敏）；密码未迁移；二次导入全部跳过 |
| NestTerm JSON 导出 | ✅ 10 连接 / 4 分组；无 password/passphrase 字段；分组路径正确 |
| JSON 导出 → 空库导入 | ✅ 重建 4 分组 / 10 连接；按 host/port/user 去重；二次导入全部跳过 |
| known_hosts 迁移 | ✅ 启动接入、幂等、只处理已有连接；本机无匹配主机，实际导入 0 条 |
| 错误 cd 不误跳 | ✅ `cd aa` 失败，面板保持当前目录 |
| 本地转发 `-L` | ✅ `127.0.0.1:2222` → 服务器 `127.0.0.1:22`，收到 `SSH-2.0-OpenSSH_9.6p1` |
| SOCKS5 `-D` | ✅ `127.0.0.1:1080` 握手成功，代理到服务器 `22` 端口收到 SSH banner |
| 系统监控 | ✅ CPU/内存/负载/磁盘/网络采样；面板 2s 刷新 |
| 远端转发 `-R` | ✅ 服务器 `127.0.0.1:9000` → Mac `127.0.0.1:18080`，curl 返回 `remote-forward-ok`；停止后端口关闭 |
| 自动重连 | ✅ `exit` 后同 sessionId 重建；新登录、SFTP 重开、状态已连接 |
| 64MB 上传中途取消 | ✅ `cancelled`，远端无最终文件、无 `.part` 残留 |
| 主机密钥 TOFU | ✅ 首次写入应用 DB，二次连接指纹一致 |

> 传输自检同样走真实 IPC。覆盖上传最初在 99% 失败：OpenSSH 的 SFTP `rename` 不覆盖已存在目标；
> 已改为“旧文件临时备份 → rename → 删备份 / 失败回滚”。

> 自检通过前端 dev hook 调用真实 IPC（非模拟）：`NESTTERM_DEV_AUTOCONNECT` / `NESTTERM_DEV_AUTOCONNECT_PASSWORD` / `NESTTERM_DEV_INITIAL_COMMAND` / `NESTTERM_DEV_SFTP_SELFTEST`，均只在 debug 构建生效，release 不编译这些分支。

### cwd 跟随配置

NestTerm 直接解析 shell 提示符，不使用 OSC 7 / 输入启发式：

- 支持 `user@host:~/path$`；
- 支持 `[user@host /abs/path]#`、`[user@host ~]#`（CentOS 用 `\w` 时）。

CentOS 7 默认 `PS1` 是 `[\u@\h \W]\$`，`\W` 只显示最后一级目录名。应用检测到这种裸目录名提示符时，会在**当前会话自动执行一次**：

```bash
PS1='[\u@\h \w]\$ '
```

只影响本次 shell，不修改远端 `.bashrc`；之后提示符为 `[root@localhost /home/demo]#`，即可跟随。自动升级失败时才提示手动配置。

## 9. 已知限制

1. **cwd 跟随**：只解析 shell 提示符（`user@host:~/path$`）；OSC 7 与输入启发式已删除。列表请求带导航序号，过期结果直接丢弃。
2. **SFTP 拖拽**：上传支持文件/目录；拖出 Finder 支持文件与目录（目录在 drop 时递归下载），需按住左侧 `⠿`。原生拖放在无辅助功能权限的环境下无法自动化验证。
3. **ssh-agent**：unix 已实现（`SSH_AUTH_SOCK`）；Windows Pageant 未实现，会返回 `SSH_AUTH_UNSUPPORTED`。
4. **端口转发**：local/dynamic/remote 均已实现；SOCKS5 无认证；`auto_start` 字段已落库但尚未自动启动；远端转发依赖服务器允许 `AllowTcpForwarding`。
5. **监控**：仅 Linux `/proc`（macOS/BSD 返回 `MONITOR_UNSUPPORTED`）；网络为累计值，非实时速率；2s 轮询，无历史落库/告警；面板与 SFTP 共用右侧 dock。
6. **导入导出**：FinalShell 4.6.5（macOS）本地导入、NestTerm JSON 导出/导入闭环均已实测；FinalShell 分组结构暂未导入；Windows/Linux 按需求不适配。密码字段是加密 base64，不做解密。
7. **known_hosts 迁移**：只迁移已有连接涉及的主机；系统文件中没有匹配主机时不会导入；不做全量主机遍历。
8. **下载目录选择器**：系统原生对话框无法在无辅助功能权限下自动点击；后端 `localDir` 已实机验证。
9. **自动重连**：后端编排，最多 3 次；仅对已保存密码/私钥的配置生效；已运行转发规则会恢复；用户主动断开不重连。
10. **应用退出/休眠**：退出后重启会恢复上次打开的 profile（需已保存凭据）；系统休眠后的连接恢复依赖后端重连，无专门唤醒钩子；PTY 尺寸取最近一次 resize。
11. **设置范围**：终端字号/行高/回滚/字体、SFTP 跟随/隐藏/权限/冲突策略/默认下载目录、监控间隔、自动重连、关闭确认、host key 策略已生效；主题与面板宽度暂未应用。
12. **CSP**：release 与 dev 均已设置；release 在沙箱环境启动时因数据目录不可写而显示存储错误，但 UI 正常渲染（非 CSP 问题）。
13. **CI/签名**：workflow YAML 已校验，但本机未运行 GitHub Actions；公证需要 Apple Developer 凭据，当前未产出公证包。
14. **测试机副作用**：已向 `nestterm` 账号写入一次性公钥、`~/nestterm-live/` 测试文件与报告；测试结束可删除。
15. **SFTP 目录列表**：`realpath` 失败回退原路径；`read_dir` 失败重置 SFTP 会话重试；仍失败则用同一 SSH 会话的 shell `find -H -printf` fallback。`cd` 解析器只在列表成功后提交 current；候选路径先经同连接 `test -d` 校验，不存在的 `cd` 不产生 cwd 事件。

## 10. 剩余（非阻塞）

1. 原生拖放的鼠标自动化验证；目录拖出的 Finder 行为需要人工确认。
2. 设置：主题与面板宽度应用；更多终端选项。
3. 系统休眠唤醒钩子/更细的重连策略；当前依赖 keepalive + 后端重连。
4. CI 已接入；签名/公证需 Apple Developer 凭据后跑 release workflow 做真机冒烟。
5. 跳板机、批量命令、片段库、日志回放等二期功能。

联调所需：Ubuntu 地址/端口/临时账号，或已配置的一次性公钥（见 [00 §7](docs/00-product-scope.md#7-ubuntu-联调环境b-期执行)）。

## 11. 版权声明

功能复刻：仅参考 FinalShell 的交互形态与能力边界，不包含、不反编译、不复制其源码或资源。
图标为原创生成（`app-icon.png` → `src-tauri/icons/`）。
