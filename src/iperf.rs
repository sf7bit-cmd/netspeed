// src/iperf.rs — LAN内スループット測定（内蔵サーバー/クライアント）

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 15101;
pub const TEST_DURATION_SECS: u64 = 5;
const CHUNK_SIZE: usize = 128 * 1024; // 128KB

#[derive(Clone, Debug, Default)]
pub struct IperfResult {
    pub mbps:      f64,
    pub bytes:     u64,
    pub duration:  f64,
    pub direction: String,
}

/// サーバーモード：接続を待ち受けてデータを受信、速度を返す
pub fn run_server(stop: Arc<AtomicBool>) -> Option<IperfResult> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", DEFAULT_PORT)).ok()?;
    listener.set_nonblocking(true).ok()?;

    // 接続待ち（最大10秒）
    let wait_start = Instant::now();
    let mut stream = loop {
        if stop.load(Ordering::Relaxed) { return None; }
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if wait_start.elapsed().as_secs() > 10 { return None; }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    };

    stream.set_nonblocking(false).ok()?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut total_bytes = 0u64;
    let t0 = Instant::now();

    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => { total_bytes += n as u64; }
            Err(_) => break,
        }
        if t0.elapsed().as_secs() > TEST_DURATION_SECS + 2 { break; }
    }

    let elapsed = t0.elapsed().as_secs_f64();
    if elapsed < 0.1 { return None; }

    Some(IperfResult {
        mbps:      (total_bytes as f64 * 8.0) / elapsed / 1_000_000.0,
        bytes:     total_bytes,
        duration:  elapsed,
        direction: "RX (server)".into(),
    })
}

/// クライアントモード：指定IPにデータを送信して速度を計測
pub fn run_client(target_ip: &str, stop: Arc<AtomicBool>) -> Option<IperfResult> {
    let addr = format!("{}:{}", target_ip, DEFAULT_PORT);
    let mut stream = TcpStream::connect_timeout(
        &addr.parse().ok()?,
        Duration::from_secs(5),
    ).ok()?;

    let buf = vec![0xABu8; CHUNK_SIZE];
    let mut total_bytes = 0u64;
    let t0 = Instant::now();
    let duration = Duration::from_secs(TEST_DURATION_SECS);

    while t0.elapsed() < duration {
        if stop.load(Ordering::Relaxed) { break; }
        match stream.write(&buf) {
            Ok(n) => { total_bytes += n as u64; }
            Err(_) => break,
        }
    }
    drop(stream);

    let elapsed = t0.elapsed().as_secs_f64();
    if elapsed < 0.1 { return None; }

    Some(IperfResult {
        mbps:      (total_bytes as f64 * 8.0) / elapsed / 1_000_000.0,
        bytes:     total_bytes,
        duration:  elapsed,
        direction: "TX (client)".into(),
    })
}
