//! SSH 子系统。
//!
//! 目录约定：
//! - `commands`  Tauri 命令（前端调用入口）
//! - `session`   会话表 + 读写循环
//! - `handler`   russh 客户端回调（主机密钥 / banner）
//! - `events`    后端 → 前端事件（事件名与 payload 结构集中定义）

pub mod commands;
mod events;
pub mod forward;
pub mod forward_commands;
mod handler;
pub mod monitor;
pub mod monitor_commands;
pub mod reconnect;
mod session;
pub mod sftp;
pub mod transfer;

pub use session::SshState;
