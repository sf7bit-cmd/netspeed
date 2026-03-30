// src/ui.rs — egui メイン UI v0.2

use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use egui::{Color32, FontId, RichText, Stroke, Vec2};
use egui_plot::{Line, Plot, PlotPoints};

use crate::measure;
use crate::scan::{self, HostResult};
use crate::history::{self, HistoryEntry};
use crate::wifi::{self, WifiInfo};
use crate::iperf;

const BG:     Color32 = Color32::from_rgb(8,  12, 16);
const PANEL:  Color32 = Color32::from_rgb(13, 21, 32);
const ACCENT: Color32 = Color32::from_rgb(0, 212, 255);
const ACCENT2:Color32 = Color32::from_rgb(255, 107, 53);
const ACCENT3:Color32 = Color32::from_rgb(57, 255, 20);
const MUTED:  Color32 = Color32::from_rgb(58, 84, 112);
const DANGER: Color32 = Color32::from_rgb(255, 45, 85);
const WARN:   Color32 = Color32::from_rgb(255, 190, 0);

#[derive(Default, Clone, PartialEq)]
enum TestState { #[default] Idle, Running, Done }

#[derive(Default, Clone)]
struct SpeedResult {
    dl_mbps:  Option<f64>,
    ul_mbps:  Option<f64>,
    ping_ms:  Option<f64>,
    jitter:   Option<f64>,
    min_ping: Option<f64>,
    max_ping: Option<f64>,
}

#[derive(Default, Clone)]
struct ScanState {
    running:  bool,
    progress: f32,
    hosts:    Vec<HostResult>,
}

#[derive(Default, Clone, PartialEq)]
enum IperfMode { #[default] Idle, Server, Client, Done }

#[derive(Default, Clone)]
struct IperfState {
    mode:        IperfMode,
    target_ip:   String,
    result:      Option<iperf::IperfResult>,
    history:     Vec<(String, f64)>,  // (方向, Mbps)
}

pub struct App {
    test_state:   TestState,
    result:       Arc<Mutex<SpeedResult>>,
    dl_history:   Arc<Mutex<Vec<f64>>>,
    ul_history:   Arc<Mutex<Vec<f64>>>,
    ping_history: Arc<Mutex<Vec<f64>>>,
    log_lines:    Arc<Mutex<Vec<(String, Color32)>>>,
    stop_flag:    Arc<AtomicBool>,

    subnet_input: String,
    scan_state:   Arc<Mutex<ScanState>>,
    scan_stop:    Arc<AtomicBool>,

    speed_history: Vec<HistoryEntry>,

    wifi_info:    Arc<Mutex<WifiInfo>>,
    wifi_timer:   f64,

    iperf_state:  Arc<Mutex<IperfState>>,
    iperf_stop:   Arc<AtomicBool>,

    active_tab:   u8,  // 0=Speed, 1=LAN, 2=iperf, 3=History
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = PANEL;
        visuals.override_text_color = Some(Color32::from_rgb(200, 223, 240));
        cc.egui_ctx.set_visuals(visuals);

        let mut app = Self {
            test_state:    TestState::Idle,
            result:        Arc::new(Mutex::new(SpeedResult::default())),
            dl_history:    Arc::new(Mutex::new(Vec::new())),
            ul_history:    Arc::new(Mutex::new(Vec::new())),
            ping_history:  Arc::new(Mutex::new(Vec::new())),
            log_lines:     Arc::new(Mutex::new(vec![
                ("NetSpeed Analyzer v0.2 起動".into(), ACCENT3),
                ("speed.cloudflare.com を使用".into(), MUTED),
            ])),
            stop_flag:     Arc::new(AtomicBool::new(false)),
            subnet_input:  "192.168.1".into(),
            scan_state:    Arc::new(Mutex::new(ScanState::default())),
            scan_stop:     Arc::new(AtomicBool::new(false)),
            speed_history: Vec::new(),
            wifi_info:     Arc::new(Mutex::new(WifiInfo::default())),
            wifi_timer:    0.0,
            iperf_state:   Arc::new(Mutex::new(IperfState::default())),
            iperf_stop:    Arc::new(AtomicBool::new(false)),
            active_tab:    0,
        };
        app.refresh_wifi();
        app
    }

    fn add_log(&self, msg: impl Into<String>, color: Color32) {
        let mut log = self.log_lines.lock().unwrap();
        log.push((format!("[{}] {}", now_str(), msg.into()), color));
        if log.len() > 300 { log.drain(0..50); }
    }

    fn refresh_wifi(&self) {
        let wifi = Arc::clone(&self.wifi_info);
        thread::spawn(move || {
            let info = wifi::get_wifi_info();
            *wifi.lock().unwrap() = info;
        });
    }

    fn start_test(&mut self, ctx: &egui::Context) {
        self.test_state = TestState::Running;
        self.stop_flag.store(false, Ordering::Relaxed);
        self.dl_history.lock().unwrap().clear();
        self.ul_history.lock().unwrap().clear();
        self.ping_history.lock().unwrap().clear();
        *self.result.lock().unwrap() = SpeedResult::default();

        let result    = Arc::clone(&self.result);
        let dl_hist   = Arc::clone(&self.dl_history);
        let ul_hist   = Arc::clone(&self.ul_history);
        let ping_hist = Arc::clone(&self.ping_history);
        let log       = Arc::clone(&self.log_lines);
        let stop      = Arc::clone(&self.stop_flag);
        let ctx       = ctx.clone();

        fn plog(log: &Arc<Mutex<Vec<(String,Color32)>>>, msg: &str, c: Color32) {
            log.lock().unwrap().push((format!("[{}] {}", now_str(), msg), c));
        }

        thread::spawn(move || {
            plog(&log, "Ping測定中...", MUTED); ctx.request_repaint();
            if let Some(p) = measure::measure_ping(5) {
                let mut r = result.lock().unwrap();
                r.ping_ms = Some(p.avg); r.jitter = Some(p.jitter);
                r.min_ping = Some(p.min); r.max_ping = Some(p.max);
                ping_hist.lock().unwrap().push(p.avg);
                plog(&log, &format!("Ping: {:.1}ms  jitter: {:.1}ms", p.avg, p.jitter), ACCENT3);
            } else { plog(&log, "Ping失敗", DANGER); }
            ctx.request_repaint();
            if stop.load(Ordering::Relaxed) { return; }

            plog(&log, "DL測定中...", MUTED); ctx.request_repaint();
            let dl2 = Arc::clone(&dl_hist);
            let log2 = Arc::clone(&log);
            let ctx2 = ctx.clone();
            let final_dl = measure::measure_download(3, move |mbps| {
                dl2.lock().unwrap().push(mbps);
                plog(&log2, &format!("  DL: {:.1} Mbps", mbps), MUTED);
                ctx2.request_repaint();
            });
            if let Some(dl) = final_dl {
                result.lock().unwrap().dl_mbps = Some(dl);
                plog(&log, &format!("DL完了: {:.1} Mbps", dl), ACCENT);
            } else { plog(&log, "DL失敗", DANGER); }
            ctx.request_repaint();
            if stop.load(Ordering::Relaxed) { return; }

            plog(&log, "UL測定中...", MUTED); ctx.request_repaint();
            let ul2 = Arc::clone(&ul_hist);
            let log3 = Arc::clone(&log);
            let ctx3 = ctx.clone();
            let final_ul = measure::measure_upload(2, move |mbps| {
                ul2.lock().unwrap().push(mbps);
                plog(&log3, &format!("  UL: {:.1} Mbps", mbps), MUTED);
                ctx3.request_repaint();
            });
            if let Some(ul) = final_ul {
                result.lock().unwrap().ul_mbps = Some(ul);
                plog(&log, &format!("UL完了: {:.1} Mbps", ul), ACCENT2);
            } else { plog(&log, "UL失敗", DANGER); }
            plog(&log, "テスト完了 ✓", ACCENT3);
            ctx.request_repaint();
        });
    }

    fn start_scan(&mut self, ctx: &egui::Context) {
        let parts: Vec<&str> = self.subnet_input.split('.').collect();
        if parts.len() != 3 {
            self.add_log("サブネット形式エラー (例: 192.168.1)", DANGER);
            return;
        }
        let subnet = [
            parts[0].parse::<u8>().unwrap_or(192),
            parts[1].parse::<u8>().unwrap_or(168),
            parts[2].parse::<u8>().unwrap_or(1),
        ];
        { let mut s = self.scan_state.lock().unwrap(); s.running = true; s.progress = 0.0; s.hosts.clear(); }
        self.scan_stop.store(false, Ordering::Relaxed);
        self.add_log(&format!("スキャン: {}.1–254", self.subnet_input), ACCENT);

        let ss    = Arc::clone(&self.scan_state);
        let log   = Arc::clone(&self.log_lines);
        let stop  = Arc::clone(&self.scan_stop);
        let ctx   = ctx.clone();

        thread::spawn(move || {
            let ss2 = Arc::clone(&ss);
            let ss3 = Arc::clone(&ss);
            let log2 = Arc::clone(&log);
            let ctx2 = ctx.clone();
            let ctx3 = ctx.clone();
            scan::scan_subnet(subnet, 500,
                move |host| {
                    let name = host.hostname.clone().unwrap_or_else(|| "—".into());
                    log2.lock().unwrap().push((
                        format!("[{}] {} ({})  {:.0}ms", now_str(), host.ip, name, host.latency_ms),
                        ACCENT3));
                    ss2.lock().unwrap().hosts.push(host);
                    ctx2.request_repaint();
                },
                move |p| { ss3.lock().unwrap().progress = p; ctx3.request_repaint(); },
                stop,
            );
            let found = ss.lock().unwrap().hosts.len();
            ss.lock().unwrap().running = false;
            log.lock().unwrap().push((
                format!("[{}] スキャン完了: {}台", now_str(), found),
                if found > 0 { ACCENT3 } else { MUTED }));
            ctx.request_repaint();
        });
    }

    fn start_iperf_server(&mut self, ctx: &egui::Context) {
        self.iperf_stop.store(false, Ordering::Relaxed);
        self.iperf_state.lock().unwrap().mode = IperfMode::Server;
        self.add_log(&format!("iperf サーバー起動 port:{}", iperf::DEFAULT_PORT), ACCENT);

        let state = Arc::clone(&self.iperf_state);
        let log   = Arc::clone(&self.log_lines);
        let stop  = Arc::clone(&self.iperf_stop);
        let ctx   = ctx.clone();

        thread::spawn(move || {
            match iperf::run_server(stop) {
                Some(r) => {
                    log.lock().unwrap().push((
                        format!("[{}] iperf RX: {:.1} Mbps ({:.1}s)", now_str(), r.mbps, r.duration),
                        ACCENT3));
                    let mut s = state.lock().unwrap();
                    s.history.push(("RX".into(), r.mbps));
                    s.result = Some(r);
                    s.mode = IperfMode::Done;
                }
                None => {
                    log.lock().unwrap().push((format!("[{}] iperf サーバー終了", now_str()), MUTED));
                    state.lock().unwrap().mode = IperfMode::Idle;
                }
            }
            ctx.request_repaint();
        });
    }

    fn start_iperf_client(&mut self, ctx: &egui::Context) {
        let target = self.iperf_state.lock().unwrap().target_ip.clone();
        if target.is_empty() { self.add_log("対象IPを入力してください", DANGER); return; }
        self.iperf_stop.store(false, Ordering::Relaxed);
        self.iperf_state.lock().unwrap().mode = IperfMode::Client;
        self.add_log(&format!("iperf クライアント → {}", target), ACCENT);

        let state = Arc::clone(&self.iperf_state);
        let log   = Arc::clone(&self.log_lines);
        let stop  = Arc::clone(&self.iperf_stop);
        let ctx   = ctx.clone();

        thread::spawn(move || {
            match iperf::run_client(&target, stop) {
                Some(r) => {
                    log.lock().unwrap().push((
                        format!("[{}] iperf TX: {:.1} Mbps ({:.1}s)", now_str(), r.mbps, r.duration),
                        ACCENT2));
                    let mut s = state.lock().unwrap();
                    s.history.push(("TX".into(), r.mbps));
                    s.result = Some(r);
                    s.mode = IperfMode::Done;
                }
                None => {
                    log.lock().unwrap().push((format!("[{}] iperf 接続失敗", now_str()), DANGER));
                    state.lock().unwrap().mode = IperfMode::Idle;
                }
            }
            ctx.request_repaint();
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.test_state == TestState::Running {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        // Running → Done 遷移
        if self.test_state == TestState::Running {
            let r = self.result.lock().unwrap();
            if r.dl_mbps.is_some() && r.ul_mbps.is_some() {
                let entry = history::new_entry(r.dl_mbps, r.ul_mbps, r.ping_ms, r.jitter);
                drop(r);
                self.speed_history.push(entry);
                self.test_state = TestState::Done;
            }
        }

        // Wi-Fi 定期更新（30秒ごと）
        self.wifi_timer += ctx.input(|i| i.unstable_dt) as f64;
        if self.wifi_timer > 30.0 { self.wifi_timer = 0.0; self.refresh_wifi(); }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(14.0)))
            .show(ctx, |ui| {
                self.draw_header(ui);
                ui.add_space(8.0);
                self.draw_tabs(ui);
                ui.add_space(10.0);
                match self.active_tab {
                    0 => self.draw_speed_tab(ui, ctx),
                    1 => self.draw_lan_tab(ui, ctx),
                    2 => self.draw_iperf_tab(ui, ctx),
                    3 => self.draw_history_tab(ui),
                    _ => {}
                }
                ui.add_space(8.0);
                self.draw_log(ui);
            });
    }
}

impl App {
    fn draw_header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::new(40.0, 40.0), egui::Sense::hover());
            ui.painter().rect_stroke(r, 6.0, Stroke::new(2.0, ACCENT));
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER,
                "NS", FontId::monospace(16.0), ACCENT);
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("NETSPEED ANALYZER")
                    .font(FontId::proportional(18.0)).color(Color32::WHITE).strong());
                ui.label(RichText::new("v0.2  —  LAN NETWORK TOOL")
                    .font(FontId::monospace(9.0)).color(MUTED));
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Wi-Fi 情報
                let wifi = self.wifi_info.lock().unwrap().clone();
                if let Some(pct) = wifi.signal_pct {
                    let color = if pct >= 70 { ACCENT3 } else if pct >= 40 { WARN } else { DANGER };
                    let bars = if pct >= 75 { "▂▄▆█" } else if pct >= 50 { "▂▄▆_" }
                               else if pct >= 25 { "▂▄__" } else { "▂___" };
                    ui.label(RichText::new(format!("{} {}%", bars, pct))
                        .font(FontId::monospace(12.0)).color(color));
                    if let Some(ssid) = &wifi.ssid {
                        ui.label(RichText::new(ssid).font(FontId::monospace(10.0)).color(MUTED));
                    }
                    ui.add_space(8.0);
                }
                let (st, sc) = match self.test_state {
                    TestState::Idle    => ("● READY",   ACCENT3),
                    TestState::Running => ("● TESTING", ACCENT),
                    TestState::Done    => ("● DONE",    ACCENT3),
                };
                ui.label(RichText::new(st).font(FontId::monospace(11.0)).color(sc));
            });
        });
        ui.add_space(6.0);
        ui.separator();
    }

    fn draw_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (i, label) in ["SPEED TEST", "LAN SCAN", "LAN THROUGHPUT", "HISTORY"].iter().enumerate() {
                let active = self.active_tab == i as u8;
                let color  = if active { ACCENT } else { MUTED };
                let fill   = if active { Color32::from_rgba_unmultiplied(0,212,255,18) } else { Color32::TRANSPARENT };
                if ui.add(egui::Button::new(
                    RichText::new(*label).font(FontId::monospace(11.0)).color(color))
                    .fill(fill)
                    .stroke(Stroke::new(if active { 1.5 } else { 0.5 }, color))
                    .min_size(Vec2::new(120.0, 28.0))
                ).clicked() {
                    self.active_tab = i as u8;
                }
                ui.add_space(4.0);
            }
        });
    }

    // ── Speed Test タブ ─────────────────────────────
    fn draw_speed_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let r = self.result.lock().unwrap().clone();
        let testing = self.test_state == TestState::Running;

        // Gauges
        ui.columns(3, |cols| {
            gauge_card(&mut cols[0], "↓  DOWNLOAD", r.dl_mbps, "Mbps", ACCENT,  1000.0, testing);
            gauge_card(&mut cols[1], "↑  UPLOAD",   r.ul_mbps, "Mbps", ACCENT2, 1000.0, testing);
            gauge_card(&mut cols[2], "◎  LATENCY",  r.ping_ms, "ms",   ACCENT3,  200.0, testing);
        });
        ui.add_space(8.0);

        // Graph + Controls
        ui.columns(2, |cols| {
            // グラフ
            {
                let ui = &mut cols[0];
                let dl   = self.dl_history.lock().unwrap().clone();
                let ul   = self.ul_history.lock().unwrap().clone();
                let ping = self.ping_history.lock().unwrap().clone();

                egui::Frame::none().fill(PANEL).rounding(8.0)
                    .inner_margin(egui::Margin::same(12.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("REALTIME GRAPH").font(FontId::monospace(9.0)).color(MUTED));
                            ui.add_space(6.0);
                            legend_dot(ui, ACCENT,  "DL(Mbps)");
                            legend_dot(ui, ACCENT2, "UL(Mbps)");
                            legend_dot(ui, ACCENT3, "Ping(ms)");
                        });
                        ui.add_space(4.0);
                        Plot::new("speed_graph")
                            .height(120.0)
                            .show_axes([false, true])
                            .show_grid(true)
                            .allow_zoom(false)
                            .allow_drag(false)
                            .show(ui, |p| {
                                if !dl.is_empty() {
                                    p.line(Line::new(pts(&dl)).color(ACCENT).width(2.0).name("DL Mbps"));
                                }
                                if !ul.is_empty() {
                                    p.line(Line::new(pts(&ul)).color(ACCENT2).width(2.0).name("UL Mbps"));
                                }
                                if !ping.is_empty() {
                                    p.line(Line::new(pts(&ping)).color(ACCENT3).width(1.5).name("Ping ms"));
                                }
                            });
                    });
            }

            // コントロール
            {
                let ui = &mut cols[1];
                egui::Frame::none().fill(PANEL).rounding(8.0)
                    .inner_margin(egui::Margin::same(12.0))
                    .show(ui, |ui| {
                        section_title(ui, "CONTROLS");
                        ui.add_space(6.0);
                        let w = ui.available_width();

                        if ui.add_enabled(!testing, egui::Button::new(
                            RichText::new("▶  START TEST").font(FontId::monospace(13.0)).color(ACCENT))
                            .min_size(Vec2::new(w, 38.0))
                            .stroke(Stroke::new(1.5, ACCENT))
                            .fill(Color32::from_rgba_unmultiplied(0,212,255,12))
                        ).clicked() { self.start_test(ctx); }

                        ui.add_space(4.0);
                        if ui.add_enabled(testing, egui::Button::new(
                            RichText::new("■  STOP").font(FontId::monospace(12.0)).color(DANGER))
                            .min_size(Vec2::new(w, 30.0))
                            .stroke(Stroke::new(1.0, if testing { DANGER } else { MUTED }))
                            .fill(Color32::TRANSPARENT)
                        ).clicked() {
                            self.stop_flag.store(true, Ordering::Relaxed);
                            self.test_state = TestState::Idle;
                        }

                        ui.add_space(4.0);
                        if ui.add_enabled(!testing, egui::Button::new(
                            RichText::new("RESET").font(FontId::monospace(10.0)).color(MUTED))
                            .min_size(Vec2::new(w, 24.0))
                            .stroke(Stroke::new(1.0, MUTED))
                            .fill(Color32::TRANSPARENT)
                        ).clicked() {
                            *self.result.lock().unwrap() = SpeedResult::default();
                            self.dl_history.lock().unwrap().clear();
                            self.ul_history.lock().unwrap().clear();
                            self.ping_history.lock().unwrap().clear();
                            self.test_state = TestState::Idle;
                        }

                        ui.add_space(10.0);
                        section_title(ui, "DETAILS");
                        ui.add_space(4.0);
                        let r = self.result.lock().unwrap().clone();
                        info_row(ui, "MIN PING",  r.min_ping.map(|v| format!("{:.1} ms", v)));
                        info_row(ui, "MAX PING",  r.max_ping.map(|v| format!("{:.1} ms", v)));
                        info_row(ui, "JITTER",    r.jitter.map(|v| format!("{:.1} ms", v)));
                        info_row(ui, "SERVER",    Some("speed.cloudflare.com".into()));

                        ui.add_space(10.0);
                        section_title(ui, "SAVE RESULT");
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.add(egui::Button::new(
                                RichText::new("CSV").font(FontId::monospace(11.0)).color(ACCENT))
                                .stroke(Stroke::new(1.0, ACCENT))
                                .fill(Color32::TRANSPARENT)
                            ).clicked() {
                                match history::save_csv(&self.speed_history) {
                                    Ok(p) => self.add_log(&format!("CSV保存: {}", p.display()), ACCENT3),
                                    Err(e) => self.add_log(&format!("CSV保存失敗: {}", e), DANGER),
                                }
                            }
                            if ui.add(egui::Button::new(
                                RichText::new("JSON").font(FontId::monospace(11.0)).color(ACCENT2))
                                .stroke(Stroke::new(1.0, ACCENT2))
                                .fill(Color32::TRANSPARENT)
                            ).clicked() {
                                match history::save_json(&self.speed_history) {
                                    Ok(p) => self.add_log(&format!("JSON保存: {}", p.display()), ACCENT3),
                                    Err(e) => self.add_log(&format!("JSON保存失敗: {}", e), DANGER),
                                }
                            }
                        });
                    });
            }
        });
    }

    // ── LAN スキャンタブ ─────────────────────────────
    fn draw_lan_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::Frame::none().fill(PANEL).rounding(8.0)
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                section_title(ui, "LAN HOST SCAN");
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Subnet:").font(FontId::monospace(11.0)).color(MUTED));
                    ui.add(egui::TextEdit::singleline(&mut self.subnet_input)
                        .font(FontId::monospace(12.0)).desired_width(120.0));
                    ui.label(RichText::new(".1–254").font(FontId::monospace(11.0)).color(MUTED));
                    ui.add_space(10.0);

                    let running = self.scan_state.lock().unwrap().running;
                    if ui.add_enabled(!running, egui::Button::new(
                        RichText::new("  SCAN  ").font(FontId::monospace(12.0)).color(ACCENT))
                        .stroke(Stroke::new(1.0, ACCENT)).fill(Color32::TRANSPARENT)
                    ).clicked() { self.start_scan(ctx); }

                    if running {
                        if ui.add(egui::Button::new(
                            RichText::new("STOP").font(FontId::monospace(11.0)).color(DANGER))
                            .stroke(Stroke::new(1.0, DANGER)).fill(Color32::TRANSPARENT)
                        ).clicked() { self.scan_stop.store(true, Ordering::Relaxed); }
                    }
                });

                // プログレス
                {
                    let s = self.scan_state.lock().unwrap();
                    if s.running || s.progress > 0.0 {
                        ui.add_space(6.0);
                        ui.add(egui::ProgressBar::new(s.progress)
                            .desired_width(ui.available_width()));
                    }
                }

                ui.add_space(10.0);

                // ヘッダー行
                ui.horizontal(|ui| {
                    ui.label(RichText::new("IP ADDRESS").font(FontId::monospace(10.0)).color(MUTED));
                    ui.add_space(80.0);
                    ui.label(RichText::new("HOSTNAME").font(FontId::monospace(10.0)).color(MUTED));
                    ui.add_space(80.0);
                    ui.label(RichText::new("LATENCY").font(FontId::monospace(10.0)).color(MUTED));
                    ui.add_space(30.0);
                    ui.label(RichText::new("OPEN PORTS").font(FontId::monospace(10.0)).color(MUTED));
                });
                ui.separator();

                let hosts = self.scan_state.lock().unwrap().hosts.clone();

                egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                    if hosts.is_empty() {
                        ui.label(RichText::new("スキャン待機中...").font(FontId::monospace(11.0)).color(MUTED));
                    }
                    for host in &hosts {
                        ui.horizontal(|ui| {
                            // ステータスドット
                            let (dr, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                            ui.painter().circle_filled(dr.center(), 4.0, ACCENT3);

                            // IP（クリックでWebGUI）
                            let ip_str = host.ip.to_string();
                            let web_url = host.web_port.map(|p| {
                                let scheme = if p == 443 || p == 8443 { "https" } else { "http" };
                                format!("{}://{}:{}", scheme, ip_str, p)
                            });

                            if let Some(ref url) = web_url {
                                if ui.add(egui::Button::new(
                                    RichText::new(&ip_str).font(FontId::monospace(12.0)).color(ACCENT))
                                    .stroke(Stroke::new(0.5, ACCENT))
                                    .fill(Color32::TRANSPARENT)
                                ).on_hover_text(format!("クリックで {} を開く", url))
                                .clicked() {
                                    let _ = open::that(url);
                                }
                            } else {
                                ui.label(RichText::new(&ip_str)
                                    .font(FontId::monospace(12.0))
                                    .color(Color32::from_rgb(200, 223, 240)));
                            }

                            ui.add_space(8.0);

                            // ホスト名
                            let name = host.hostname.clone().unwrap_or_else(|| "—".into());
                            ui.label(RichText::new(&name).font(FontId::monospace(11.0)).color(MUTED));

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                // オープンポート
                                let ports: Vec<String> = host.open_ports.iter()
                                    .map(|p| p.to_string()).collect();
                                ui.label(RichText::new(ports.join(" "))
                                    .font(FontId::monospace(10.0)).color(MUTED));
                                ui.add_space(20.0);
                                // レイテンシ
                                let ms = host.latency_ms;
                                let color = if ms < 5.0 { ACCENT3 } else if ms < 30.0 { WARN } else { DANGER };
                                ui.label(RichText::new(format!("{:.0} ms", ms))
                                    .font(FontId::monospace(11.0)).color(color));
                            });
                        });
                    }
                });
            });
    }

    // ── LAN スループット（iperf）タブ ──────────────────
    fn draw_iperf_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::Frame::none().fill(PANEL).rounding(8.0)
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                section_title(ui, "LAN THROUGHPUT  (内蔵 iperf3 相当)");
                ui.add_space(6.0);
                ui.label(RichText::new(
                    "使い方: PC-A でサーバー起動 → PC-B でクライアント実行（両方に netspeed.exe が必要）")
                    .font(FontId::monospace(10.0)).color(MUTED));
                ui.add_space(10.0);

                let mode = self.iperf_state.lock().unwrap().mode.clone();
                let busy = mode == IperfMode::Server || mode == IperfMode::Client;

                ui.columns(2, |cols| {
                    // サーバー側
                    {
                        let ui = &mut cols[0];
                        egui::Frame::none()
                            .stroke(Stroke::new(1.0, MUTED)).rounding(6.0)
                            .inner_margin(egui::Margin::same(10.0))
                            .show(ui, |ui| {
                                ui.label(RichText::new("SERVER MODE (このPCで受信)")
                                    .font(FontId::monospace(11.0)).color(ACCENT));
                                ui.add_space(6.0);
                                ui.label(RichText::new(format!("待ち受けポート: {}", iperf::DEFAULT_PORT))
                                    .font(FontId::monospace(10.0)).color(MUTED));
                                ui.add_space(8.0);
                                if ui.add_enabled(!busy, egui::Button::new(
                                    RichText::new("▶ サーバー起動").font(FontId::monospace(12.0)).color(ACCENT))
                                    .stroke(Stroke::new(1.0, ACCENT))
                                    .fill(Color32::from_rgba_unmultiplied(0,212,255,10))
                                    .min_size(Vec2::new(ui.available_width(), 34.0))
                                ).clicked() { self.start_iperf_server(ctx); }

                                if mode == IperfMode::Server {
                                    ui.add_space(6.0);
                                    ui.label(RichText::new("⏳ クライアントからの接続待ち...")
                                        .font(FontId::monospace(10.0)).color(WARN));
                                }
                            });
                    }
                    // クライアント側
                    {
                        let ui = &mut cols[1];
                        egui::Frame::none()
                            .stroke(Stroke::new(1.0, MUTED)).rounding(6.0)
                            .inner_margin(egui::Margin::same(10.0))
                            .show(ui, |ui| {
                                ui.label(RichText::new("CLIENT MODE (対象PCへ送信)")
                                    .font(FontId::monospace(11.0)).color(ACCENT2));
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("対象IP:").font(FontId::monospace(11.0)).color(MUTED));
                                    let mut target = self.iperf_state.lock().unwrap().target_ip.clone();
                                    if ui.add(egui::TextEdit::singleline(&mut target)
                                        .font(FontId::monospace(12.0))
                                        .desired_width(130.0)
                                    ).changed() {
                                        self.iperf_state.lock().unwrap().target_ip = target;
                                    }
                                });
                                ui.add_space(8.0);
                                if ui.add_enabled(!busy, egui::Button::new(
                                    RichText::new("▶ 送信開始").font(FontId::monospace(12.0)).color(ACCENT2))
                                    .stroke(Stroke::new(1.0, ACCENT2))
                                    .fill(Color32::from_rgba_unmultiplied(255,107,53,10))
                                    .min_size(Vec2::new(ui.available_width(), 34.0))
                                ).clicked() { self.start_iperf_client(ctx); }

                                if mode == IperfMode::Client {
                                    ui.add_space(6.0);
                                    ui.label(RichText::new(format!("⏳ 送信中... ({}s)", iperf::TEST_DURATION_SECS))
                                        .font(FontId::monospace(10.0)).color(WARN));
                                }
                            });
                    }
                });

                ui.add_space(12.0);

                // 結果
                let state = self.iperf_state.lock().unwrap().clone();
                if let Some(ref r) = state.result {
                    egui::Frame::none()
                        .fill(Color32::from_rgba_unmultiplied(0,212,255,8))
                        .rounding(6.0).inner_margin(egui::Margin::same(12.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new(format!("結果: {:.1} Mbps  ({})",
                                r.mbps, r.direction))
                                .font(FontId::monospace(22.0))
                                .color(if r.direction.starts_with("TX") { ACCENT2 } else { ACCENT3 })
                                .strong());
                            ui.label(RichText::new(format!(
                                "転送量: {:.1} MB  /  時間: {:.1}s",
                                r.bytes as f64 / 1_048_576.0, r.duration))
                                .font(FontId::monospace(11.0)).color(MUTED));
                        });
                }

                // 履歴グラフ
                if !state.history.is_empty() {
                    ui.add_space(10.0);
                    section_title(ui, "THROUGHPUT HISTORY");
                    Plot::new("iperf_hist")
                        .height(80.0).show_axes([false, true])
                        .allow_zoom(false).allow_drag(false)
                        .show(ui, |p| {
                            let tx_pts: PlotPoints = state.history.iter().enumerate()
                                .filter(|(_, (d, _))| d == "TX")
                                .map(|(i, (_, v))| [i as f64, *v]).collect();
                            let rx_pts: PlotPoints = state.history.iter().enumerate()
                                .filter(|(_, (d, _))| d == "RX")
                                .map(|(i, (_, v))| [i as f64, *v]).collect();
                            p.line(Line::new(tx_pts).color(ACCENT2).width(2.0).name("TX Mbps"));
                            p.line(Line::new(rx_pts).color(ACCENT3).width(2.0).name("RX Mbps"));
                        });
                }

                ui.add_space(6.0);
                if ui.add_enabled(busy, egui::Button::new(
                    RichText::new("■ STOP").font(FontId::monospace(11.0)).color(DANGER))
                    .stroke(Stroke::new(1.0, DANGER)).fill(Color32::TRANSPARENT)
                ).clicked() {
                    self.iperf_stop.store(true, Ordering::Relaxed);
                    self.iperf_state.lock().unwrap().mode = IperfMode::Idle;
                }
            });
    }

    // ── 履歴タブ ────────────────────────────────────
    fn draw_history_tab(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none().fill(PANEL).rounding(8.0)
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                section_title(ui, "MEASUREMENT HISTORY");
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(
                        RichText::new("💾 CSV保存").font(FontId::monospace(11.0)).color(ACCENT))
                        .stroke(Stroke::new(1.0, ACCENT)).fill(Color32::TRANSPARENT)
                    ).clicked() {
                        match history::save_csv(&self.speed_history) {
                            Ok(p)  => self.add_log(&format!("CSV保存: {}", p.display()), ACCENT3),
                            Err(e) => self.add_log(&format!("失敗: {}", e), DANGER),
                        }
                    }
                    if ui.add(egui::Button::new(
                        RichText::new("💾 JSON保存").font(FontId::monospace(11.0)).color(ACCENT2))
                        .stroke(Stroke::new(1.0, ACCENT2)).fill(Color32::TRANSPARENT)
                    ).clicked() {
                        match history::save_json(&self.speed_history) {
                            Ok(p)  => self.add_log(&format!("JSON保存: {}", p.display()), ACCENT3),
                            Err(e) => self.add_log(&format!("失敗: {}", e), DANGER),
                        }
                    }
                    if ui.add(egui::Button::new(
                        RichText::new("🗑 クリア").font(FontId::monospace(11.0)).color(MUTED))
                        .stroke(Stroke::new(1.0, MUTED)).fill(Color32::TRANSPARENT)
                    ).clicked() { self.speed_history.clear(); }
                });

                ui.add_space(8.0);

                if self.speed_history.is_empty() {
                    ui.label(RichText::new("履歴なし。Speed Testを実行すると記録されます。")
                        .font(FontId::monospace(11.0)).color(MUTED));
                    return;
                }

                // グラフ
                let dl_pts: PlotPoints = self.speed_history.iter().enumerate()
                    .filter_map(|(i, e)| e.dl_mbps.map(|v| [i as f64, v])).collect();
                let ul_pts: PlotPoints = self.speed_history.iter().enumerate()
                    .filter_map(|(i, e)| e.ul_mbps.map(|v| [i as f64, v])).collect();
                let ping_pts: PlotPoints = self.speed_history.iter().enumerate()
                    .filter_map(|(i, e)| e.ping_ms.map(|v| [i as f64, v])).collect();

                Plot::new("hist_graph")
                    .height(110.0).show_axes([false, true])
                    .allow_zoom(false).allow_drag(false)
                    .show(ui, |p| {
                        p.line(Line::new(dl_pts).color(ACCENT).width(2.0).name("DL Mbps"));
                        p.line(Line::new(ul_pts).color(ACCENT2).width(2.0).name("UL Mbps"));
                        p.line(Line::new(ping_pts).color(ACCENT3).width(1.5).name("Ping ms"));
                    });

                ui.add_space(8.0);

                // テーブル
                ui.horizontal(|ui| {
                    for h in ["#", "時刻", "DL(Mbps)", "UL(Mbps)", "Ping(ms)", "Jitter(ms)"] {
                        ui.label(RichText::new(h).font(FontId::monospace(10.0)).color(MUTED));
                        ui.add_space(20.0);
                    }
                });
                ui.separator();

                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for (i, e) in self.speed_history.iter().enumerate().rev() {
                        ui.horizontal(|ui| {
                            let row = [
                                format!("{}", i + 1),
                                e.timestamp.clone(),
                                e.dl_mbps.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "—".into()),
                                e.ul_mbps.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "—".into()),
                                e.ping_ms.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "—".into()),
                                e.jitter_ms.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "—".into()),
                            ];
                            for val in &row {
                                ui.label(RichText::new(val).font(FontId::monospace(11.0))
                                    .color(Color32::from_rgb(200, 223, 240)));
                                ui.add_space(20.0);
                            }
                        });
                    }
                });
            });
    }

    fn draw_log(&self, ui: &mut egui::Ui) {
        egui::Frame::none().fill(PANEL).rounding(8.0)
            .inner_margin(egui::Margin::same(10.0))
            .show(ui, |ui| {
                section_title(ui, "SYSTEM LOG");
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(70.0).stick_to_bottom(true)
                    .show(ui, |ui| {
                        for (msg, color) in self.log_lines.lock().unwrap().iter() {
                            ui.label(RichText::new(msg).font(FontId::monospace(9.0)).color(*color));
                        }
                    });
            });
    }
}

// ── ヘルパー ─────────────────────────────────────

fn gauge_card(ui: &mut egui::Ui, label: &str, val: Option<f64>,
              unit: &str, color: Color32, max: f64, _testing: bool) {
    egui::Frame::none().fill(PANEL).rounding(8.0)
        .stroke(Stroke::new(1.0, color.linear_multiply(0.3)))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.label(RichText::new(label).font(FontId::monospace(9.0)).color(MUTED));
            ui.add_space(6.0);
            ui.label(RichText::new(val.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "—".into()))
                .font(FontId::monospace(34.0))
                .color(if val.is_some() { color } else { MUTED }).strong());
            ui.label(RichText::new(unit).font(FontId::monospace(10.0)).color(MUTED));
            ui.add_space(6.0);
            let pct = val.map(|v| (v / max).min(1.0) as f32).unwrap_or(0.0);
            let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 1.0, Color32::from_rgba_unmultiplied(255,255,255,8));
            if pct > 0.0 {
                let f = egui::Rect::from_min_size(r.min, Vec2::new(r.width() * pct, r.height()));
                ui.painter().rect_filled(f, 1.0, color);
            }
        });
}

fn section_title(ui: &mut egui::Ui, t: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(t).font(FontId::monospace(9.0)).color(MUTED));
        ui.add(egui::Separator::default().horizontal().spacing(6.0));
    });
}

fn info_row(ui: &mut egui::Ui, k: &str, v: Option<String>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(k).font(FontId::monospace(9.0)).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(v.unwrap_or_else(|| "—".into()))
                .font(FontId::monospace(10.0)).color(Color32::from_rgb(200,223,240)));
        });
    });
}

fn legend_dot(ui: &mut egui::Ui, color: Color32, label: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(10.0, 3.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 1.0, color);
        ui.label(RichText::new(label).font(FontId::monospace(9.0)).color(MUTED));
    });
}

fn pts(data: &[f64]) -> PlotPoints {
    data.iter().enumerate().map(|(i, &v)| [i as f64, v]).collect()
}

fn now_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let s = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("{:02}:{:02}:{:02}", (s/3600)%24, (s/60)%60, s%60)
}
