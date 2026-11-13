# FinalShell 本地配置导入：实测字段映射

> 状态：**macOS FinalShell 4.6.5 已实测**（2026-10-26）
> 关联：[IPC 契约](02-ipc-contract.md) · [数据库 profiles](03-database-schema.md)

**结论：FinalShell 4.6.5 没有导出功能，但连接配置是明文 JSON，可直接扫描，不需要“脱敏样本”。**

---

## 1. 数据位置（macOS 实测）

```
~/Library/FinalShell/
├─ conn/<id>_connect_config.json   # 每个连接一个 JSON
├─ config.json                     # 应用设置（不含连接列表）
├─ knownhosts.json                 # FinalShell 自己的 known_hosts
├─ backup/                         # 历史备份
└─ deleted/                        # 删除记录
```

- 只需 `conn/`。`config.json` 不包含连接，不要拿它当导入源。
- 当前机器实测：3 个 `*_connect_config.json`，解析 3 条连接，0 条告警。
- Windows / Linux / 其它 FinalShell 版本**未实测**；如果路径不同，可在 IPC 层传 `dir` 覆盖（`finalshell_preview` / `finalshell_import` 的 `dir` 参数）。

## 2. 实测字段映射

| FinalShell 字段 | 示例/类型 | NestTerm 字段 | 处理 |
| --- | --- | --- | --- |
| `name` | string（可中文） | `Profile.name` | 保留；空则用 host |
| `host` | string | `Profile.host` | 保留 |
| `port` | int | `Profile.port` | 保留 |
| `user_name` | string | `Profile.username` | 空则 `root` |
| `description` | string | `Profile.notes` | 保留，追加“从 FinalShell 导入”说明 |
| `password` | 32 字符 base64 | — | **不导入**（加密/不可逆），首次连接重输 |
| `authentication_type` | 1 = 密码 | `Profile.authType` | 统一按 `password` 导入；非 1 给 warning |
| `terminal_encoding` | `"UTF-8"` | `Profile.charset` | 转小写；空则 `utf-8` |
| `parent_id` | `"root"` / 分组 id | `Group` | 当前样本全为 root；分组元数据缺失 → 统一放入「FinalShell 导入」 |
| `conection_type` | 100 = SSH | — | 100 才导入；其它类型 warning 跳过 |
| `id` / 时间戳 / 转发字段 | — | — | 忽略 |

> 字段名 `conection_type` 是 FinalShell 的原始拼写（少一个 n），解析时按原样读取。

## 3. 密码与私钥

- `password` 是 32 字符 base64，**本项目不尝试解密**（不做逆向/破解）。
- 导入后 `hasPassword=false`，首次连接会弹窗要求输入密码。
- 私钥路径字段在当前样本中不存在（样本全是密码认证）；如后续版本提供，需新增映射并实测。

## 4. 分组

- 当前 3 条连接的 `parent_id` 都是 `"root"`，`conn/` 中没有分组对象。
- 导入策略：全部放入自动创建的「FinalShell 导入」分组，保留连接名与备注。
- 如你有多级分组，请提供**带分组的 `conn/` 目录副本**（脱敏后），我再补分组映射；在此之前不猜测格式。

## 5. 导入规则

1. 只读扫描 `conn/*_connect_config.json`，不执行任何内容。
2. 按 `(host, port, username)` 去重：目标 DB 已存在 → 跳过。
3. 单条解析失败只记 warning，不中断整批。
4. 名称重复不阻断（NestTerm 允许同名连接）。
5. 导入可重复执行：第二次会全部走“已存在，跳过”（已实测）。

## 6. 实测结果（2026-10-26）

在本机 `~/Library/FinalShell/conn` 上：

```
connections = 3
with_password = 3
groups_detected = 0
warnings = 0
```

- 首次导入：创建 3 条（Demo A / Demo B / Demo C），状态栏连接数 7 → 10。
- 二次导入：3 条全部跳过（幂等）。
- 密码未导入；host/port/user/name/notes 均正确。
- 分组「FinalShell 导入」自动创建并复用。

## 7. 如果你仍想提供样本（其它平台/版本）

不需要专有导出格式，**直接给 `conn/*_connect_config.json`**：

- 最省事：在 FinalShell 里新建 2–3 个假连接，主机用 `192.0.2.10` / `198.51.100.20`，用户名 `demo`，密码 `demo-password`；
- 把 `conn/` 下生成的 JSON 复制出来（或整目录 zip）；
- 或者手工脱敏：保留字段名/结构，把 host、user、password、description 替换成假值；
- 放到工程 `testdata/finalshell/`（已 gitignore），或直接贴 JSON；
- **不要发真实私钥内容**；密码即使是加密值也不必提供真实值。

## 8. 已知限制

1. 分组结构未导入（当前样本无分组元数据）。
2. 密码/私钥不迁移，首次连接需重输。
3. Windows/Linux 路径未实测，需要样本校准。
4. 只扫描当前 `conn/`，不导入 `backup/` / `deleted/`（避免重复与误删恢复）。
5. 不读取 FinalShell 的 `knownhosts.json` 来预置信任（安全考虑，首次连接仍走 TOFU）。
