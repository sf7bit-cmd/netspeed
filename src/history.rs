// src/history.rs

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct HistoryEntry {
    pub timestamp:  String,
    pub dl_mbps:    Option<f64>,
    pub ul_mbps:    Option<f64>,
    pub ping_ms:    Option<f64>,
    pub jitter_ms:  Option<f64>,
}

pub fn history_path(ext: &str) -> PathBuf {
    let mut p = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    p.pop();
    p.push(format!("netspeed_history.{}", ext));
    p
}

fn timestamp_str() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    let year  = 1970 + days / 365;
    let doy   = days % 365;
    let month = doy / 30 + 1;
    let day   = doy % 30 + 1;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", year, month, day, h, m, s)
}

pub fn save_csv(entries: &[HistoryEntry]) -> Result<PathBuf, String> {
    let path = history_path("csv");
    let mut out = String::from("timestamp,dl_mbps,ul_mbps,ping_ms,jitter_ms\n");
    for e in entries {
        out += &format!(
            "{},{},{},{},{}\n",
            e.timestamp,
            e.dl_mbps.map(|v| format!("{:.2}", v)).unwrap_or_default(),
            e.ul_mbps.map(|v| format!("{:.2}", v)).unwrap_or_default(),
            e.ping_ms.map(|v| format!("{:.2}", v)).unwrap_or_default(),
            e.jitter_ms.map(|v| format!("{:.2}", v)).unwrap_or_default(),
        );
    }
    fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn save_json(entries: &[HistoryEntry]) -> Result<PathBuf, String> {
    let path = history_path("json");
    let json = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn new_entry(dl: Option<f64>, ul: Option<f64>, ping: Option<f64>, jitter: Option<f64>) -> HistoryEntry {
    HistoryEntry {
        timestamp: timestamp_str(),
        dl_mbps:   dl,
        ul_mbps:   ul,
        ping_ms:   ping,
        jitter_ms: jitter,
    }
}
