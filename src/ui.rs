// src/ui.rs — egui メイン UI

use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use egui::{Color32, FontId, RichText, Stroke, Vec2};
use egui_plot::{Line, Plot, PlotPoints};

use crate::measure;
use crate::scan::{self, HostResult};

// ── カラーパレット ─────────────────────────────
const BG:       Color32 = Color32::from_rgb(8,  12, 16);
const PANEL:    Color32 = Color32::from_rgb(13, 21, 32);
const ACCENT:   Color32 = Color32::from_rgb(0,  212, 255);
const ACCENT2:  Color32 = Color32::from_rgb(255, 107, 53);
const ACCENT3:  Color32 = Color32::from_rgb(57,  255, 20);
const MUTED:    Color32 = Color32::from_rgb(58,  84, 112);
const DANGER:   Color32 = Color32::from_rgb(255, 45,  85);

// ── 状態 ─────────────────────────────────────
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

pub struct App {
    // 速度テスト
    test_state:    TestState,
    result:        Arc<Mutex<SpeedResult>>,
    dl_history:    Arc<Mutex<Vec<f64>>>,
    ul_history:    Arc<Mutex<Vec<f64>>>,
    ping_history:  Arc<Mutex<Vec<f64>>>,
    log_lines:     Arc<Mutex<Vec<(String, Color32)>>>,
    stop_flag:     Arc<AtomicBool>,

    // LAN スキャン
    subnet_input:  String,
    scan_state:    Arc<Mutex<ScanState>>,
    scan_stop:     Arc<AtomicBool>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // フォント設定
        let mut fonts = egui::FontDefinitions::default();
        // システムフォントをそのまま使用（日本語対応）
        cc.egui_ctx.set_fonts(fonts);

        // ダークスタイル
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill       = BG;
        visuals.window_fill      = PANEL;
        visuals.override_text_color = Some(Color32::from_rgb(200, 223, 240));
        cc.egui_ctx.set_visuals(visuals);

        Self {
            test_state:   TestState::Idle,
            result:       Arc::new(Mutex::new(SpeedResult::default())),
            dl_history:   Arc::new(Mutex::new(Vec::new())),
            ul_history:   Arc::new(Mutex::new(Vec::new())),
            ping_history: Arc::new(Mutex::new(Vec::new())),
            log_lines:    Arc::new(Mutex::new(vec![
                ("NetSpeed Analyzer 起動".into(), ACCENT3),
                ("speed.cloudflare.com を使用します".into(), MUTED),
            ])),
            stop_flag:    Arc::new(AtomicBool::new(false)),
            subnet_input: "192.168.1".into(),
            scan_state:   Arc::new(Mutex::new(ScanState::default())),
            scan_stop:    Arc::new(AtomicBool::new(false)),
        }
    }

    fn add_log(&self, msg: impl Into<String>, color: Color32) {
        let mut log = self.log_lines.lock().unwrap();
        let now = chrono_now();
        log.push((format!("[{}] {}", now, msg.into()), color));
        if log.len() > 200 { log.drain(0..50); }
    }

    fn start_test(&mut self, ctx: &egui::Context) {
        self.test_state = TestState::Running;
        self.stop_flag.store(false, Ordering::Relaxed);

        // clear histories
        self.dl_history.lock().unwrap().clear();
        self.ul_history.lock().unwrap().clear();
        self.ping_history.lock().unwrap().clear();
        *self.result.lock().unwrap() = SpeedResult::default();

        let result     = Arc::clone(&self.result);
        let dl_hist    = Arc::clone(&self.dl_history);
        let ul_hist    = Arc::clone(&self.ul_history);
        let ping_hist  = Arc::clone(&self.ping_history);
        let log        = Arc::clone(&self.log_lines);
        let stop       = Arc::clone(&self.stop_flag);
        let ctx        = ctx.clone();

        fn push_log(log: &Arc<Mutex<Vec<(String, Color32)>>>, msg: &str, color: Color32) {
            let mut l = log.lock().unwrap();
            l.push((format!("[{}] {}", chrono_now(), msg), color));
        }

        thread::spawn(move || {
            // ── Ping ──
            push_log(&log, "レイテンシ測定中...", MUTED);
            ctx.request_repaint();
            if let Some(p) = measure::measure_ping(5) {
                let mut r = result.lock().unwrap();
                r.ping_ms  = Some(p.avg);
                r.jitter   = Some(p.jitter);
                r.min_ping = Some(p.min);
                r.max_ping = Some(p.max);
                ping_hist.lock().unwrap().push(p.avg);
                push_log(&log, &format!("Ping: {:.1} ms (jitter {:.1} ms)", p.avg, p.jitter), ACCENT3);
            } else {
                push_log(&log, "Ping 失敗", DANGER);
            }
            ctx.request_repaint();
            if stop.load(Ordering::Relaxed) { return; }

            // ── Download ──
            push_log(&log, "ダウンロード測定中...", MUTED);
            ctx.request_repaint();
            let dl_hist2 = Arc::clone(&dl_hist);
            let log2     = Arc::clone(&log);
            let ctx2     = ctx.clone();
            let final_dl = measure::measure_download(3, move |mbps| {
                dl_hist2.lock().unwrap().push(mbps);
                push_log(&log2, &format!("  DL: {:.1} Mbps", mbps), MUTED);
                ctx2.request_repaint();
            });
            if let Some(dl) = final_dl {
                result.lock().unwrap().dl_mbps = Some(dl);
                push_log(&log, &format!("ダウンロード: {:.1} Mbps", dl), ACCENT);
            } else {
                push_log(&log, "DL 失敗", DANGER);
            }
            ctx.request_repaint();
            if stop.load(Ordering::Relaxed) { return; }

            // ── Upload ──
            push_log(&log, "アップロード測定中...", MUTED);
            ctx.request_repaint();
            let ul_hist2 = Arc::clone(&ul_hist);
            let log3     = Arc::clone(&log);
            let ctx3     = ctx.clone();
            let final_ul = measure::measure_upload(2, move |mbps| {
                ul_hist2.lock().unwrap().push(mbps);
                push_log(&log3, &format!("  UL: {:.1} Mbps", mbps), MUTED);
                ctx3.request_repaint();
            });
            if let Some(ul) = final_ul {
                result.lock().unwrap().ul_mbps = Some(ul);
                push_log(&log, &format!("アップロード: {:.1} Mbps", ul), ACCENT2);
            } else {
                push_log(&log, "UL 失敗", DANGER);
            }
            push_log(&log, "テスト完了 ✓", ACCENT3);
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

        {
            let mut s = self.scan_state.lock().unwrap();
            s.running  = true;
            s.progress = 0.0;
            s.hosts.clear();
        }
        self.scan_stop.store(false, Ordering::Relaxed);
        self.add_log(&format!("スキャン開始: {}.1–254", self.subnet_input), ACCENT);

        let scan_state = Arc::clone(&self.scan_state);
        let log        = Arc::clone(&self.log_lines);
        let stop       = Arc::clone(&self.scan_stop);
        let ctx        = ctx.clone();

        thread::spawn(move || {
            let ss_found = Arc::clone(&scan_state);
            let ss_prog  = Arc::clone(&scan_state);
            let log2     = Arc::clone(&log);
            let ctx2     = ctx.clone();
            let ctx3     = ctx.clone();

            scan::scan_subnet(
                subnet,
                500,
                move |host| {
                    {
                        let mut l = log2.lock().unwrap();
                        l.push((format!("[{}] 発見: {} ({:.0} ms)",
                            chrono_now(), host.ip, host.latency_ms), ACCENT3));
                    }
                    ss_found.lock().unwrap().hosts.push(host);
                    ctx2.request_repaint();
                },
                move |p| {
                    ss_prog.lock().unwrap().progress = p;
                    ctx3.request_repaint();
                },
                stop,
            );

            {
                let mut s = scan_state.lock().unwrap();
                let found = s.hosts.len();
                s.running  = false;
                s.progress = 1.0;
                let mut l = log.lock().unwrap();
                l.push((format!("[{}] スキャン完了: {}台発見", chrono_now(), found),
                    if found > 0 { ACCENT3 } else { MUTED }));
            }
            ctx.request_repaint();
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 連続再描画（テスト中）
        if self.test_state == TestState::Running {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }

        // 外部スレッドからの Running→Done 遷移チェック
        if self.test_state == TestState::Running {
            let r = self.result.lock().unwrap();
            if r.dl_mbps.is_some() && r.ul_mbps.is_some() {
                drop(r);
                self.test_state = TestState::Done;
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                self.draw_header(ui);
                ui.add_space(12.0);
                self.draw_gauges(ui);
                ui.add_space(10.0);
                self.draw_graph(ui);
                ui.add_space(10.0);
                self.draw_bottom(ui, ctx);
                ui.add_space(10.0);
                self.draw_log(ui);
            });
    }
}

impl App {
    fn draw_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // ロゴ
            let (rect, _) = ui.allocate_exact_size(Vec2::new(44.0, 44.0), egui::Sense::hover());
            ui.painter().rect_stroke(rect, 8.0, Stroke::new(2.0, ACCENT));
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER,
                "NS", FontId::monospace(18.0), ACCENT);

            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("NETSPEED ANALYZER")
                    .font(FontId::proportional(20.0)).color(Color32::WHITE).strong());
                ui.label(RichText::new("NETWORK SPEED TEST v0.1")
                    .font(FontId::monospace(10.0)).color(MUTED));
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (status_text, status_color) = match self.test_state {
                    TestState::Idle    => ("● READY",   ACCENT3),
                    TestState::Running => ("● TESTING", ACCENT),
                    TestState::Done    => ("● DONE",    ACCENT3),
                };
                ui.label(RichText::new(status_text).font(FontId::monospace(12.0)).color(status_color));
            });
        });
        ui.add_space(8.0);
        ui.separator();
    }

    fn draw_gauges(&self, ui: &mut egui::Ui) {
        let r = self.result.lock().unwrap().clone();
        let testing = self.test_state == TestState::Running;

        ui.columns(3, |cols| {
            gauge_card(&mut cols[0], "↓  DOWNLOAD", r.dl_mbps, "Mbps", ACCENT,   1000.0, testing);
            gauge_card(&mut cols[1], "↑  UPLOAD",   r.ul_mbps, "Mbps", ACCENT2,  1000.0, testing);
            gauge_card(&mut cols[2], "◎  LATENCY",  r.ping_ms, "ms",   ACCENT3,   200.0, testing);
        });
    }

    fn draw_graph(&self, ui: &mut egui::Ui) {
        let dl   = self.dl_history.lock().unwrap().clone();
        let ul   = self.ul_history.lock().unwrap().clone();
        let ping = self.ping_history.lock().unwrap().clone();

        egui::Frame::none()
            .fill(PANEL)
            .rounding(8.0)
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("REALTIME GRAPH").font(FontId::monospace(10.0)).color(MUTED));
                    ui.add_space(8.0);
                    legend_dot(ui, ACCENT,  "DL");
                    legend_dot(ui, ACCENT2, "UL");
                    legend_dot(ui, ACCENT3, "PING");
                });
                ui.add_space(6.0);

                Plot::new("speed_plot")
                    .height(90.0)
                    .show_axes([false, true])
                    .show_grid(true)
                    .allow_zoom(false)
                    .allow_drag(false)
                    .show(ui, |plot_ui| {
                        if !dl.is_empty() {
                            let pts: PlotPoints = dl.iter().enumerate()
                                .map(|(i, &v)| [i as f64, v]).collect();
                            plot_ui.line(Line::new(pts).color(ACCENT).width(2.0).name("DL Mbps"));
                        }
                        if !ul.is_empty() {
                            let pts: PlotPoints = ul.iter().enumerate()
                                .map(|(i, &v)| [i as f64, v]).collect();
                            plot_ui.line(Line::new(pts).color(ACCENT2).width(2.0).name("UL Mbps"));
                        }
                        if !ping.is_empty() {
                            let pts: PlotPoints = ping.iter().enumerate()
                                .map(|(i, &v)| [i as f64, v]).collect();
                            plot_ui.line(Line::new(pts).color(ACCENT3).width(1.5).name("Ping ms"));
                        }
                    });
            });
    }

    fn draw_bottom(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.columns(2, |cols| {
            // ── 左：LAN スキャン ──
            {
                let ui = &mut cols[0];
                egui::Frame::none()
                    .fill(PANEL).rounding(8.0)
                    .inner_margin(egui::Margin::same(12.0))
                    .show(ui, |ui| {
                        section_title(ui, "LAN HOST SCAN");
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Subnet:").font(FontId::monospace(11.0)).color(MUTED));
                            ui.add(egui::TextEdit::singleline(&mut self.subnet_input)
                                .font(FontId::monospace(12.0))
                                .desired_width(110.0));
                            ui.label(RichText::new(".1–254").font(FontId::monospace(11.0)).color(MUTED));
                        });
                        ui.add_space(8.0);

                        let scan_running = self.scan_state.lock().unwrap().running;
                        ui.horizontal(|ui| {
                            if ui.add_enabled(!scan_running,
                                styled_btn("SCAN", ACCENT)).clicked() {
                                self.start_scan(ctx);
                            }
                            if scan_running {
                                if ui.add(styled_btn("STOP", DANGER)).clicked() {
                                    self.scan_stop.store(true, Ordering::Relaxed);
                                }
                            }
                        });

                        // プログレス
                        {
                            let s = self.scan_state.lock().unwrap();
                            if scan_running || s.progress > 0.0 {
                                ui.add_space(6.0);
                                ui.add(egui::ProgressBar::new(s.progress)
                                    .desired_width(ui.available_width()));
                            }
                        }

                        ui.add_space(8.0);

                        // ホストリスト
                        let hosts = self.scan_state.lock().unwrap().hosts.clone();
                        if hosts.is_empty() && !scan_running {
                            ui.label(RichText::new("スキャン待機中").font(FontId::monospace(10.0)).color(MUTED));
                        }

                        egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                            for host in &hosts {
                                ui.horizontal(|ui| {
                                    let dot_rect = ui.allocate_exact_size(
                                        Vec2::splat(8.0), egui::Sense::hover()).0;
                                    ui.painter().circle_filled(dot_rect.center(), 4.0, ACCENT3);
                                    ui.label(RichText::new(host.ip.to_string())
                                        .font(FontId::monospace(12.0))
                                        .color(Color32::from_rgb(200, 223, 240)));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(RichText::new(format!("{:.0} ms", host.latency_ms))
                                            .font(FontId::monospace(11.0)).color(MUTED));
                                    });
                                });
                            }
                        });
                    });
            }

            // ── 右：コントロール ──
            {
                let ui = &mut cols[1];
                egui::Frame::none()
                    .fill(PANEL).rounding(8.0)
                    .inner_margin(egui::Margin::same(12.0))
                    .show(ui, |ui| {
                        section_title(ui, "CONTROLS");
                        ui.add_space(8.0);

                        let testing = self.test_state == TestState::Running;
                        let w = ui.available_width();

                        if ui.add_enabled(!testing,
                            egui::Button::new(
                                RichText::new("▶  START TEST").font(FontId::monospace(14.0)).color(ACCENT))
                            .min_size(Vec2::new(w, 42.0))
                            .stroke(Stroke::new(1.5, ACCENT))
                            .fill(Color32::from_rgba_unmultiplied(0, 212, 255, 12))
                        ).clicked() {
                            self.start_test(ctx);
                        }

                        ui.add_space(6.0);

                        if ui.add_enabled(testing,
                            egui::Button::new(
                                RichText::new("■  STOP").font(FontId::monospace(13.0)).color(DANGER))
                            .min_size(Vec2::new(w, 34.0))
                            .stroke(Stroke::new(1.0, if testing { DANGER } else { MUTED }))
                            .fill(Color32::TRANSPARENT)
                        ).clicked() {
                            self.stop_flag.store(true, Ordering::Relaxed);
                            self.test_state = TestState::Idle;
                        }

                        ui.add_space(6.0);

                        if ui.add_enabled(!testing,
                            egui::Button::new(
                                RichText::new("RESET").font(FontId::monospace(11.0)).color(MUTED))
                            .min_size(Vec2::new(w, 28.0))
                            .stroke(Stroke::new(1.0, MUTED))
                            .fill(Color32::TRANSPARENT)
                        ).clicked() {
                            *self.result.lock().unwrap() = SpeedResult::default();
                            self.dl_history.lock().unwrap().clear();
                            self.ul_history.lock().unwrap().clear();
                            self.ping_history.lock().unwrap().clear();
                            self.log_lines.lock().unwrap().push(
                                (format!("[{}] リセット", chrono_now()), MUTED));
                            self.test_state = TestState::Idle;
                        }

                        ui.add_space(12.0);
                        section_title(ui, "RESULT DETAILS");
                        ui.add_space(6.0);

                        let r = self.result.lock().unwrap().clone();
                        info_row(ui, "MIN PING",  r.min_ping.map(|v| format!("{:.1} ms", v)));
                        info_row(ui, "MAX PING",  r.max_ping.map(|v| format!("{:.1} ms", v)));
                        info_row(ui, "JITTER",    r.jitter.map(|v| format!("{:.1} ms", v)));
                        info_row(ui, "SERVER",    Some("speed.cloudflare.com".into()));
                    });
            }
        });
    }

    fn draw_log(&self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(PANEL).rounding(8.0)
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                section_title(ui, "SYSTEM LOG");
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(90.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let lines = self.log_lines.lock().unwrap().clone();
                        for (msg, color) in &lines {
                            ui.label(RichText::new(msg).font(FontId::monospace(10.0)).color(*color));
                        }
                    });
            });
    }
}

// ── ヘルパー関数 ─────────────────────────────

fn gauge_card(ui: &mut egui::Ui, label: &str, val: Option<f64>,
              unit: &str, color: Color32, max: f64, _testing: bool)
{
    egui::Frame::none()
        .fill(PANEL)
        .rounding(8.0)
        .stroke(Stroke::new(1.0, color.linear_multiply(0.3)))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.label(RichText::new(label).font(FontId::monospace(10.0)).color(MUTED));
            ui.add_space(8.0);

            let val_str = val.map(|v| format!("{:.1}", v)).unwrap_or_else(|| "—".into());
            ui.label(RichText::new(&val_str)
                .font(FontId::monospace(36.0))
                .color(if val.is_some() { color } else { MUTED })
                .strong());

            ui.label(RichText::new(unit).font(FontId::monospace(11.0)).color(MUTED));
            ui.add_space(8.0);

            // ミニバー
            let pct = val.map(|v| (v / max).min(1.0) as f32).unwrap_or(0.0);
            let (bar_rect, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), 4.0), egui::Sense::hover());
            ui.painter().rect_filled(bar_rect, 2.0, Color32::from_rgba_unmultiplied(255,255,255,8));
            if pct > 0.0 {
                let filled = egui::Rect::from_min_size(
                    bar_rect.min,
                    Vec2::new(bar_rect.width() * pct, bar_rect.height()),
                );
                ui.painter().rect_filled(filled, 2.0, color);
            }
        });
}

fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(FontId::monospace(10.0)).color(MUTED));
        ui.add(egui::Separator::default().horizontal().spacing(8.0));
    });
}

fn info_row(ui: &mut egui::Ui, key: &str, val: Option<String>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).font(FontId::monospace(10.0)).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(val.unwrap_or_else(|| "—".into()))
                .font(FontId::monospace(11.0))
                .color(Color32::from_rgb(200, 223, 240)));
        });
    });
}

fn legend_dot(ui: &mut egui::Ui, color: Color32, label: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::new(12.0, 4.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 2.0, color);
        ui.label(RichText::new(label).font(FontId::monospace(10.0)).color(MUTED));
    });
}

fn styled_btn(label: &str, color: Color32) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(label).font(FontId::monospace(12.0)).color(color))
        .stroke(Stroke::new(1.0, color))
        .fill(Color32::TRANSPARENT)
        .min_size(Vec2::new(80.0, 30.0))
}

fn chrono_now() -> String {
    // 簡易時刻（std のみ）
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let h = (secs / 3600) % 24;
    let m = (secs / 60)   % 60;
    let s =  secs          % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}
