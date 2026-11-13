# cwd 跟随问题复盘（SFTP 面板不跟随 / 跳回 home）

> 归档时间：2026-10-27
> 状态：**已解决**（用户在正式服务器验证：cd 跟踪正确）
> 影响版本：`0.1.0` 的 B3 迭代包
> 相关文件：[TerminalPane.vue](../src/components/TerminalPane.vue)、[SftpPane.vue](../src/components/SftpPane.vue)、[sessions.ts](../src/stores/sessions.ts)、[ssh/session.rs](../src-tauri/src/ssh/session.rs)、[ssh/sftp.rs](../src-tauri/src/ssh/sftp.rs)

## 1. 问题现象

在正式服务器（`/home/demo/www`）上依次出现：

| 现象 | 表现 |
| --- | --- |
| 列目录失败 | 进入 `www` 提示 `读取目录失败 / No such file`，但终端 `ls` 正常 |
| 鼠标能进、cd 不能进 | 鼠标双击目录正常；终端 `cd` 后右侧不跟随 |
| 错误 cd 污染基准 | `cd aa`（不存在）后，下一次 `cd backend` 也被解析成错误路径 |
| cd 成功但面板不动 | 终端 `pwd` 已到 `www/html`，右侧仍停在 `www` |
| 面板刷新回 home | `cd` 成功后右侧先切到目标，又被一次迟到请求覆盖回 home |

关键事实：**终端 `cd` 成功只改变远端 shell 的 cwd；文件面板的 SFTP 列表是另一组异步请求。**

## 2. 根因不是单一 bug，而是四层叠加

### 2.1 错误的“权威来源”：输入启发式

最初 cwd 跟随依赖：

- OSC 7（很多默认 bash 不发）；
- 输入启发式：解析终端输入里的 `cd xxx`，用猜测的 `current` 拼出绝对路径，再让文件面板去 list。

输入启发式**只知道用户输入了什么，不知道 shell 是否真的切换成功**。它会产生：

- 语法合法但目录不存在的候选；
- 基于过期 `current` 的相对路径；
- 与真实 shell 状态不一致的事件。

这是后续“错误 cd 污染基准”“面板不跟随”的主要原因之一。

### 2.2 current 提交时机错误

早期实现中，解析器在 `cd` 语法合法时就更新 `current`，不等目录验证。  
`cd aa` 失败后，`current` 已变成 `/www/backend/aa`，下一次 `cd backend` 自然解析错。

后来一度改成“列表成功后提交”，但提交依赖前端 fire-and-forget 的 `sshHintCwd`，仍然存在时序问题。

### 2.3 异步 hint 与列表请求的竞态

- `sshHintCwd` 是异步 invoke；
- 连续 `cd` 时，hint 可能晚于下一条输入到达后端；
- 首屏 `openSession` 会发 home 列表请求，用户随后 `cd www` 又发一个列表请求；
- **两个列表请求的返回顺序不确定**：
  - `cd www` 的结果先返回 → 面板 www；
  - 首屏 home 的结果后返回 → 面板被覆盖回 home。

同步 current 不能解决这个问题，因为 current 只是变量，**不能让已发出的两个列表请求按顺序返回**。

### 2.4 服务器 SFTP 行为差异

正式服务器的 SFTP 对部分目录（疑似 symlink / 特殊目录）的 `OPENDIR/READDIR` 返回 `NO_SUCH_FILE`，但同一 SSH 会话的 shell `ls` 正常。

还遇到：

- `realpath` 失败会卡住 SFTP 会话；
- exec 通道不返回 `ExitStatus`，`test -d` 校验全部失败；
- 某些目录 `read_dir` 失败后会话需要重置。

这些不是应用逻辑能完全控制的，需要容错链。

## 3. 最终方案

### 3.1 以 shell 提示符为权威 cwd

在 xterm 每次输出后，解析当前行：

```
demo@host:~/www/backend$
```

- 支持 `user@host:~/path$` 与 `[user@host /abs/path]#`；
- 方括号提示符取最后一个空格后的路径；
- `~` 用 SFTP 的 home 展开；
- `setCwd(sessionId, path, 'prompt')`；
- 提示符路径是 **shell 命令执行后的真实结果**，不依赖输入猜测。

识别到提示符后，后续输入启发式事件被忽略（`promptSeen`）。

### 3.2 导航请求带序号，只接受最新结果

`SftpPane` 的每次 `navigate`：

```
seq = ++navigationSeq
list(path)
if (seq !== navigationSeq) return   // 过期结果丢弃
apply listing / path
```

首屏 home 的迟到结果不会再覆盖 `cd` 的新目录。

### 3.3 current 只由“被采用的最新列表”提交

- `sftp_list` 不再自己写 current；
- 前端只在 `seq` 校验通过后调用 `ssh_set_cwd`；
- 后端 `SshState` 维护每个 session 的 `cwd` 槽；
- pump 处理下一条输入前读取该槽，保证相对 `cd` 的基准是真实目录。

### 3.4 SFTP 列目录容错链

`sftp_list` 顺序：

1. `read_dir`；
2. 失败 → 重置 SFTP 会话（关闭旧 session）再试；
3. 仍失败 → `readlink` 取 symlink 目标，再对目标 `read_dir`；
4. 仍失败 → 同一 SSH 会话 `find -H -printf`（NUL 分隔）fallback；
5. 仍失败 → 返回带路径的错误；前端启发式导航静默保持原目录。

> `find` fallback 依赖服务器允许 exec 与 GNU find；不是所有服务器都可用。最终权威路径仍来自提示符 + 列表成功。

### 3.5 OSC 7：已删除

OSC 7 曾作为备选来源，但目标环境未使用且增加冗余路径；当前实现已彻底删除：

- 不再解析 `ESC ] 7 ; file://...`；
- 不再发送 `ssh:cwd` 事件；
- 不保留后端 cwd 槽/异步 hint。

## 4. “输入启发式”是什么（已移除）

历史实现位于 `src-tauri/src/ssh/cwd.rs::InputCwdTracker`，当前代码已删除。

它曾做：

1. 接收终端输入字节流；
2. 遇到 Enter，取一行；
3. 识别 `cd` / `cd ~` / `cd ../x` / `cd 'x'` 等简单形式；
4. 用内部 `current` 拼接绝对路径；
5. 发给前端，让 SFTP 面板去 list。

它**不是** shell 的 cwd，也**不执行**命令；只是“根据按键猜测用户可能去了哪”。这是它不可靠的原因。



代码位置：`src-tauri/src/ssh/cwd.rs` 的 `InputCwdTracker`。

工作方式：

1. 接收终端输入字节流；
2. 遇到 Enter，取一行；
3. 识别 `cd` / `cd ~` / `cd ../x` / `cd 'x'` 等简单形式；
4. 用内部 `current` 拼接绝对路径；
5. 发给前端，让 SFTP 面板去 list。

它**不是** shell 的 cwd，也**不执行**命令；只是“根据按键猜测用户可能去了哪”。

## 5. 它是“根本原因”吗？

准确说：**它是主要设计缺陷之一，但不是唯一根因。**

| 问题层 | 输入启发式责任 | 说明 |
| --- | --- | --- |
| 错误 cd 污染基准 | 直接责任 | 语法合法就提交 current，未验证目录 |
| 连续 cd 不跟随 | 直接责任 | 过期 current 拼出错误相对路径 |
| cd 成功面板不动 | 部分责任 | 启发式路径错误；也可能是 SFTP 列目录失败 |
| 刷新回 home | 间接 | 根因是“首屏 home 列表迟到 + 没有导航序号”，与启发式无直接关系 |
| 列目录失败 | 无 | 服务器 SFTP 行为 + 容错链问题 |

最终修复不是“把启发式改得更聪明”，而是**换掉权威来源**：只用提示符；输入启发式与 OSC 7 均已删除。并加请求序号，禁止过期结果覆盖。

## 6. 回归清单

在正式服务器验证：

```
cd www/       -> 右侧 www
cd html       -> 右侧 www/html
cd ..         -> 右侧 www
cd aa         -> 失败，右侧保持 www
cd backend    -> 右侧 www/backend
```

鼠标双击目录、返回上层、隐藏文件切换、重连后 SFTP 重开也应正常。

自动化状态：

- Rust：`cargo test --lib` → 26 passed / 4 ignored；
- 前端提示符解析暂无单测（人工验证通过）；
- 原生拖拽/菜单等仍需人工验证。

## 7. 代码改动索引

| 文件 | 改动 |
| --- | --- |
| `src/components/TerminalPane.vue` | 解析提示符 → `setCwd(..., 'prompt')` |
| `src/components/SftpPane.vue` | `navigationSeq`、过期结果丢弃、`ssh_set_cwd` |
| `src/stores/sessions.ts` | `home`、`promptSeen`、`prompt` 来源 |
| `src-tauri/src/ssh/session.rs` | 会话读写循环（不含 cwd 推断） |
| `src-tauri/src/ssh/sftp.rs` | 列目录容错链、`ssh_set_cwd` 前不再自动提交 |
| `src-tauri/src/ssh/cwd.rs` | 已删除 |

## 8. 长期约定（已写入 AGENTS.md）

- 以后所有类似的 SSH/SFTP/运维工具：**cwd 只用 shell 提示符，不用输入启发式/OSC 7**。
- 识别不到提示符时不猜，提示用户配置或仅用 OSC 7。
- 导航请求必须带序号，过期结果不写 UI/cwd。
- 详细规则：[AGENTS.md](../AGENTS.md)。

## 9. 经验

1. **不要用输入推断 shell 状态**；shell 的提示符/OSC 7 才是权威。
2. 异步结果必须带 generation/seq，旧结果不能写 UI。
3. 单一事实来源：cwd 只由一个地方提交，避免 hint、OSC 7、鼠标导航互相覆盖。
4. 对服务器差异做容错链，但不要用“猜测”掩盖失败；失败应静默保持当前状态。
5. 远端工具优先复用同一 SSH 连接；SFTP 失败时 readlink/find 只是 fallback，不是主路径。
