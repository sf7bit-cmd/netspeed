// src/measure.rs

use std::time::{Duration, Instant};

const DOWN_URL: &str = "https://speed.cloudflare.com/__down?bytes=10000000";
const UP_URL:   &str = "https://speed.cloudflare.com/__up";
const PING_URL: &str = "https://speed.cloudflare.com/__down?bytes=1";

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("NetSpeedAnalyzer/0.1")
        .build()
        .expect("HTTP client error")
}

pub struct PingResult {
    pub avg: f64,
    pub min: f64,
    pub max: f64,
    pub jitter: f64,
}

pub fn measure_ping(n: usize) -> Option<PingResult> {
    let c = client();
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        let t0 = Instant::now();
        if c.head(PING_URL).send().is_ok() {
            samples.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if samples.is_empty() { return None; }
    let avg = samples.iter().sum::<f64>() / samples.len() as f64;
    let min = samples.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = samples.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let jitter = if samples.len() > 1 {
        samples.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>()
            / (samples.len() - 1) as f64
    } else { 0.0 };
    Some(PingResult { avg, min, max, jitter })
}

pub fn measure_download<F>(samples: usize, mut on_sample: F) -> Option<f64>
where F: FnMut(f64) {
    let c = client();
    let mut results = Vec::with_capacity(samples);
    for i in 0..samples {
        let url = format!("{}?r={}", DOWN_URL, i);
        let t0 = Instant::now();
        match c.get(&url).send().and_then(|r| r.bytes()) {
            Ok(bytes) => {
                let elapsed = t0.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    let mbps = (bytes.len() as f64 * 8.0) / elapsed / 1_000_000.0;
                    results.push(mbps);
                    on_sample(mbps);
                }
            }
            Err(_) => {}
        }
    }
    if results.is_empty() { return None; }
    Some(results.iter().sum::<f64>() / results.len() as f64)
}

pub fn measure_upload<F>(samples: usize, mut on_sample: F) -> Option<f64>
where F: FnMut(f64) {
    use rand::RngCore;
    let c = client();
    let mut results = Vec::with_capacity(samples);
    let size = 2 * 1024 * 1024usize;
    for _ in 0..samples {
        let mut data = vec![0u8; size];
        rand::thread_rng().fill_bytes(&mut data);
        let t0 = Instant::now();
        let _ = c.post(UP_URL).body(data).send();
        let elapsed = t0.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            let mbps = (size as f64 * 8.0) / elapsed / 1_000_000.0;
            results.push(mbps);
            on_sample(mbps);
        }
    }
    if results.is_empty() { return None; }
    Some(results.iter().sum::<f64>() / results.len() as f64)
}
