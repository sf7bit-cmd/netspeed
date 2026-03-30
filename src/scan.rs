// src/scan.rs — LAN ホストスキャン (TCP connect + ICMP fallback)

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::thread;

#[derive(Clone, Debug)]
pub struct HostResult {
    pub ip: Ipv4Addr,
    pub latency_ms: f64,
    pub open_ports: Vec<u16>,
}

/// ポートへの TCP 接続でホストの生存確認
fn tcp_ping(ip: Ipv4Addr, port: u16, timeout: Duration) -> Option<f64> {
    let addr = SocketAddr::new(IpAddr::V4(ip), port);
    let t0 = Instant::now();
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(_)  => Some(t0.elapsed().as_secs_f64() * 1000.0),
        Err(e) => {
            // "Connection refused" = ポートは閉じているがホストは存在
            use std::io::ErrorKind::*;
            match e.kind() {
                ConnectionRefused => Some(t0.elapsed().as_secs_f64() * 1000.0),
                _ => None,
            }
        }
    }
}

/// よく使われるポートリスト
const PROBE_PORTS: &[u16] = &[80, 443, 22, 445, 3389, 8080, 139, 21, 23, 5000];

pub fn scan_host(ip: Ipv4Addr, timeout_ms: u64) -> Option<HostResult> {
    let timeout = Duration::from_millis(timeout_ms);
    let mut first_latency: Option<f64> = None;
    let mut open_ports = Vec::new();

    for &port in PROBE_PORTS {
        if let Some(ms) = tcp_ping(ip, port, timeout) {
            if first_latency.is_none() {
                first_latency = Some(ms);
            }
            // ポートが実際に open か (refused でない) チェック
            let addr = SocketAddr::new(IpAddr::V4(ip), port);
            if TcpStream::connect_timeout(&addr, timeout).is_ok() {
                open_ports.push(port);
            }
            break; // 1ポートで alive 確認できたら十分
        }
    }

    first_latency.map(|ms| HostResult { ip, latency_ms: ms, open_ports })
}

/// サブネット全体をスキャン。found_cb で発見のたびに通知。progress_cb で進捗 0.0–1.0 通知。
pub fn scan_subnet<FFound, FProg>(
    subnet: [u8; 3],
    timeout_ms: u64,
    found_cb: FFound,
    progress_cb: FProg,
    stop: Arc<std::sync::atomic::AtomicBool>,
)
where
    FFound: Fn(HostResult) + Send + Sync + 'static,
    FProg:  Fn(f32)        + Send + Sync + 'static,
{
    const TOTAL: u32 = 254;
    const BATCH: u32 = 32;

    let found_cb  = Arc::new(found_cb);
    let progress_cb = Arc::new(progress_cb);
    let done_count = Arc::new(Mutex::new(0u32));

    let mut base = 1u32;
    while base <= TOTAL {
        if stop.load(std::sync::atomic::Ordering::Relaxed) { break; }

        let end = (base + BATCH - 1).min(TOTAL);
        let mut handles = Vec::new();

        for i in base..=end {
            let ip = Ipv4Addr::new(subnet[0], subnet[1], subnet[2], i as u8);
            let found_cb   = Arc::clone(&found_cb);
            let progress_cb = Arc::clone(&progress_cb);
            let done_count = Arc::clone(&done_count);
            let stop = Arc::clone(&stop);

            handles.push(thread::spawn(move || {
                if !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Some(result) = scan_host(ip, timeout_ms) {
                        found_cb(result);
                    }
                }
                let mut d = done_count.lock().unwrap();
                *d += 1;
                progress_cb(*d as f32 / TOTAL as f32);
            }));
        }
        for h in handles { let _ = h.join(); }
        base = end + 1;
    }
}
