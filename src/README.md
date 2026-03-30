// src/iperf.rs — SMB共有フォルダへの書き込みでLANスループット測定

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Instant;

pub const TEST_DURATION_SECS: u64 = 5;
const CHUNK_SIZE: usize = 1024 * 1024; // 1MB チャンク

#[derive(Clone, Debug, Default)]
pub struct IperfResult {
    pub mbps:      f64,
    pub bytes:     u64,
    pub duration:  f64,
    pub direction: String,
    pub path:      String,
}

/// SMB共有への書き込み速度測定
/// path例: \\192.168.1.10\share  または  Z:\  (マウント済みドライブ)
pub fn measure_smb_write(
    smb_path: &str,
    stop: Arc<AtomicBool>,
    on_progress: impl Fn(f64),  // 現在の速度(Mbps)を通知
) -> Result<IperfResult, String> {
    let mut base = PathBuf::from(smb_path);
    if !base.exists() {
        return Err(format!("パスにアクセスできません: {}", smb_path));
    }

    let tmp_name = format!("netspeed_test_{}.tmp", std::process::id());
    base.push(&tmp_name);

    let mut file = std::fs::File::create(&base)
        .map_err(|e| format!("ファイル作成失敗: {}", e))?;

    let chunk = vec![0xABu8; CHUNK_SIZE];
    let mut total_bytes = 0u64;
    let t0 = Instant::now();
    let duration = std::time::Duration::from_secs(TEST_DURATION_SECS);
    let mut last_report = Instant::now();

    while t0.elapsed() < duration {
        if stop.load(Ordering::Relaxed) { break; }
        match file.write_all(&chunk) {
            Ok(_) => { total_bytes += CHUNK_SIZE as u64; }
            Err(e) => {
                // クリーンアップ
                drop(file);
                let _ = std::fs::remove_file(&base);
                return Err(format!("書き込みエラー: {}", e));
            }
        }
        // 0.5秒ごとに途中速度を通知
        if last_report.elapsed().as_millis() >= 500 {
            let elapsed = t0.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                let mbps = (total_bytes as f64 * 8.0) / elapsed / 1_000_000.0;
                on_progress(mbps);
            }
            last_report = Instant::now();
        }
    }

    // フラッシュして実際のネットワーク転送を完了させる
    let _ = file.flush();
    drop(file);

    // 一時ファイル削除
    let _ = std::fs::remove_file(&base);

    let elapsed = t0.elapsed().as_secs_f64();
    if elapsed < 0.1 || total_bytes == 0 {
        return Err("測定データが不十分です".into());
    }

    let mbps = (total_bytes as f64 * 8.0) / elapsed / 1_000_000.0;
    Ok(IperfResult {
        mbps,
        bytes: total_bytes,
        duration: elapsed,
        direction: "Write (SMB)".into(),
        path: smb_path.to_string(),
    })
}

/// SMB共有からの読み込み速度測定
/// 先に measure_smb_write を呼んでファイルを残した状態で使う、または別ファイルを指定
pub fn measure_smb_read(
    smb_file_path: &str,
    stop: Arc<AtomicBool>,
    on_progress: impl Fn(f64),
) -> Result<IperfResult, String> {
    use std::io::Read;
    let path = PathBuf::from(smb_file_path);
    if !path.exists() {
        return Err(format!("ファイルが見つかりません: {}", smb_file_path));
    }

    let mut file = std::fs::File::open(&path)
        .map_err(|e| format!("ファイルオープン失敗: {}", e))?;

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut total_bytes = 0u64;
    let t0 = Instant::now();
    let duration = std::time::Duration::from_secs(TEST_DURATION_SECS);
    let mut last_report = Instant::now();

    loop {
        if stop.load(Ordering::Relaxed) || t0.elapsed() >= duration { break; }
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => { total_bytes += n as u64; }
            Err(e) => return Err(format!("読み込みエラー: {}", e)),
        }
        if last_report.elapsed().as_millis() >= 500 {
            let elapsed = t0.elapsed().as_secs_f64();
            if elapsed > 0.0 {
                on_progress((total_bytes as f64 * 8.0) / elapsed / 1_000_000.0);
            }
            last_report = Instant::now();
        }
    }

    let elapsed = t0.elapsed().as_secs_f64();
    if elapsed < 0.1 || total_bytes == 0 {
        return Err("読み込みデータ不十分".into());
    }

    Ok(IperfResult {
        mbps: (total_bytes as f64 * 8.0) / elapsed / 1_000_000.0,
        bytes: total_bytes,
        duration: elapsed,
        direction: "Read (SMB)".into(),
        path: smb_file_path.to_string(),
    })
}
