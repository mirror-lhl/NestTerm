//! Linux 远程监控采样（/proc + df），每次采样开一个独立 exec channel。
//!
//! 不常驻后台任务：面板可见时前端每 2s 调一次 `monitor_sample`；
//! 面板关闭就不再采样，避免空转和额外连接。

use std::time::Duration;

use russh::client::Handle;
use russh::ChannelMsg;
use serde::Serialize;

use super::handler::ClientHandler;
use crate::db::models::now_ms;
use crate::error::{ApiError, ApiResult, ErrorCode};

/// 一次采样脚本：CPU 取两次 200ms 差值，避免用开机以来的累计值。
const SCRIPT: &str = r#"printf '@@LOAD\n'; cat /proc/loadavg
printf '@@CPU\n'; awk '/^cpu /{print}' /proc/stat; sleep 0.2; awk '/^cpu /{print}' /proc/stat
printf '@@MEM\n'; grep -E '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' /proc/meminfo
printf '@@DISK\n'; df -kP / | tail -n 1
printf '@@UPTIME\n'; cut -d' ' -f1 /proc/uptime
printf '@@HOST\n'; hostname; uname -sr
printf '@@NET\n'; cat /proc/net/dev"#;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorStats {
    pub host: String,
    pub os: String,
    pub uptime_secs: u64,
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    pub cpu_percent: f64,
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_percent: f64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub disk_total: u64,
    pub disk_used: u64,
    pub disk_percent: f64,
    pub disk_mount: String,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub sampled_at: i64,
}

pub async fn sample(handle: &Handle<ClientHandler>) -> ApiResult<MonitorStats> {
    let mut channel = handle.channel_open_session().await.map_err(|error| {
        ApiError::new(ErrorCode::SshChannel, "打开监控 channel 失败").detail(error.to_string())
    })?;
    channel.exec(true, SCRIPT).await.map_err(|error| {
        ApiError::new(ErrorCode::MonitorUnsupported, "远端无法执行监控脚本")
            .detail(error.to_string())
    })?;

    let mut output = Vec::new();
    let read = async {
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } => output.extend_from_slice(&data),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(8), read)
        .await
        .map_err(|_| ApiError::new(ErrorCode::MonitorTimeout, "监控采样超时"))?;

    parse(&String::from_utf8_lossy(&output))
}

fn parse_number<T: std::str::FromStr>(value: &str) -> Option<T> {
    value.trim().parse().ok()
}

fn parse_cpu(line: &str) -> Option<(u64, u64)> {
    let numbers: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .filter_map(parse_number)
        .collect();
    if numbers.len() < 5 {
        return None;
    }
    let total: u64 = numbers.iter().sum();
    let idle = numbers[3] + numbers[4];
    Some((total.saturating_sub(idle), total))
}

fn parse_stats(output: &str) -> ApiResult<MonitorStats> {
    let mut section = "";
    let mut load = Vec::<f64>::new();
    let mut cpu = Vec::<(u64, u64)>::new();
    let mut mem = std::collections::HashMap::<String, u64>::new();
    let mut disk_total = 0_u64;
    let mut disk_used = 0_u64;
    let mut disk_percent = 0_f64;
    let mut disk_mount = "/".to_string();
    let mut uptime = 0_u64;
    let mut host = String::new();
    let mut os = String::new();
    let mut net_rx = 0_u64;
    let mut net_tx = 0_u64;
    let mut host_lines = 0_usize;

    for line in output.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            section = name;
            continue;
        }
        let line = line.trim_end();
        match section {
            "LOAD" => {
                for value in line.split_whitespace().take(3) {
                    if let Some(value) = parse_number::<f64>(value) {
                        load.push(value);
                    }
                }
            }
            "CPU" => {
                if let Some(values) = parse_cpu(line) {
                    cpu.push(values);
                }
            }
            "MEM" => {
                if let Some((key, value)) = line.split_once(':') {
                    if let Some(value) = value
                        .split_whitespace()
                        .next()
                        .and_then(parse_number::<u64>)
                    {
                        mem.insert(key.trim().to_string(), value * 1024);
                    }
                }
            }
            "DISK" => {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.len() >= 6 {
                    disk_total = parse_number::<u64>(fields[1]).unwrap_or(0) * 1024;
                    disk_used = parse_number::<u64>(fields[2]).unwrap_or(0) * 1024;
                    disk_percent = fields[4].trim_end_matches('%').parse().unwrap_or(0.0);
                    disk_mount = fields[5].to_string();
                }
            }
            "UPTIME" => uptime = parse_number(line).unwrap_or(0),
            "HOST" => {
                if host_lines == 0 {
                    host = line.trim().to_string();
                } else if host_lines == 1 {
                    os = line.trim().to_string();
                }
                host_lines += 1;
            }
            "NET" => {
                if let Some((name, rest)) = line.split_once(':') {
                    if name.trim() == "lo" {
                        continue;
                    }
                    let fields: Vec<&str> = rest.split_whitespace().collect();
                    if fields.len() >= 9 {
                        net_rx += parse_number::<u64>(fields[0]).unwrap_or(0);
                        net_tx += parse_number::<u64>(fields[8]).unwrap_or(0);
                    }
                }
            }
            _ => {}
        }
    }

    if cpu.len() < 2 || mem.is_empty() {
        return Err(ApiError::new(
            ErrorCode::MonitorUnsupported,
            "远端不是 Linux / 缺少 /proc 监控数据",
        ));
    }
    let (busy1, total1) = cpu[0];
    let (busy2, total2) = cpu[cpu.len() - 1];
    let total_delta = total2.saturating_sub(total1);
    let cpu_percent = if total_delta == 0 {
        0.0
    } else {
        (busy2.saturating_sub(busy1) as f64 / total_delta as f64) * 100.0
    };

    let mem_total = *mem.get("MemTotal").unwrap_or(&0);
    let mem_available = *mem.get("MemAvailable").unwrap_or(&0);
    let mem_used = mem_total.saturating_sub(mem_available);
    let mem_percent = if mem_total == 0 {
        0.0
    } else {
        mem_used as f64 / mem_total as f64 * 100.0
    };
    let swap_total = *mem.get("SwapTotal").unwrap_or(&0);
    let swap_used = swap_total.saturating_sub(*mem.get("SwapFree").unwrap_or(&0));

    Ok(MonitorStats {
        host,
        os,
        uptime_secs: uptime,
        load1: load.first().copied().unwrap_or(0.0),
        load5: load.get(1).copied().unwrap_or(0.0),
        load15: load.get(2).copied().unwrap_or(0.0),
        cpu_percent,
        mem_total,
        mem_used,
        mem_percent,
        swap_total,
        swap_used,
        disk_total,
        disk_used,
        disk_percent,
        disk_mount,
        net_rx_bytes: net_rx,
        net_tx_bytes: net_tx,
        sampled_at: now_ms(),
    })
}

fn parse(output: &str) -> ApiResult<MonitorStats> {
    parse_stats(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "@@LOAD\n0.10 0.20 0.30 1/100 12345\n@@CPU\ncpu 100 0 100 800 0 0 0 0 0 0\ncpu 200 0 200 1600 0 0 0 0 0 0\n@@MEM\nMemTotal: 1000 kB\nMemAvailable: 400 kB\nSwapTotal: 200 kB\nSwapFree: 150 kB\n@@DISK\n/dev/sda1 1000 600 400 60% /\n@@UPTIME\n123.45\n@@HOST\ntest-host\nLinux 6.8.0\n@@NET\nInter-| Receive | Transmit\n face |bytes packets ...\neth0: 1000 1 0 0 0 0 0 0 2000 2 0 0 0 0 0 0\nlo: 999 1 0 0 0 0 0 0 999 2 0 0 0 0 0 0\n";

    #[test]
    fn parses_proc_sample() {
        let stats = parse(SAMPLE).expect("stats");
        assert_eq!(stats.host, "test-host");
        assert_eq!(stats.os, "Linux 6.8.0");
        assert_eq!(stats.load1, 0.10);
        assert_eq!(stats.cpu_percent, 20.0);
        assert_eq!(stats.mem_total, 1_024_000);
        assert_eq!(stats.mem_used, 614_400);
        assert_eq!(stats.swap_used, 51_200);
        assert_eq!(stats.disk_total, 1_024_000);
        assert_eq!(stats.disk_used, 614_400);
        assert_eq!(stats.disk_mount, "/");
        assert_eq!(stats.net_rx_bytes, 1000);
        assert_eq!(stats.net_tx_bytes, 2000);
    }

    #[test]
    fn rejects_non_linux_output() {
        assert!(parse("@@HOST\nmac\nDarwin\n").is_err());
    }
}
