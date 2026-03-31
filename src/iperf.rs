// src/iperf.rs — SMB Write→Read sequential throughput test

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};

pub const TEST_DURATION_SECS: u64 = 5;
const CHUNK_SIZE: usize = 1024 * 1024; // 1MB

#[derive(Clone, Debug, Default)]
pub struct IperfResult {
    pub write_mbps: f64,
    pub read_mbps:  f64,
    pub bytes:      u64,
    pub duration:   f64,
    pub path:       String,
}

fn tmp_path(base: &str) -> PathBuf {
    let mut p = PathBuf::from(base);
    p.push(format!("netspeed_bench_{}.tmp", std::process::id()));
    p
}

fn progress_mbps(bytes: u64, elapsed: f64) -> f64 {
    if elapsed < 0.001 { return 0.0; }
    (bytes as f64 * 8.0) / elapsed / 1_000_000.0
}

/// Write → Read の連続測定
pub fn measure_smb(
    smb_dir: &str,
    stop: Arc<AtomicBool>,
    on_progress: impl Fn(f64, &str) + Send + 'static, // (Mbps, phase)
) -> Result<IperfResult, String> {
    let path = tmp_path(smb_dir);

    // ── Phase 1: Write ──────────────────────────────
    {
        let mut f = std::fs::File::create(&path)
            .map_err(|e| format!("Cannot create file: {}", e))?;
        let chunk = vec![0xABu8; CHUNK_SIZE];
        let mut total = 0u64;
        let t0 = Instant::now();
        let mut last = Instant::now();

        while t0.elapsed() < Duration::from_secs(TEST_DURATION_SECS) {
            if stop.load(Ordering::Relaxed) {
                drop(f);
                let _ = std::fs::remove_file(&path);
                return Err("Stopped".into());
            }
            f.write_all(&chunk).map_err(|e| format!("Write error: {}", e))?;
            total += CHUNK_SIZE as u64;
            if last.elapsed().as_millis() >= 400 {
                on_progress(progress_mbps(total, t0.elapsed().as_secs_f64()), "Write");
                last = Instant::now();
            }
        }
        let _ = f.flush();
    }

    let write_bytes = std::fs::metadata(&path)
        .map(|m| m.len()).unwrap_or(0);
    let write_mbps = {
        let f = std::fs::File::open(&path).ok();
        // re-measure from file size
        (write_bytes as f64 * 8.0) / TEST_DURATION_SECS as f64 / 1_000_000.0
    };

    on_progress(write_mbps, "Write done");

    // ── Phase 2: Read ───────────────────────────────
    let read_mbps = {
        let mut f = std::fs::File::open(&path)
            .map_err(|e| format!("Cannot open file: {}", e))?;
        let mut buf = vec![0u8; CHUNK_SIZE];
        let mut total = 0u64;
        let t0 = Instant::now();
        let mut last = Instant::now();

        loop {
            if stop.load(Ordering::Relaxed) { break; }
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => { total += n as u64; }
                Err(e) => {
                    let _ = std::fs::remove_file(&path);
                    return Err(format!("Read error: {}", e));
                }
            }
            if last.elapsed().as_millis() >= 400 {
                on_progress(progress_mbps(total, t0.elapsed().as_secs_f64()), "Read");
                last = Instant::now();
            }
        }
        let elapsed = t0.elapsed().as_secs_f64();
        on_progress(progress_mbps(total, elapsed), "Read done");
        progress_mbps(total, elapsed.max(0.001))
    };

    let _ = std::fs::remove_file(&path);

    Ok(IperfResult {
        write_mbps,
        read_mbps,
        bytes:    write_bytes,
        duration: TEST_DURATION_SECS as f64 * 2.0,
        path:     smb_dir.to_string(),
    })
}
