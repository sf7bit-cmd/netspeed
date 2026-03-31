#![allow(unused_imports)]
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use egui::{Color32, FontId, RichText, Stroke, Vec2};
use egui_plot::{Line, Plot, PlotPoints};
use crate::{measure, scan, history, wifi, iperf};

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
    status:   String,
    hosts:    Vec<scan::HostResult>,
}

#[derive(Default, Clone, PartialEq)]
enum IperfMode { #[default] Idle, Running, Done }

#[derive(Default, Clone)]
struct IperfState {
    mode:      IperfMode,
    smb_path:  String,
    live_mbps: f64,
    result:    Option<iperf::IperfResult>,
    history:   Vec<(String, f64)>,
}

pub struct App {
    test_state:    TestState,
    result:        Arc<Mutex<SpeedResult>>,
    dl_history:    Arc<Mutex<Vec<f64>>>,
    ul_history:    Arc<Mutex<Vec<f64>>>,
    ping_history:  Arc<Mutex<Vec<f64>>>,
    log_lines:     Arc<Mutex<Vec<(String, Color32)>>>,
    stop_flag:     Arc<AtomicBool>,
    subnet_input:  String,
    scan_state:    Arc<Mutex<ScanState>>,
    scan_stop:     Arc<AtomicBool>,
    speed_history: Vec<history::HistoryEntry>,
    wifi_info:     Arc<Mutex<wifi::WifiInfo>>,
    wifi_timer:    f64,
    iperf_state:   Arc<Mutex<IperfState>>,
    iperf_stop:    Arc<AtomicBool>,
    active_tab:    u8,
}

fn now_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let s = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("{:02}:{:02}:{:02}", (s/3600)%24, (s/60)%60, s%60)
}

fn pts(data: &[f64]) -> PlotPoints {
    data.iter().enumerate().map(|(i, &v)| [i as f64, v]).collect()
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
            ui.label(RichText::new(v.unwrap_or_else(|| "-".into()))
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

fn gauge_card(ui: &mut egui::Ui, label: &str, val: Option<f64>,
              unit: &str, color: Color32, max: f64) {
    egui::Frame::none().fill(PANEL).rounding(8.0)
        .stroke(Stroke::new(1.0, color.linear_multiply(0.3)))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.label(RichText::new(label).font(FontId::monospace(9.0)).color(MUTED));
            ui.add_space(6.0);
            ui.label(RichText::new(val.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "-".into()))
                .font(FontId::monospace(34.0))
                .color(if val.is_some() { color } else { MUTED }).strong());
            ui.label(RichText::new(unit).font(FontId::monospace(10.0)).color(MUTED));
            ui.add_space(6.0);
            let pct = val.map(|v| (v / max).min(1.0) as f32).unwrap_or(0.0);
            let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 1.0, Color32::from_rgba_unmultiplied(255,255,255,8));
            if pct > 0.0 {
                let f = egui::Rect::from_min_size(r.min, Vec2::new(r.width()*pct, r.height()));
                ui.painter().rect_filled(f, 1.0, color);
            }
        });
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Japanese font
        let mut fonts = egui::FontDefinitions::default();
        for path in &[
            "C:/Windows/Fonts/meiryo.ttc",
            "C:/Windows/Fonts/msgothic.ttc",
            "C:/Windows/Fonts/YuGothM.ttc",
        ] {
            if let Ok(data) = std::fs::read(path) {
                fonts.font_data.insert("jp".to_owned(), egui::FontData::from_owned(data));
                fonts.families.entry(egui::FontFamily::Proportional).or_default().push("jp".to_owned());
                fonts.families.entry(egui::FontFamily::Monospace).or_default().push("jp".to_owned());
                break;
            }
        }
        cc.egui_ctx.set_fonts(fonts);

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = PANEL;
        visuals.override_text_color = Some(Color32::from_rgb(200, 223, 240));
        cc.egui_ctx.set_visuals(visuals);

        let app = Self {
            test_state:    TestState::Idle,
            result:        Arc::new(Mutex::new(SpeedResult::default())),
            dl_history:    Arc::new(Mutex::new(Vec::new())),
            ul_history:    Arc::new(Mutex::new(Vec::new())),
            ping_history:  Arc::new(Mutex::new(Vec::new())),
            log_lines:     Arc::new(Mutex::new(vec![
                (format!("[{}] NetSpeed Analyzer v0.2", now_str()), ACCENT3),
            ])),
            stop_flag:     Arc::new(AtomicBool::new(false)),
            subnet_input:  "192.168.1".into(),
            scan_state:    Arc::new(Mutex::new(ScanState::default())),
            scan_stop:     Arc::new(AtomicBool::new(false)),
            speed_history: Vec::new(),
            wifi_info:     Arc::new(Mutex::new(wifi::WifiInfo::default())),
            wifi_timer:    0.0,
            iperf_state:   Arc::new(Mutex::new(IperfState::default())),
            iperf_stop:    Arc::new(AtomicBool::new(false)),
            active_tab:    0,
        };
        // Initial wifi fetch
        let wi = Arc::clone(&app.wifi_info);
        thread::spawn(move || { *wi.lock().unwrap() = wifi::get_wifi_info(); });
        app
    }

    fn plog(&self, msg: &str, color: Color32) {
        self.log_lines.lock().unwrap().push((format!("[{}] {}", now_str(), msg), color));
    }

    fn start_test(&mut self, ctx: &egui::Context) {
        self.test_state = TestState::Running;
        self.stop_flag.store(false, Ordering::Relaxed);
        self.dl_history.lock().unwrap().clear();
        self.ul_history.lock().unwrap().clear();
        self.ping_history.lock().unwrap().clear();
        *self.result.lock().unwrap() = SpeedResult::default();

        let result   = Arc::clone(&self.result);
        let dl_hist  = Arc::clone(&self.dl_history);
        let ul_hist  = Arc::clone(&self.ul_history);
        let ph       = Arc::clone(&self.ping_history);
        let log      = Arc::clone(&self.log_lines);
        let stop     = Arc::clone(&self.stop_flag);
        let ctx      = ctx.clone();

        thread::spawn(move || {
            macro_rules! log {
                ($msg:expr, $c:expr) => {
                    log.lock().unwrap().push((format!("[{}] {}", now_str(), $msg), $c));
                    ctx.request_repaint();
                }
            }

            log!("Ping measuring...", MUTED);
            if let Some(p) = measure::measure_ping(5) {
                let mut r = result.lock().unwrap();
                r.ping_ms = Some(p.avg); r.jitter = Some(p.jitter);
                r.min_ping = Some(p.min); r.max_ping = Some(p.max);
                ph.lock().unwrap().push(p.avg);
                log!(format!("Ping: {:.1}ms  jitter: {:.1}ms", p.avg, p.jitter), ACCENT3);
            } else { log!("Ping failed", DANGER); }
            if stop.load(Ordering::Relaxed) { return; }

            log!("Download measuring...", MUTED);
            let dl2 = Arc::clone(&dl_hist);
            let log2 = Arc::clone(&log);
            let ctx2 = ctx.clone();
            let fdl = measure::measure_download(3, move |mbps| {
                dl2.lock().unwrap().push(mbps);
                log2.lock().unwrap().push((format!("[{}]   DL: {:.1} Mbps", now_str(), mbps), MUTED));
                ctx2.request_repaint();
            });
            if let Some(dl) = fdl {
                result.lock().unwrap().dl_mbps = Some(dl);
                log!(format!("Download: {:.1} Mbps", dl), ACCENT);
            } else { log!("Download failed", DANGER); }
            if stop.load(Ordering::Relaxed) { return; }

            log!("Upload measuring...", MUTED);
            let ul2 = Arc::clone(&ul_hist);
            let log3 = Arc::clone(&log);
            let ctx3 = ctx.clone();
            let ful = measure::measure_upload(2, move |mbps| {
                ul2.lock().unwrap().push(mbps);
                log3.lock().unwrap().push((format!("[{}]   UL: {:.1} Mbps", now_str(), mbps), MUTED));
                ctx3.request_repaint();
            });
            if let Some(ul) = ful {
                result.lock().unwrap().ul_mbps = Some(ul);
                log!(format!("Upload: {:.1} Mbps", ul), ACCENT2);
            } else { log!("Upload failed", DANGER); }
            log!("Test complete", ACCENT3);
        });
    }

    fn start_scan(&mut self, ctx: &egui::Context) {
        let parts: Vec<&str> = self.subnet_input.split('.').collect();
        if parts.len() != 3 {
            self.plog("Subnet format error (e.g. 192.168.1)", DANGER);
            return;
        }
        let subnet = [
            parts[0].parse::<u8>().unwrap_or(192),
            parts[1].parse::<u8>().unwrap_or(168),
            parts[2].parse::<u8>().unwrap_or(1),
        ];
        {
            let mut s = self.scan_state.lock().unwrap();
            s.running = true; s.progress = 0.0;
            s.status = "Getting ARP table...".into();
            s.hosts.clear();
        }
        self.scan_stop.store(false, Ordering::Relaxed);
        self.plog(&format!("Scan: {}.1-254", self.subnet_input), ACCENT);

        let ss   = Arc::clone(&self.scan_state);
        let log  = Arc::clone(&self.log_lines);
        let stop = Arc::clone(&self.scan_stop);
        let ctx  = ctx.clone();

        thread::spawn(move || {
            let ss2 = Arc::clone(&ss);
            let ss3 = Arc::clone(&ss);
            let log2 = Arc::clone(&log);
            let ctx2 = ctx.clone();
            let ctx3 = ctx.clone();

            scan::scan_subnet(subnet, 500,
                move |host| {
                    let name = host.hostname.clone().unwrap_or_else(|| "-".into());
                    log2.lock().unwrap().push((
                        format!("[{}] {} ({})  {:.0}ms", now_str(), host.ip, name, host.latency_ms),
                        ACCENT3));
                    let mut state = ss2.lock().unwrap();
                    if let Some(ex) = state.hosts.iter_mut().find(|h| h.ip == host.ip) {
                        if host.latency_ms > 0.0 { *ex = host; }
                    } else {
                        state.hosts.push(host);
                        state.hosts.sort_by_key(|h| {
                            let o = h.ip.octets();
                            ((o[0] as u32)<<24)|((o[1] as u32)<<16)|((o[2] as u32)<<8)|(o[3] as u32)
                        });
                    }
                    ctx2.request_repaint();
                },
                move |p, status| {
                    let mut s = ss3.lock().unwrap();
                    s.progress = p; s.status = status;
                    ctx3.request_repaint();
                },
                stop,
            );
            let found = ss.lock().unwrap().hosts.len();
            let mut s = ss.lock().unwrap();
            s.running = false;
            s.status = format!("Done: {} hosts", found);
            log.lock().unwrap().push((
                format!("[{}] Scan done: {} hosts", now_str(), found),
                if found > 0 { ACCENT3 } else { MUTED }));
            ctx.request_repaint();
        });
    }

    fn start_smb_write(&mut self, ctx: &egui::Context) {
        let path = self.iperf_state.lock().unwrap().smb_path.clone();
        if path.is_empty() { self.plog("Enter SMB path", DANGER); return; }
        self.iperf_stop.store(false, Ordering::Relaxed);
        { let mut s = self.iperf_state.lock().unwrap(); s.mode = IperfMode::Running; s.live_mbps = 0.0; }
        self.plog(&format!("SMB Write: {}", path), ACCENT);

        let state = Arc::clone(&self.iperf_state);
        let log   = Arc::clone(&self.log_lines);
        let stop  = Arc::clone(&self.iperf_stop);
        let ctx   = ctx.clone();

        thread::spawn(move || {
            let s2 = Arc::clone(&state);
            let c2 = ctx.clone();
            let result = iperf::measure_smb_write(&path, stop, move |mbps| {
                s2.lock().unwrap().live_mbps = mbps;
                c2.request_repaint();
            });
            match result {
                Ok(r) => {
                    log.lock().unwrap().push((
                        format!("[{}] SMB Write: {:.1} Mbps", now_str(), r.mbps), ACCENT3));
                    let mut s = state.lock().unwrap();
                    s.history.push(("Write".into(), r.mbps));
                    s.result = Some(r); s.mode = IperfMode::Done;
                }
                Err(e) => {
                    log.lock().unwrap().push((format!("[{}] Error: {}", now_str(), e), DANGER));
                    state.lock().unwrap().mode = IperfMode::Idle;
                }
            }
            ctx.request_repaint();
        });
    }

    fn start_smb_read(&mut self, ctx: &egui::Context) {
        let path = self.iperf_state.lock().unwrap().smb_path.clone();
        if path.is_empty() { self.plog("Enter SMB path", DANGER); return; }
        self.iperf_stop.store(false, Ordering::Relaxed);
        { let mut s = self.iperf_state.lock().unwrap(); s.mode = IperfMode::Running; s.live_mbps = 0.0; }
        self.plog(&format!("SMB Read: {}", path), ACCENT2);

        let state = Arc::clone(&self.iperf_state);
        let log   = Arc::clone(&self.log_lines);
        let stop  = Arc::clone(&self.iperf_stop);
        let ctx   = ctx.clone();

        thread::spawn(move || {
            let s2 = Arc::clone(&state);
            let c2 = ctx.clone();
            let result = iperf::measure_smb_read(&path, stop, move |mbps| {
                s2.lock().unwrap().live_mbps = mbps;
                c2.request_repaint();
            });
            match result {
                Ok(r) => {
                    log.lock().unwrap().push((
                        format!("[{}] SMB Read: {:.1} Mbps", now_str(), r.mbps), ACCENT2));
                    let mut s = state.lock().unwrap();
                    s.history.push(("Read".into(), r.mbps));
                    s.result = Some(r); s.mode = IperfMode::Done;
                }
                Err(e) => {
                    log.lock().unwrap().push((format!("[{}] Error: {}", now_str(), e), DANGER));
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
            let r = self.result.lock().unwrap();
            if r.dl_mbps.is_some() && r.ul_mbps.is_some() {
                let entry = history::new_entry(r.dl_mbps, r.ul_mbps, r.ping_ms, r.jitter);
                drop(r);
                self.speed_history.push(entry);
                self.test_state = TestState::Done;
            }
        }

        self.wifi_timer += ctx.input(|i| i.unstable_dt) as f64;
        if self.wifi_timer > 30.0 {
            self.wifi_timer = 0.0;
            let wi = Arc::clone(&self.wifi_info);
            thread::spawn(move || { *wi.lock().unwrap() = wifi::get_wifi_info(); });
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(14.0)))
            .show(ctx, |ui| {
                // Header
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(40.0, 40.0), egui::Sense::hover());
                    ui.painter().rect_stroke(r, 6.0, Stroke::new(2.0, ACCENT));
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER,
                        "NS", FontId::monospace(16.0), ACCENT);
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("NETSPEED ANALYZER")
                            .font(FontId::proportional(18.0)).color(Color32::WHITE).strong());
                        ui.label(RichText::new("v0.2  LAN NETWORK TOOL")
                            .font(FontId::monospace(9.0)).color(MUTED));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let wifi = self.wifi_info.lock().unwrap().clone();
                        if let Some(pct) = wifi.signal_pct {
                            let c = if pct >= 70 { ACCENT3 } else if pct >= 40 { WARN } else { DANGER };
                            ui.label(RichText::new(format!("WiFi {}%", pct)).font(FontId::monospace(11.0)).color(c));
                            if let Some(ssid) = &wifi.ssid {
                                ui.label(RichText::new(ssid).font(FontId::monospace(10.0)).color(MUTED));
                            }
                            ui.add_space(8.0);
                        }
                        let (st, sc) = match self.test_state {
                            TestState::Idle    => ("READY",   ACCENT3),
                            TestState::Running => ("TESTING", ACCENT),
                            TestState::Done    => ("DONE",    ACCENT3),
                        };
                        ui.label(RichText::new(format!("● {}", st)).font(FontId::monospace(11.0)).color(sc));
                    });
                });
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(8.0);

                // Tabs
                ui.horizontal(|ui| {
                    for (i, label) in ["SPEED TEST", "LAN SCAN", "LAN THROUGHPUT", "HISTORY"].iter().enumerate() {
                        let active = self.active_tab == i as u8;
                        let color = if active { ACCENT } else { MUTED };
                        let fill  = if active { Color32::from_rgba_unmultiplied(0,212,255,18) } else { Color32::TRANSPARENT };
                        if ui.add(egui::Button::new(RichText::new(*label).font(FontId::monospace(11.0)).color(color))
                            .fill(fill).stroke(Stroke::new(if active {1.5} else {0.5}, color))
                            .min_size(Vec2::new(120.0, 28.0))
                        ).clicked() { self.active_tab = i as u8; }
                        ui.add_space(4.0);
                    }
                });
                ui.add_space(10.0);

                match self.active_tab {
                    0 => self.draw_speed(ui, ctx),
                    1 => self.draw_lan(ui, ctx),
                    2 => self.draw_throughput(ui, ctx),
                    3 => self.draw_history(ui),
                    _ => {}
                }

                ui.add_space(8.0);
                // Log
                egui::Frame::none().fill(PANEL).rounding(8.0)
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        section_title(ui, "SYSTEM LOG");
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical().max_height(70.0).stick_to_bottom(true).show(ui, |ui| {
                            for (msg, color) in self.log_lines.lock().unwrap().iter() {
                                ui.label(RichText::new(msg).font(FontId::monospace(9.0)).color(*color));
                            }
                        });
                    });
            });
    }
}

impl App {
    fn draw_speed(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let r = self.result.lock().unwrap().clone();
        let testing = self.test_state == TestState::Running;
        ui.columns(3, |cols| {
            gauge_card(&mut cols[0], "DOWN",  r.dl_mbps, "Mbps", ACCENT,  1000.0);
            gauge_card(&mut cols[1], "UP",    r.ul_mbps, "Mbps", ACCENT2, 1000.0);
            gauge_card(&mut cols[2], "PING",  r.ping_ms, "ms",   ACCENT3,  200.0);
        });
        ui.add_space(8.0);
        ui.columns(2, |cols| {
            // Graph
            {
                let ui = &mut cols[0];
                let dl   = self.dl_history.lock().unwrap().clone();
                let ul   = self.ul_history.lock().unwrap().clone();
                let ping = self.ping_history.lock().unwrap().clone();
                egui::Frame::none().fill(PANEL).rounding(8.0).inner_margin(egui::Margin::same(12.0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("REALTIME").font(FontId::monospace(9.0)).color(MUTED));
                        legend_dot(ui, ACCENT, "DL");
                        legend_dot(ui, ACCENT2, "UL");
                        legend_dot(ui, ACCENT3, "Ping");
                    });
                    Plot::new("sg").height(120.0).show_axes([false, true])
                        .allow_zoom(false).allow_drag(false).show(ui, |p| {
                            if !dl.is_empty()   { p.line(Line::new(pts(&dl)).color(ACCENT).width(2.0)); }
                            if !ul.is_empty()   { p.line(Line::new(pts(&ul)).color(ACCENT2).width(2.0)); }
                            if !ping.is_empty() { p.line(Line::new(pts(&ping)).color(ACCENT3).width(1.5)); }
                        });
                });
            }
            // Controls
            {
                let ui = &mut cols[1];
                egui::Frame::none().fill(PANEL).rounding(8.0).inner_margin(egui::Margin::same(12.0)).show(ui, |ui| {
                    section_title(ui, "CONTROLS");
                    ui.add_space(6.0);
                    let w = ui.available_width();
                    if ui.add_enabled(!testing, egui::Button::new(
                        RichText::new("START TEST").font(FontId::monospace(13.0)).color(ACCENT))
                        .min_size(Vec2::new(w, 38.0)).stroke(Stroke::new(1.5, ACCENT))
                        .fill(Color32::from_rgba_unmultiplied(0,212,255,12))
                    ).clicked() { self.start_test(ctx); }
                    ui.add_space(4.0);
                    if ui.add_enabled(testing, egui::Button::new(
                        RichText::new("STOP").font(FontId::monospace(12.0)).color(DANGER))
                        .min_size(Vec2::new(w, 30.0))
                        .stroke(Stroke::new(1.0, if testing { DANGER } else { MUTED }))
                        .fill(Color32::TRANSPARENT)
                    ).clicked() { self.stop_flag.store(true, Ordering::Relaxed); self.test_state = TestState::Idle; }
                    ui.add_space(4.0);
                    if ui.add_enabled(!testing, egui::Button::new(
                        RichText::new("RESET").font(FontId::monospace(10.0)).color(MUTED))
                        .min_size(Vec2::new(w, 24.0)).stroke(Stroke::new(1.0, MUTED))
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
                    let r2 = self.result.lock().unwrap().clone();
                    info_row(ui, "MIN PING", r2.min_ping.map(|v| format!("{:.1} ms", v)));
                    info_row(ui, "MAX PING", r2.max_ping.map(|v| format!("{:.1} ms", v)));
                    info_row(ui, "JITTER",   r2.jitter.map(|v| format!("{:.1} ms", v)));
                    info_row(ui, "SERVER",   Some("speed.cloudflare.com".into()));
                    ui.add_space(10.0);
                    section_title(ui, "SAVE HISTORY");
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new("CSV").font(FontId::monospace(11.0)).color(ACCENT))
                            .stroke(Stroke::new(1.0, ACCENT)).fill(Color32::TRANSPARENT)).clicked() {
                            match history::save_csv(&self.speed_history) {
                                Ok(p)  => self.plog(&format!("CSV: {}", p.display()), ACCENT3),
                                Err(e) => self.plog(&format!("Error: {}", e), DANGER),
                            }
                        }
                        if ui.add(egui::Button::new(RichText::new("JSON").font(FontId::monospace(11.0)).color(ACCENT2))
                            .stroke(Stroke::new(1.0, ACCENT2)).fill(Color32::TRANSPARENT)).clicked() {
                            match history::save_json(&self.speed_history) {
                                Ok(p)  => self.plog(&format!("JSON: {}", p.display()), ACCENT3),
                                Err(e) => self.plog(&format!("Error: {}", e), DANGER),
                            }
                        }
                    });
                });
            }
        });
    }

    fn draw_lan(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::Frame::none().fill(PANEL).rounding(8.0).inner_margin(egui::Margin::same(14.0)).show(ui, |ui| {
            section_title(ui, "LAN HOST SCAN");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Subnet:").font(FontId::monospace(11.0)).color(MUTED));
                ui.add(egui::TextEdit::singleline(&mut self.subnet_input)
                    .font(FontId::monospace(12.0)).desired_width(120.0));
                ui.label(RichText::new(".1-254").font(FontId::monospace(11.0)).color(MUTED));
                ui.add_space(10.0);
                let running = self.scan_state.lock().unwrap().running;
                if ui.add_enabled(!running, egui::Button::new(
                    RichText::new("SCAN").font(FontId::monospace(12.0)).color(ACCENT))
                    .stroke(Stroke::new(1.0, ACCENT)).fill(Color32::TRANSPARENT)
                ).clicked() { self.start_scan(ctx); }
                if running {
                    if ui.add(egui::Button::new(RichText::new("STOP").font(FontId::monospace(11.0)).color(DANGER))
                        .stroke(Stroke::new(1.0, DANGER)).fill(Color32::TRANSPARENT)
                    ).clicked() { self.scan_stop.store(true, Ordering::Relaxed); }
                }
            });
            {
                let s = self.scan_state.lock().unwrap();
                if s.running || s.progress > 0.0 {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::ProgressBar::new(s.progress).desired_width(ui.available_width() - 160.0));
                        ui.label(RichText::new(&s.status).font(FontId::monospace(10.0)).color(MUTED));
                    });
                }
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("IP ADDRESS").font(FontId::monospace(10.0)).color(MUTED));
                ui.add_space(80.0);
                ui.label(RichText::new("HOSTNAME").font(FontId::monospace(10.0)).color(MUTED));
                ui.add_space(80.0);
                ui.label(RichText::new("LATENCY").font(FontId::monospace(10.0)).color(MUTED));
                ui.add_space(30.0);
                ui.label(RichText::new("PORTS").font(FontId::monospace(10.0)).color(MUTED));
            });
            ui.separator();
            let hosts = self.scan_state.lock().unwrap().hosts.clone();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if hosts.is_empty() {
                    ui.label(RichText::new("Waiting for scan...").font(FontId::monospace(11.0)).color(MUTED));
                }
                for host in &hosts {
                    ui.horizontal(|ui| {
                        let (dr, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                        ui.painter().circle_filled(dr.center(), 4.0, ACCENT3);
                        let ip_str = host.ip.to_string();
                        let web_url = host.web_port.map(|p| {
                            let s = if p == 443 || p == 8443 { "https" } else { "http" };
                            format!("{}://{}:{}", s, ip_str, p)
                        });
                        if let Some(ref url) = web_url {
                            if ui.add(egui::Button::new(
                                RichText::new(&ip_str).font(FontId::monospace(12.0)).color(ACCENT))
                                .stroke(Stroke::new(0.5, ACCENT)).fill(Color32::TRANSPARENT)
                            ).on_hover_text(url).clicked() { let _ = open::that(url); }
                        } else {
                            ui.label(RichText::new(&ip_str).font(FontId::monospace(12.0))
                                .color(Color32::from_rgb(200, 223, 240)));
                        }
                        ui.add_space(8.0);
                        ui.label(RichText::new(host.hostname.clone().unwrap_or_else(|| "-".into()))
                            .font(FontId::monospace(11.0)).color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let ports: Vec<String> = host.open_ports.iter().map(|p| p.to_string()).collect();
                            ui.label(RichText::new(ports.join(" ")).font(FontId::monospace(10.0)).color(MUTED));
                            ui.add_space(20.0);
                            let ms = host.latency_ms;
                            let c = if ms < 5.0 { ACCENT3 } else if ms < 30.0 { WARN } else { DANGER };
                            ui.label(RichText::new(format!("{:.0} ms", ms)).font(FontId::monospace(11.0)).color(c));
                        });
                    });
                }
            });
        });
    }

    fn draw_throughput(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::Frame::none().fill(PANEL).rounding(8.0).inner_margin(egui::Margin::same(14.0)).show(ui, |ui| {
            section_title(ui, "LAN THROUGHPUT  (SMB)");
            ui.add_space(4.0);
            ui.label(RichText::new("Enter a shared folder path. No extra software needed.")
                .font(FontId::monospace(10.0)).color(MUTED));
            ui.add_space(10.0);

            egui::Frame::none().stroke(Stroke::new(1.0, MUTED)).rounding(6.0)
                .inner_margin(egui::Margin::same(12.0)).show(ui, |ui| {
                    ui.label(RichText::new("SMB Path").font(FontId::monospace(10.0)).color(MUTED));
                    ui.add_space(4.0);
                    let mut path = self.iperf_state.lock().unwrap().smb_path.clone();
                    if ui.add(egui::TextEdit::singleline(&mut path)
                        .font(FontId::monospace(13.0)).desired_width(ui.available_width())
                        .hint_text(r"e.g. \\192.168.1.10\share  or  Z:\")
                    ).changed() { self.iperf_state.lock().unwrap().smb_path = path; }
                });

            ui.add_space(10.0);
            let busy = self.iperf_state.lock().unwrap().mode == IperfMode::Running;
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new(
                    RichText::new("Write Test").font(FontId::monospace(12.0)).color(ACCENT))
                    .stroke(Stroke::new(1.0, ACCENT))
                    .fill(Color32::from_rgba_unmultiplied(0,212,255,10))
                    .min_size(Vec2::new(160.0, 36.0))
                ).clicked() { self.start_smb_write(ctx); }
                ui.add_space(8.0);
                if ui.add_enabled(!busy, egui::Button::new(
                    RichText::new("Read Test").font(FontId::monospace(12.0)).color(ACCENT2))
                    .stroke(Stroke::new(1.0, ACCENT2))
                    .fill(Color32::from_rgba_unmultiplied(255,107,53,10))
                    .min_size(Vec2::new(160.0, 36.0))
                ).clicked() { self.start_smb_read(ctx); }
                if busy {
                    ui.add_space(8.0);
                    if ui.add(egui::Button::new(RichText::new("STOP").font(FontId::monospace(11.0)).color(DANGER))
                        .stroke(Stroke::new(1.0, DANGER)).fill(Color32::TRANSPARENT)
                    ).clicked() { self.iperf_stop.store(true, Ordering::Relaxed); }
                }
            });

            if busy {
                ui.add_space(10.0);
                let live = self.iperf_state.lock().unwrap().live_mbps;
                egui::Frame::none().fill(Color32::from_rgba_unmultiplied(0,212,255,6))
                    .rounding(6.0).inner_margin(egui::Margin::same(12.0)).show(ui, |ui| {
                        ui.label(RichText::new(format!("{:.1} Mbps", live))
                            .font(FontId::monospace(28.0)).color(ACCENT).strong());
                    });
            }

            let state = self.iperf_state.lock().unwrap().clone();
            if let Some(ref r) = state.result {
                ui.add_space(10.0);
                egui::Frame::none().fill(Color32::from_rgba_unmultiplied(57,255,20,6))
                    .rounding(6.0).inner_margin(egui::Margin::same(14.0)).show(ui, |ui| {
                        ui.label(RichText::new(format!("{:.1} Mbps", r.mbps))
                            .font(FontId::monospace(40.0))
                            .color(if r.direction.starts_with("Write") { ACCENT } else { ACCENT2 }).strong());
                        ui.label(RichText::new(format!("{}  |  {:.0} MB  |  {:.1}s",
                            r.direction, r.bytes as f64/1_048_576.0, r.duration))
                            .font(FontId::monospace(10.0)).color(MUTED));
                    });
            }

            if !state.history.is_empty() {
                ui.add_space(10.0);
                section_title(ui, "HISTORY");
                let max = state.history.iter().map(|(_, v)| *v).fold(100.0f64, f64::max);
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (dir, mbps) in &state.history {
                            let color = if dir == "Write" { ACCENT } else { ACCENT2 };
                            let pct = (*mbps / max) as f32;
                            ui.vertical(|ui| {
                                ui.label(RichText::new(format!("{:.0}", mbps))
                                    .font(FontId::monospace(9.0)).color(color));
                                let (r, _) = ui.allocate_exact_size(Vec2::new(36.0, 60.0), egui::Sense::hover());
                                ui.painter().rect_filled(
                                    egui::Rect::from_min_size(
                                        egui::pos2(r.min.x, r.max.y - r.height()*pct),
                                        Vec2::new(r.width(), r.height()*pct)), 2.0, color);
                                ui.painter().rect_stroke(r, 2.0, Stroke::new(0.5, MUTED));
                                ui.label(RichText::new(if dir == "Write" { "W" } else { "R" })
                                    .font(FontId::monospace(9.0)).color(MUTED));
                            });
                            ui.add_space(4.0);
                        }
                    });
                });
            }
        });
    }

    fn draw_history(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none().fill(PANEL).rounding(8.0).inner_margin(egui::Margin::same(14.0)).show(ui, |ui| {
            section_title(ui, "MEASUREMENT HISTORY");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Save CSV").font(FontId::monospace(11.0)).color(ACCENT))
                    .stroke(Stroke::new(1.0, ACCENT)).fill(Color32::TRANSPARENT)).clicked() {
                    match history::save_csv(&self.speed_history) {
                        Ok(p)  => self.plog(&format!("CSV: {}", p.display()), ACCENT3),
                        Err(e) => self.plog(&format!("Error: {}", e), DANGER),
                    }
                }
                if ui.add(egui::Button::new(RichText::new("Save JSON").font(FontId::monospace(11.0)).color(ACCENT2))
                    .stroke(Stroke::new(1.0, ACCENT2)).fill(Color32::TRANSPARENT)).clicked() {
                    match history::save_json(&self.speed_history) {
                        Ok(p)  => self.plog(&format!("JSON: {}", p.display()), ACCENT3),
                        Err(e) => self.plog(&format!("Error: {}", e), DANGER),
                    }
                }
                if ui.add(egui::Button::new(RichText::new("Clear").font(FontId::monospace(11.0)).color(MUTED))
                    .stroke(Stroke::new(1.0, MUTED)).fill(Color32::TRANSPARENT)).clicked() {
                    self.speed_history.clear();
                }
            });
            ui.add_space(8.0);

            if self.speed_history.is_empty() {
                ui.label(RichText::new("No history. Run Speed Test to record.")
                    .font(FontId::monospace(11.0)).color(MUTED));
                return;
            }

            let dl_pts: PlotPoints = self.speed_history.iter().enumerate()
                .filter_map(|(i, e)| e.dl_mbps.map(|v| [i as f64, v])).collect();
            let ul_pts: PlotPoints = self.speed_history.iter().enumerate()
                .filter_map(|(i, e)| e.ul_mbps.map(|v| [i as f64, v])).collect();
            let pp: PlotPoints = self.speed_history.iter().enumerate()
                .filter_map(|(i, e)| e.ping_ms.map(|v| [i as f64, v])).collect();

            Plot::new("hg").height(110.0).show_axes([false, true])
                .allow_zoom(false).allow_drag(false).show(ui, |p| {
                    p.line(Line::new(dl_pts).color(ACCENT).width(2.0));
                    p.line(Line::new(ul_pts).color(ACCENT2).width(2.0));
                    p.line(Line::new(pp).color(ACCENT3).width(1.5));
                });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for h in ["#", "Timestamp", "DL(Mbps)", "UL(Mbps)", "Ping(ms)", "Jitter(ms)"] {
                    ui.label(RichText::new(h).font(FontId::monospace(10.0)).color(MUTED));
                    ui.add_space(20.0);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                for (i, e) in self.speed_history.iter().enumerate().rev() {
                    ui.horizontal(|ui| {
                        for val in &[
                            format!("{}", i+1),
                            e.timestamp.clone(),
                            e.dl_mbps.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "-".into()),
                            e.ul_mbps.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "-".into()),
                            e.ping_ms.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "-".into()),
                            e.jitter_ms.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "-".into()),
                        ] {
                            ui.label(RichText::new(val).font(FontId::monospace(11.0))
                                .color(Color32::from_rgb(200, 223, 240)));
                            ui.add_space(20.0);
                        }
                    });
                }
            });
        });
    }
}
