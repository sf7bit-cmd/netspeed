

#[derive(Clone, Debug, Default)]
pub struct WifiInfo {
    pub ssid:       Option<String>,
    pub signal_pct: Option<u32>,
    pub bssid:      Option<String>,
    pub radio_type: Option<String>,
}

pub fn get_wifi_info() -> WifiInfo {
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("netsh")
            .args(["wlan", "show", "interfaces"])
            .output();
        let Ok(output) = output else { return WifiInfo::default(); };
        let text = String::from_utf8_lossy(&output.stdout);
        let mut info = WifiInfo::default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("SSID") && !line.starts_with("BSSID") {
                info.ssid = line.splitn(2, ':').nth(1).map(|s| s.trim().to_string());
            } else if line.starts_with("BSSID") {
                info.bssid = line.splitn(2, ':').nth(1).map(|s| s.trim().to_string());
            } else if line.starts_with("Signal") {
                let val = line.splitn(2, ':').nth(1)
                    .and_then(|s| s.trim().trim_end_matches('%').parse::<u32>().ok());
                info.signal_pct = val;
            } else if line.starts_with("Radio type") {
                info.radio_type = line.splitn(2, ':').nth(1).map(|s| s.trim().to_string());
            }
        }
        info
    }
    #[cfg(not(target_os = "windows"))]
    { WifiInfo::default() }
}
