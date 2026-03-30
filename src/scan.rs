// src/scan.rs — ARP ベース LAN スキャン + ホスト名解決

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::thread;

#[derive(Clone, Debug)]
pub struct HostResult {
    pub ip:         Ipv4Addr,
    pub hostname:   Option<String>,
    pub latency_ms: f64,
    pub open_ports: Vec<u16>,
    pub web_port:   Option<u16>,
}

// ── ARP テーブルから既知ホストを取得 ──────────────────
/// `arp -a` の出力をパースして IP アドレス一覧を返す
pub fn arp_hosts(subnet_prefix: &str) -> Vec<Ipv4Addr> {
    let output = std::process::Command::new("arp")
        .arg("-a")
        .output();
    let Ok(output) = output else { return Vec::new() };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut hosts = Vec::new();
    for line in text.lines() {
        // 例: "  192.168.1.1          xx-xx-xx-xx-xx-xx     動的"
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() { continue; }
        let ip_str = parts[0];
        if !ip_str.starts_with(subnet_prefix) { continue; }
        if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
            hosts.push(ip);
        }
    }
    hosts
}

// ── ホスト名解決（nbtstat） ──────────────────────────
pub fn resolve_hostname_os(ip: Ipv4Addr) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("nbtstat")
            .args(["-A", &ip.to_string()])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let line = line.trim();
            if line.contains("<00>") && !line.contains("GROUP") {
                if let Some(name) = line.split_whitespace().next() {
                    if !name.is_empty() && name != "Name" {
                        return Some(name.to_string());
                    }
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    { let _ = ip; None }
}

// ── TCP ポートスキャン ────────────────────────────────
fn tcp_ping(ip: Ipv4Addr, port: u16, timeout: Duration) -> Option<f64> {
    let addr = SocketAddr::new(IpAddr::V4(ip), port);
    let t0 = Instant::now();
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(_) => Some(t0.elapsed().as_secs_f64() * 1000.0),
        Err(e) => match e.kind() {
            std::io::ErrorKind::ConnectionRefused =>
                Some(t0.elapsed().as_secs_f64() * 1000.0),
            _ => None,
        },
    }
}

const PROBE_PORTS: &[u16] = &[80, 443, 22, 445, 3389, 8080, 139, 21, 23, 5000, 8443, 8888];
const WEB_PORTS:  &[u16] = &[80, 8080, 443, 8443, 8888, 3000, 5000];

pub fn scan_host(ip: Ipv4Addr, timeout_ms: u64) -> Option<HostResult> {
    let timeout = Duration::from_millis(timeout_ms);
    let mut first_latency: Option<f64> = None;
    let mut open_ports = Vec::new();

    for &port in PROBE_PORTS {
        if let Some(ms) = tcp_ping(ip, port, timeout) {
            if first_latency.is_none() { first_latency = Some(ms); }
            let addr = SocketAddr::new(IpAddr::V4(ip), port);
            if TcpStream::connect_timeout(&addr, timeout).is_ok() {
                open_ports.push(port);
            }
        }
    }

    let latency = first_latency?;
    let web_port = WEB_PORTS.iter().find(|&&p| open_ports.contains(&p)).copied();
    let hostname = resolve_hostname_os(ip);

    Some(HostResult { ip, hostname, latency_ms: latency, open_ports, web_port })
}

// ── サブネットスキャン（ARP優先 + TCP補完） ─────────────
pub fn scan_subnet<FFound, FProg>(
    subnet: [u8; 3],
    timeout_ms: u64,
    found_cb: FFound,
    progress_cb: FProg,
    stop: Arc<std::sync::atomic::AtomicBool>,
)
where
    FFound: Fn(HostResult) + Send + Sync + 'static,
    FProg:  Fn(f32, String) + Send + Sync + 'static,  // (進捗, ステータス文字列)
{
    let prefix = format!("{}.{}.{}.", subnet[0], subnet[1], subnet[2]);

    // Phase 1: ARP テーブルから即座にリストアップ
    let arp_ips = arp_hosts(&prefix);
    let found_cb  = Arc::new(found_cb);
    let progress_cb = Arc::new(progress_cb);

    // ARP で見つかったホストを先に通知（latency=0 で仮登録）
    for ip in &arp_ips {
        if stop.load(std::sync::atomic::Ordering::Relaxed) { return; }
        let host = HostResult {
            ip: *ip,
            hostname: None,
            latency_ms: 0.0,
            open_ports: Vec::new(),
            web_port: None,
        };
        found_cb(host);
    }
    progress_cb(0.1, format!("ARPテーブル: {}台検出", arp_ips.len()));

    // Phase 2: 全 .1〜.254 を TCP スキャン（ARPになかったホストも検出）
    const TOTAL: u32 = 254;
    const BATCH: u32 = 32;
    let done_count = Arc::new(Mutex::new(0u32));
    // ARP 済み IP セット（重複通知防止）
    let arp_set: std::collections::HashSet<Ipv4Addr> = arp_ips.into_iter().collect();
    let arp_set = Arc::new(arp_set);

    let mut base = 1u32;
    while base <= TOTAL {
        if stop.load(std::sync::atomic::Ordering::Relaxed) { break; }
        let end = (base + BATCH - 1).min(TOTAL);
        let mut handles = Vec::new();

        for i in base..=end {
            let ip = Ipv4Addr::new(subnet[0], subnet[1], subnet[2], i as u8);
            let found_cb    = Arc::clone(&found_cb);
            let progress_cb = Arc::clone(&progress_cb);
            let done_count  = Arc::clone(&done_count);
            let stop        = Arc::clone(&stop);
            let arp_set     = Arc::clone(&arp_set);

            handles.push(thread::spawn(move || {
                if !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Some(mut result) = scan_host(ip, timeout_ms) {
                        // ARP で既に登録済みの場合は詳細情報で上書き通知
                        // （UI側で IP をキーに更新する）
                        result.latency_ms = result.latency_ms; // 実測値で上書き
                        found_cb(result);
                    } else if arp_set.contains(&ip) {
                        // ARP にはいるが TCP 応答なし → latency=—で表示はそのまま
                    }
                }
                let mut d = done_count.lock().unwrap();
                *d += 1;
                let pct = 0.1 + (*d as f32 / TOTAL as f32) * 0.9;
                progress_cb(pct, format!("{} / {} スキャン済", *d, TOTAL));
            }));
        }
        for h in handles { let _ = h.join(); }
        base = end + 1;
    }
}
