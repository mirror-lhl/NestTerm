# NestTerm 工程约定（Agent Memory）

> 这是给后续 AI/开发者的持久约定。需求冲突时，以本文件为准，并在改动前说明偏差。

## 1. cwd 跟随：只用 shell 提示符，不用输入启发式 / OSC 7

**适用范围：NestTerm 以及以后所有类似的 SSH / SFTP / 运维工具。**

- **禁止**通过解析用户键盘输入（`cd xxx`）来推断远端 cwd。
- cwd 的权威来源只能是：
  1. **shell 提示符**，例如 `user@host:~/www/backend$`；
  2. 用户显式指定的路径。
- 识别不到提示符时：**不要猜**，提示用户配置提示符格式；不得退回输入启发式。
- **OSC 7 已从当前实现删除**（目标环境未使用）。没有明确需求与实测前，不要重新引入。
- 提示符解析要求：
  - 在终端输出写入后解析当前行，去掉 ANSI；
  - 支持 `user@host:~/path$` 与 `[user@host /abs/path]#`；
  - `~` 用已通过 SFTP 获取的 home 展开；
  - 只接受绝对路径；裸目录名（CentOS `\W`）不跟随，提示改为 `\w`；
  - 提示符路径变化才触发，避免重复刷新。
- CentOS 默认 `\W` 只显示目录名：检测到裸目录名提示符时，在同一会话自动执行一次
  `PS1='[\u@\h \w]\$ '`，升级为完整路径；只影响当前 shell，不修改远端 `.bashrc`。
- 禁止把“`cd` 语法合法”当成“`cd` 执行成功”。

## 2. 异步列表请求必须有序

- 每次目录导航带递增 generation / sequence。
- 列表返回后先校验是否仍是最新请求；过期结果**不更新 UI、不提交 cwd**。
- 首屏 home 请求、鼠标导航、提示符导航可能并发，必须能安全丢弃迟到结果。

## 3. cwd 单一事实来源

- cwd 只由“被采用的最新列表结果”提交。
- 不要同时用异步 hint、输入解析、鼠标导航各自写 cwd。
- 相对路径的基准必须来自该权威 cwd，而不是猜测值。
- 错误 `cd` 不应改变 cwd，也不应改变文件面板。

## 4. 远端容错

- SFTP 列目录失败时，优先重置 SFTP 子系统并重试；再考虑 `readlink` / `find` fallback。
- fallback 只是容错，不是主路径；不要用 fallback 掩盖未知路径。
- 文件操作仍走 SFTP；fallback 只用于列目录。
- 所有错误信息带路径，便于定位。

## 5. 本项目的当前实现（历史说明）

- 早期 `InputCwdTracker` 输入启发式已从当前代码移除。
- 当前 NestTerm 只使用 shell 提示符（`TerminalPane.vue` 的 `detectPrompt`），并加导航序号。
- 新增功能不得重新引入输入启发式。

## 6. 回归验证

至少覆盖：

```
cd www/       -> 右侧 www
cd html       -> 右侧 www/html
cd ..         -> 右侧 www
cd aa         -> 失败，右侧保持 www
cd backend    -> 右侧 www/backend
```

并验证首屏 home 迟到不会覆盖新目录、鼠标导航、重连后 SFTP 重开。

更完整的复盘见 [docs/cwd-follow-postmortem.md](docs/cwd-follow-postmortem.md)。
