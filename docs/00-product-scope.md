# NestTerm 产品范围与验收口径

> 文档版本：v0.1（2026-10-26） · 对应代码：0.1.0（A 期脚手架）
> 同组文档：[界面布局](01-ui-layout.md) · [IPC 契约](02-ipc-contract.md) · [数据库设计](03-database-schema.md)

---

## 1. 产品定位

**一句话**：原生 arm64、单窗口高信息密度的 SSH/SFTP 运维工作台，替代 FinalShell 的日常使用路径。

**目标用户**：管理 1–100 台 Linux 服务器的开发者 / 单人 OPC / 小团队运维。典型动作是「连上去、看日志、改配置、传文件、看负载」，而不是批量编排或合规审计。

**对标方式**：功能复刻，不做源码/资源复制。交互形态参考 FinalShell 的高频路径（会话树 + 标签页 + 终端与文件联动），但布局、命名、组件、配色均为自研。

**明确不做的用户场景**：

- 堡垒机/双人复核/命令审计合规（需要独立审计端，不在本产品范围）
- 大规模并行编排（Ansible/Terraform 的领域）
- 浏览器版 / 移动端（WebView 与移动端输入法不适合终端高频操作）

## 2. 硬性约束与自证方式

| 约束 | 落地方式 | 自证 |
| --- | --- | --- |
| arm64 原生，不依赖 Rosetta | 构建 `aarch64-apple-darwin`，不引入 x86 依赖 | `lipo -archs` = `arm64`；运行期 `app_info.arch` 显示 `aarch64` |
| 禁止 Java / Electron / Wails / Qt | Tauri 2 + 系统 WebView | 产物无 JRE / 无 Chromium 内核（`du -sh` ≈ 7MB 级别） |
| 禁止调用系统 `ssh` 子进程 | `russh` 纯 Rust 实现 | 代码中无 `Command::new("ssh")`；进程树无 ssh 子进程 |
| SFTP 复用同一 SSH 通道 | `russh` 通道上开 `sftp` 子系统 | 断开 SSH 即 SFTP 失效；服务端 `sshd` 仅一条 TCP 连接 |
| rusqlite 加密存储 | SQLCipher + 密钥入系统钥匙串 | `file nestterm.db` 非明文 SQLite 头；无密钥打不开 |
| macOS 密码/私钥入 Keychain | `keyring` crate，服务名 `com.nestterm.desktop` | Keychain Access 可检索；DB 内无明文 secret |

## 3. 范围分层

### 3.1 P0 —— 没有就不算替代品

| 模块 | 能力 | 计划期 |
| --- | --- | --- |
| 连接管理 | 分组树、标签页、搜索、新建/编辑/删除、FinalShell 配置导入、JSON 导出 | B |
| 终端 | 真实 PTY、xterm 兼容、vim/tmux、resize 同步、断线自动重连 | A（基础）/ B（重连） |
| SFTP | 目录列表、上传/下载/拖拽、权限修改、在线编辑、与终端 cwd 联动 | B |
| 认证 | 密码、ed25519/rsa 私钥（含 passphrase）、ssh-agent | B |
| 端口转发 | 本地 `-L`、远程 `-R`、动态 SOCKS5 `-D`，规则持久化 | B |
| 监控 | 复用 SSH 通道采集 CPU/内存/负载/磁盘/网络，侧边实时面板 | B |
| 安全 | 主机密钥 TOFU、指纹变更拒绝、密钥不落前端 | A（TOFU）/ B（迁移入库） |

### 3.2 P1 —— 二期

跳板机（ProxyJump）、批量命令、命令片段库、终端日志与回放、主题/字体自定义、标签页会话恢复、SFTP 断点续传、目录树懒加载。

### 3.3 P2 —— 可选

本地 Ollama 运维助手、负载阈值告警（Webhook/通知）、SFTP 双栏对比、团队配置同步。

### 3.4 明确不做

X11 转发、FTP/FTPS/WebDAV、内置代码编辑器（只做纯文本编辑，不替代 IDE）、内置数据库客户端、自动巡检脚本引擎。

## 4. P0 验收口径

> 原则：每条都能在 Ubuntu 测试机上用一条命令或一次点击验证，不写“体验流畅”这类不可测描述。

### 4.1 连接管理

1. 新建连接（密码/私钥/agent 三种）→ 出现在对应分组 → 重启应用后仍在 → 密码不出现在任何日志/DB 明文中。
2. 500 条连接的分组树，展开/搜索响应 < 100ms（首屏渲染 < 300ms）。
3. 删除分组时其下连接按「移入未分组」处理，不静默删除；删除连接需二次确认。
4. 导入 FinalShell 导出文件：100 条 < 3s，0 条导致崩溃，无法映射的字段进入 warnings 且可复制。
5. 导出 JSON 默认不含密码；勾选“包含凭据”时必须输入导出口令（AES-256-GCM）。

### 4.2 终端

1. 连接后 1s 内出现 shell 提示符；`echo $TERM` = `xterm-256color`。
2. `vim` 打开 500 行文件，方向键/插入/退出正常，`Ctrl+C` 不误触发；256 色主题正常。
3. `tmux` 新建/分屏/`Ctrl+b d` 正常；窗口 resize 后 `tmux` 状态栏不错行（`stty size` 与窗口一致）。
4. 粘贴 1MB 文本：字节数完全一致（`wc -c` 对比），无卡死 > 2s。
5. 中文输入法上屏、中文文件名 `ls` 显示不乱码；`cat` 二进制 100MB 不 OOM（前端可丢弃）。
6. 拔网线/`sudo ip link set down`：≤ 10s 内状态变为“重连中”，≤ 30s 内自动恢复（第 1 次立即重试，其后 1/2/4/8s 退避，最多 5 次；沿用同一 `sessionId`）；恢复后终端有分隔线与时间戳。
7. 认证失败、主机指纹变更、端口拒绝三种错误，提示文案可区分且不泄露密码。

### 4.3 SFTP

1. 终端 `cd /var/log` 后 ≤ 500ms 内文件面板切到同目录（跟随开关打开时）。
2. 面板中输入路径回车、面包屑点击、`..` 返回均可用；无权限目录给出明确错误且不白屏。
3. 拖拽上传 100MB：进度事件 ≥ 1 次/秒，完成后大小/权限正确；局域网吞吐 ≥ 5MB/s。
4. 下载到本地：生成 `.part` 临时文件，完成后改名，失败可重试。
5. 修改权限 0644 → 0755 生效（`stat -c %a` 验证）；非法八进制被拒。
6. 在线编辑 `nginx.conf` 保存后，远端内容一致、原文件属主不变；外部同时改动时提示冲突而非覆盖。
7. 符号链接、空目录、隐藏文件、超长文件名（≥ 200 字符）展示与操作正常。

### 4.4 认证

1. ed25519 私钥（无口令 / 有口令）均可登录；口令错误提示明确。
2. RSA 私钥（PEM / OpenSSH 新格式）可登录。
3. `ssh-agent` 中存在可用身份时免密登录；无可用身份时提示“agent 中没有匹配身份”。
4. 密钥文件不存在/权限 0644/格式错误，分别给出可操作提示。

### 4.5 端口转发

1. `-L 8080:127.0.0.1:80`：本机 `curl http://127.0.0.1:8080` 返回后端响应。
2. `-R 9000:127.0.0.1:22`：服务器上 `nc -z 127.0.0.1 9000` 成功（受服务端 `AllowTcpForwarding` 约束时有明确报错）。
3. `-D 1080`：`curl --socks5 127.0.0.1:1080 http://目标` 成功。
4. 端口占用、绑定 0.0.0.0 的风险提示、规则重启后自动启动（开关打开时）均按预期。

### 4.6 监控

1. 2s 采样，CPU 使用率与 `top -bn1` 偏差 < 5%（负载稳定时）。
2. 内存、swap、负载 1/5/15、各挂载点磁盘、网卡 rx/tx 速率均有值；`/proc` 不可用时降级提示。
3. 断线后监控自动停止，重连后自动恢复，不产生僵尸采集任务。
4. 采集命令执行失败（如无 `df`）不影响终端会话。

### 4.7 稳定性与体积

1. 连续运行 24h、10 个会话 + 4 个转发，RSS 增长 < 50MB。
2. 冷启动到可操作 < 1.5s（M1/同级别机器）。
3. `.app` ≤ 15MB；DMG ≤ 10MB。
4. 优雅退出：关闭窗口时断开所有会话、停止转发、flush 日志，无残留进程。

## 5. 安全基线

| 项 | 要求 |
| --- | --- |
| 主机密钥 | 首次 TOFU 记录并提示；指纹变化**拒绝**连接；支持手动删除记录（known_hosts 管理入口） |
| 凭据 | 密码/私钥口令只进系统钥匙串；前端只能看到 `hasPassword/hasPassphrase` 布尔值 |
| 日志 | 禁止输出密码、私钥、passphrase；错误信息不得回显输入内容 |
| 转发 | `-D`/`-L` 默认绑定 `127.0.0.1`；绑定 `0.0.0.0` 需显式确认并红字提示 |
| 导入 | 解析失败不中断整体；不执行配置中的任何命令；不自动连接 |
| 编辑器 | 单文件 > 2MB 或含 NUL 字节拒绝内置编辑；保存前校验远端 mtime 防覆盖 |
| 依赖 | 发版前 `cargo audit` + `npm audit --omit=dev`；无高危未处置项 |
| CSP | 发布构建必须配置 CSP（A 期为 `null`，B 期收敛） |

## 6. 与 FinalShell 的有意差异

1. **不展示已保存密码**：FinalShell 允许查看保存的密码，NestTerm 永不回传明文。
2. **known_hosts 应用自有**：不写用户 `~/.ssh/known_hosts`（A 期为过渡实现，B 期迁移），避免污染系统文件。
3. **导入是单向兼容**：能读 FinalShell 导出，但不承诺反向导出可被 FinalShell 识别。
4. **不做“一键脚本市场”**：片段库只存命令文本，不内置任何第三方脚本。

## 7. Ubuntu 联调环境（B 期执行）

### 7.1 服务器一次性准备

```bash
# 1) SSH 服务与常用验收工具
sudo apt update
sudo apt install -y openssh-server vim tmux htop curl netcat-openbsd
sudo systemctl enable --now ssh

# 2) 专用测试账号（示例，勿用生产账号）
sudo adduser --disabled-password --gecos "" demo
sudo passwd nestterm                     # 设置一个临时密码
sudo mkdir -p /srv/nestterm && sudo chown nestterm:nestterm /srv/nestterm

# 3) 测试机允许密码登录（仅测试环境；测完建议改回）
sudo sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication yes/' /etc/ssh/sshd_config
sudo systemctl restart ssh

# 4) 确认版本与监听
lsb_release -a; ssh -V; ss -lntp | grep ':22'
```

密钥认证验收：由我方生成一次性 ed25519 密钥对（私钥只留在本机工程目录，测试结束删除），把公钥写入 `/home/demo/.ssh/authorized_keys`（`chmod 700 ~/.ssh && chmod 600 authorized_keys`）。

### 7.2 我需要你提供的信息（B 期开工时给一次即可）

| 项 | 说明 |
| --- | --- |
| 地址 / 端口 | 公网 IP、域名或内网 IP；本机（这台 Mac）能否直连 |
| 网络路径 | 直连 / NAT 端口映射 / Tailscale 等；若是内网，说明如何从本机访问 |
| 测试账号 | 建议 `nestterm` 这类一次性账号；**不要给生产账号** |
| 认证方式 | 临时密码 + 是否允许我写入一次性公钥 |
| 环境类型 | 物理机 / VM / 容器（容器内 `/proc`、`df`、`-R` 转发可能受限） |
| 权限 | 是否允许 `sudo`（P0 不需要，chmod 测试用 `/srv/nestterm` 即可） |
| 基线数据 | 带宽/延迟大致值（用于吞吐验收） |

### 7.3 测试结束清理

```bash
sudo deluser --remove-home nestterm
sudo rm -rf /srv/nestterm
# 恢复 sshd 配置中的 PasswordAuthentication（如原本为 no）
```

## 8. 术语表

| 术语 | 含义 |
| --- | --- |
| **Profile** | 持久化的连接配置（主机/端口/用户/认证方式），存 SQLite |
| **Group** | 连接分组（可嵌套），存 SQLite |
| **Session** | 一次运行中的 SSH 连接，`sessionId` 由前端生成，仅存活于内存 |
| **SftpSession** | 挂在同一 SSH 连接上的 SFTP 子系统句柄 |
| **ForwardRule** | 保存的转发规则；启动后产生运行中的 forward 实例 |
| **Transfer** | 一次上传/下载任务，带 `transferId` 与进度事件 |
| **KnownHost** | 应用自有的主机公钥记录（替代 `~/.ssh/known_hosts`） |
| **A 期 / B 期 …** | 交付批次：A 脚手架、B 模块、C 文档（本文）、D 打包、E 修复 |
