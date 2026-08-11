// 面板布局: 顶栏 top_bar + 中央 central (从 lib.rs update() 闭包拆出, Step 3).
// 注释保留原 update 内的中文说明. 零参数依赖, 仅 &mut self + ui.

use crate::XgApp;
use eframe::egui;
use crate::midi_topology;
use crate::midi_wasm;
use crate::CentralView;
use crate::CHANNEL_ROW_H;
use crate::smf;

impl XgApp {
    /// 顶部工具栏 (标题/Tempo/传输按钮/读取按钮等). 原 update 内 TopPanel 闭包体.
    pub fn top_bar(&mut self, ui: &mut egui::Ui) {
            ui.horizontal(|ui| {
                ui.heading(format!("XG Editor (v{})", self.app_version));
                ui.separator();
                ui.label("Tempo");
                ui.add(egui::DragValue::new(&mut self.tempo_bpm).speed(0.1).suffix(" bpm").range(30.0..=240.0));
                ui.label("TSig 4/4");
                ui.separator();
                // 传输控制: Play / Pause / Stop
                if self.playing {
                    if ui.button("[Pause]").clicked() {
                        // Pause: 停表 + 清掉设备上挂音 (用户 2026-08-09: Pause 有长音悬挂问题)
                        let bb = self.playhead_bar_beat();
                        self.playing = false;
                        self.send_all_sound_off();
                        self.log_status(format!("Pause @ {}:{}:{}", bb.0, bb.1, bb.2));
                    }
                } else {
                    if ui.button("[Play]").clicked() {
                        // Play: 从当前位置续播 (Stop 已把 playhead 归 0 → 从头).
                        // Pause 后不应重头 → play_resume 不清 playhead/不重建事件表
                        self.play_resume();
                        let bb = self.playhead_bar_beat();
                        self.log_status(format!("Play @ {}:{}:{}", bb.0, bb.1, bb.2));
                    }
                }
                if ui.button("[Stop]").clicked() {
                    self.playing = false;
                    self.playhead_tick = 0;
                    self.event_cursor = 0;
                    self.event_cursor_origin = 0;
                    self.play_events.clear();
                    self.live_levels = [0.0; 16]; // 电平表归零 (停止播放)
                    self.live_vel_peaks = [0.0; 16];
                    self.raw_vel_peaks = [0.0; 16];
                    self.active_notes.iter_mut().for_each(|m| m.clear());
                    // 清掉设备上所有挂音 (否则 Stop 后仍在响)
                    self.send_all_sound_off();
                    self.log_status("Stop");
                }
                // playhead 位置显示 (bar:beat:tick)
                let bb = self.playhead_bar_beat();
                ui.label(format!("{:>3}:{:02}.{:02}", bb.0, bb.1, bb.2));
                ui.separator();
                // Open MIDI: wasm 文件选择 + 显示已加载名/结果
                #[cfg(target_arch = "wasm32")]
                {
                    if ui.button("[Open MIDI]").clicked() {
                        midi_wasm::open_midi_file_dialog();
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if ui.button("[Open MIDI] (drag .mid here)").clicked() {
                        // native: 提示用户拖放
                    }
                }
                ui.separator();
                ui.separator();
                ui.separator();
                // MIDI 设备下拉(判据 7)
                ui.label("MIDI:");
                egui::ComboBox::from_id_salt("midi_devs")
                    .selected_text(
                        self.selected_midi
                            .map(|i| self.midi_devices[i].clone())
                            .unwrap_or("Select...".into()),
                    )
                    .show_ui(ui, |ui| {
                        for (i, d) in self.midi_devices.iter().enumerate() {
                            if ui.selectable_label(self.selected_midi == Some(i), d).clicked() {
                                self.selected_midi = Some(i);
                                // 不再乐观地标 connected —— 需真实打开端口验证 (John 2026-08-09: 谎报)
                                self.midi_connected = false;
                                #[cfg(target_arch = "wasm32")]
                                {
                                    // 清旧缓存 + 异步验证端口真能打开
                                    midi_wasm::reset_output_cache();
                                    let dev = d.clone();
                                    let cell = self.midi_verify_cell.get_or_insert_with(|| {
                                        std::rc::Rc::new(std::cell::RefCell::new(None))
                                    }).clone();
                                    *cell.borrow_mut() = None;
                                    let cell2 = cell.clone();
                                    wasm_bindgen_futures::spawn_local(async move {
                                        let r = midi_wasm::verify_output(&dev).await;
                                        *cell2.borrow_mut() = Some(r);
                                    });
                                }
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    let _ = d; // native 无校验 (非 wasm 开发路径)
                                    self.midi_connected = true;
                                }
                            }
                        }
                        ui.separator();
                        // 冒烟验证: 显示真·Web MIDI 探测结果(wasm: 真实设备; native: 不支持)
                        match &self.midi_probe_result {
                            Some(Ok((_ins, outs))) if !outs.is_empty() => {
                                ui.colored_label(egui::Color32::from_rgb(0x2e, 0xcc, 0x40),
                                    format!("Web MIDI OK: {} output device(s)", outs.len()));
                            }
                            Some(Ok(_)) => {
                                ui.colored_label(egui::Color32::from_rgb(0xff, 0xcc, 0x44),
                                    "Web MIDI OK (0 devices)");
                            }
                            Some(Err(e)) => {
                                ui.colored_label(egui::Color32::from_rgb(0xff, 0x66, 0x66),
                                    format!("Web MIDI err: {}", e));
                            }
                            None => {
                                ui.label("[Web MIDI probing...]");
                            }
                        }
                        // 拓扑读out: PortA/PortB 分配 + 已路由 part 数 (数据结构可见化)
                        ui.separator();
                        let a = self.midi_topology.output_for_role(midi_topology::MidiRole::PortA);
                        let b = self.midi_topology.output_for_role(midi_topology::MidiRole::PortB);
                        ui.label(format!(
                            "Topo: A={} B={} | parts routed: {}",
                            a.as_deref().unwrap_or("-"),
                            b.as_deref().unwrap_or("-"),
                            (0..32u8).filter(|&p| self.midi_topology.part_is_routed(p)).count(),
                        ));
                        // 端口清单 (in/out 标签)
                        let desc: Vec<String> = self.midi_topology.ports.iter()
                            .map(|p| format!("{}{}{}", p.name, if p.is_input { " in" } else { "" }, if p.is_output { " out" } else { "" }))
                            .collect();
                        if !desc.is_empty() {
                            ui.label(desc.join(" | "));
                        }
                        // System Dump 捕获: 开关 + 全量查看/复制 (分析真实 bulk 格式)
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.toggle_value(&mut self.sysex_capture, "Capture SysEx").changed() {
                                if self.sysex_capture {
                                    self.sysex_capture_log.clear();
                                    self.sysex_capture_count = 0;
                                    self.log_status("SysEx capture ON — 请从 MU90 发送 System Dump");
                                } else {
                                    self.log_status(format!("SysEx capture OFF (captured {})", self.sysex_capture_count));
                                }
                            }
                            if self.sysex_capture {
                                ui.label(format!("{} msg", self.sysex_capture_count));
                                if ui.button("Clear").clicked() {
                                    self.sysex_capture_count = 0;
                                    self.sysex_capture_log.clear();
                                }
                            }
                        });
                        // 查看捕获 (折叠区)
                        if self.sysex_capture && !self.sysex_capture_log.is_empty() {
                            egui::CollapsingHeader::new(format!("Captured ({}) — click to view", self.sysex_capture_log.len()))
                                .default_open(false)
                                .show(ui, |ui| {
                                    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                        for (src, hex, n) in &self.sysex_capture_log {
                                            ui.monospace(format!("[{src}] RX#{n} {hex}"));
                                        }
                                    });
                                });
                        }
                        // 分析按钮: 解析捕获为 地址→值 聚合表 (供复制/定位 msb/lsb/pc 地址)
                        if self.sysex_capture && !self.sysex_capture_log.is_empty() {
                            ui.horizontal(|ui| {
                                if ui.button("Analyze dump").clicked() {
                                    self.analyze_sysex_capture();
                                }
                                if !self.sysex_analysis.is_empty() {
                                    ui.label(format!("{} unique addrs", self.sysex_analysis.len()));
                                }
                                // 下载地址表 (聚合后, 200 行内)
                                if !self.sysex_analysis.is_empty() {
                                    if ui.button("Save table").clicked() {
                                        match crate::download_text("mu90_addr_table.txt", &self.build_analysis_text()) {
                                            Ok(()) => self.log_status("Address table saved (mu90_addr_table.txt)"),
                                            Err(e) => self.log_status(format!("Save failed: {e}")),
                                        }
                                    }
                                }
                                // 下载原始捕获 (全量, 用于完整抓包)
                                if ui.button("Save raw").clicked() {
                                    let mut s = String::new();
                                    for (src, hex, n) in &self.sysex_capture_log {
                                        s.push_str(&format!("{src}\t{n}\t{hex}\n"));
                                    }
                                    match crate::download_text("mu90_sysex_capture.txt", &s) {
                                        Ok(()) => self.log_status(format!("Saved {} raw msgs", self.sysex_capture_log.len())),
                                        Err(e) => self.log_status(format!("Save raw failed: {e}")),
                                    }
                                }
                            });
                            if !self.sysex_analysis.is_empty() {
                                egui::CollapsingHeader::new("Address table (click to view)")
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                                            for (addr, val, cnt) in &self.sysex_analysis {
                                                let hh = (addr >> 14) & 0x7F;
                                                let mm = (addr >> 7) & 0x7F;
                                                let ll = addr & 0x7F;
                                                ui.monospace(format!("{hh:02X} {mm:02X} {ll:02X} = {val} (x{cnt})"));
                                            }
                                        });
                                    });
                            }
                        }
                    });
                // ============ Port B (MU90 Port B = parts 17-32) ============
                ui.horizontal(|ui| {
                    ui.label("PortB:");
                    egui::ComboBox::from_id_salt("midi_devs_b")
                        .selected_text(
                            self.selected_midi_b
                                .map(|i| self.midi_devices[i].clone())
                                .unwrap_or("Select...".into()),
                        )
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for (i, d) in self.midi_devices.iter().enumerate() {
                                if ui.selectable_label(self.selected_midi_b == Some(i), d).clicked() {
                                    self.selected_midi_b = Some(i);
                                }
                            }
                            ui.separator();
                            if ui.selectable_label(self.selected_midi_b.is_none(), "None").clicked() {
                                self.selected_midi_b = None;
                            }
                        });
                    // 镜像到 Port B: 32 part 全响应
                    ui.checkbox(&mut self.mirror_to_b, "mirror→B (32pt)");
                });
                ui.separator();
                if self.midi_connected {
                    ui.colored_label(egui::Color32::from_rgb(0x2e, 0xcc, 0x40), "[OK] Connected");
                } else {
                    ui.colored_label(egui::Color32::GRAY, "[--] Not connected");
                }
                // 发送测试: 选中设备后发 Program Change + Note(验证"编辑→硬件响应"链路)
                if self.midi_connected && self.selected_midi.is_some() {
                    if ui.button("Send Test (PC+Note)").clicked() {
                        let dev = self.midi_devices[self.selected_midi.unwrap()].clone();
                        self.midi_send_status = None;
                        #[cfg(target_arch = "wasm32")]
                        {
                            let cell = std::rc::Rc::new(std::cell::RefCell::new(None::<Result<(), String>>));
                            let c2 = cell.clone();
                            let dev2 = dev.clone();
                            wasm_bindgen_futures::spawn_local(async move {
                                // 完整 XG 音色选择: CC0(MSB)=0, CC32(LSB)=0, PC=40(Violin)
                                // 长音验证: Note On 立即, Note Off 延迟 1500ms → 小提琴能完整起音
                                let t0 = web_sys::window()
                                    .map(|w| w.performance())
                                    .and_then(|p| p.map(|p| p.now()))
                                    .unwrap_or(0.0);
                                let mut last = midi_wasm::send_to(&dev2, &[0xB0, 0x00, 0x00]).await; // CC0
                                if last.is_ok() {
                                    last = midi_wasm::send_to(&dev2, &[0xB0, 0x20, 0x00]).await; // CC32
                                }
                                if last.is_ok() {
                                    last = midi_wasm::send_to(&dev2, &[0xC0, 0x28]).await; // PC Violin
                                }
                                if last.is_ok() {
                                    last = midi_wasm::send_at(&dev2, &[0x90, 60, 100], None).await; // Note On (立即)
                                }
                                // Note Off 在 1500ms 后
                                if last.is_ok() {
                                    last = midi_wasm::send_at(&dev2, &[0x80, 60, 0], Some(t0 + 1500.0)).await;
                                }
                                *c2.borrow_mut() = Some(last);
                            });
                            self.midi_send_ui_cell = Some(cell);
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            self.midi_send_status = Some("native: no Web MIDI".into());
                        }
                        let _ = dev;
                    }
                    // 轮询发送结果 cell(收进 self.midi_send_status)
                    #[cfg(target_arch = "wasm32")]
                    if let Some(cell) = &self.midi_send_ui_cell {
                        if let Some(r) = cell.borrow_mut().take() {
                            self.midi_send_status = Some(match r {
                                Ok(_) => "sent ok (PC+Note)".into(),
                                Err(e) => format!("send err: {e}"),
                            });
                        }
                    }
                }
                if let Some(s) = &self.midi_send_status {
                    ui.label(s);
                }
                // ======== 双向通信: 读 part 音色 ========
                ui.separator();
                // 绑定输入: 自动绑同名 input (UX16 通常是 in/out 同名), 也支持手动
                let bound = midi_wasm::bound_input_names();
                if bound.is_empty() {
                    if ui.button("Bind Input").clicked() {
                        if let Some(dev) = self.selected_midi.map(|i| self.midi_devices[i].clone()) {
                            #[cfg(target_arch = "wasm32")]
                            {
                                let cell = self.midi_bind_cell.get_or_insert_with(|| {
                                    std::rc::Rc::new(std::cell::RefCell::new(None))
                                }).clone();
                                *cell.borrow_mut() = None;
                                let c2 = cell.clone();
                                let d2 = dev.clone();
                                wasm_bindgen_futures::spawn_local(async move {
                                    // 1) 先试同名 (输出 "UM-ONE (UM-ONE) [Port1]" → 输入可能同名)
                                    let mut r = midi_wasm::bind_input(&d2).await;
                                    if r.is_err() {
                                        // 2) 兜底: 名截到空格/( 前 (如 "USB-MIDI (FX16) [Port1]" → "USB-MIDI")
                                        let base: String = d2.split('[').next()
                                            .unwrap_or(&d2).split('(').next()
                                            .unwrap_or(&d2).trim().to_string();
                                        if !base.is_empty() {
                                            r = midi_wasm::bind_input(&base).await;
                                        }
                                    }
                                    if r.is_err() {
                                        // 3) 兜底: 探测 inputs 绑第一个
                                        if let Ok((ins, _)) = midi_wasm::probe_pair().await {
                                            if let Some(first) = ins.first().cloned() {
                                                r = midi_wasm::bind_input(&first).await;
                                            }
                                        }
                                    }
                                    *c2.borrow_mut() = Some(r);
                                });
                            }
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                self.midi_send_status = Some("native: no Web MIDI".into());
                            }
                            let _ = dev;
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    if let Some(cell) = &self.midi_bind_cell {
                        if let Some(r) = cell.borrow_mut().take() {
                            match r {
                                Ok(_) => self.midi_send_status = Some("input bound".into()),
                                Err(e) => self.midi_send_status = Some(format!("bind err: {e}")),
                            }
                        }
                    }
                } else {
                    ui.label(format!("In: {}", bound.join(", ")));
                }
                // 读当前 part 音色 (单 part)
                if !bound.is_empty() && self.selected_midi.is_some() {
                    let dev = self.midi_devices[self.selected_midi.unwrap()].clone();
                    ui.horizontal(|ui| {
                        if ui.button(format!("Read Part {}", self.cur_part)).clicked() {
                            let part = (self.cur_part.saturating_sub(1)).min(31) as u8;
                            self.read_batch_next = None; // 单读
                            self.read_batch_msb_only = false; // 单读读全 (msb+lsb+pc)
                            self.start_read_part(part);
                        }
                        if ui.button("Read All 32").clicked() {
                            self.read_parts = Default::default();
                            self.read_part_cursor = Some(0);
                            self.log_status(format!("reading all 32 parts ... (msb+lsb+pc, {}ms gap)", self.read_req_gap_ms));
                            // 从 part0 开始, 全读 (msb+lsb+pc); 每完成一个读下一个
                            self.read_batch_next = Some(0);
                            self.read_batch_msb_only = false; // 全读
                            self.read_part_cursor = Some(0);
                            self.start_read_part(0);
                        }
                        // Bulk Read All 32: 2n dump request 握手式连发, 绕过 3n 的 160ms 冷却 (2026-08-09 定案)
                        if ui.button("Bulk Read 32").clicked() {
                            self.start_bulk_read();
                        }
                        // 请求间隔可调 (实验: 0=回包立即发下一条, 测程序/硬件真实下限; 160 已知可用)
                        ui.separator();
                        ui.label("gap:");
                        let mut gap = self.read_req_gap_ms;
                        ui.add(egui::DragValue::new(&mut gap).range(0..=2000).speed(10));
                        self.read_req_gap_ms = gap;
                        // 面板 dump 引导已移除 (2026-08-09 John: title 空间有限) — 不再占顶栏
                    });
                    if let Some(infl) = &self.read_request_inflight {
                        ui.label(format!("reading {infl} ..."));
                    }
                }
                // 读结果 (read partN: ...) 与原始 rx 已移到底部 status bar(status_log 已含)
                // → 顶栏只保留操作按钮, 不再显示瞬态调试文本 (John 2026-08-09 要求清理)
                // 32-part 表格已移至右栏 (Rev: Hall 行后) — 见 update 中 params 右栏; 这里不再渲染
            });

    }

    /// 中央面板: 三视图分发 (PianoRoll/ChannelNotes/PlayView). 原 update 内 CentralPanel 闭包体.
    pub fn central(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
            ui.horizontal(|ui| {
                ui.heading(self.central_view.label());
                ui.separator();
                // 三态 tab: 播放中可任意切换, 三种视图解读同一份播放事件流 (播放状态独立于视图)
                for (mode, label) in [
                    (CentralView::PianoRoll, "Piano Roll"),
                    (CentralView::ChannelNotes, "Channel"),
                    (CentralView::PlayView, "Play"),
                ] {
                    if ui.selectable_label(self.central_view == mode, label).clicked() {
                        self.central_view = mode;
                    }
                }
                ui.separator();
                // Zoom/Scroll 只对时间轴类视图 (Piano Roll / Channel Notes) 有意义; PlayView 实时播放画面不需要
                if self.central_view != CentralView::PlayView {
                ui.label("Zoom");
                // log 缩放: 0.02x(缩小50).. 200x(放大200), 平滑过渡不敏感; 显示具体倍率
                // 1x = 全区正好充满 view (John 语义定案 2026-08-09); 2x=看半曲, 4x=1/4曲
                // zoom 语义: 1x = 全区正好充满 view; 0.02x = 缩小50x(看50倍宽度) .. 200x(放大200倍)
                ui.add(
                    egui::Slider::new(&mut self.track_view_zoom, 0.02..=200.0)
                        .logarithmic(true)
                        .show_value(true) // 显示具体倍率
                        .custom_formatter(|v, _| format!("{v:.2}x"))
                        .custom_parser(|s| s.parse::<f64>().ok()),
                );
                ui.separator();
                let t_end = if self.smf.is_some() { self.smf_end_tick.max(1) } else { self.total_ticks.max(1) };
                let zoom_s = self.track_view_zoom.max(0.002);
                let win = (t_end.max(1) as f32 / zoom_s).round().max(1.0) as u64; // 1x=fit 全区
                let win = win.max(1);
                ui.label("Scroll");
                let max_scroll = t_end.saturating_sub(win) as f64;
                let mut scf = self.track_view_scroll_ticks as f64;
                ui.add(egui::Slider::new(&mut scf, 0.0..=max_scroll).step_by((win.max(1) / 20).max(1) as f64).custom_formatter(|v, _| format!("{}t", v as i64)));
                self.track_view_scroll_ticks = scf.max(0.0) as u64;
                } // end Zoom/Scroll (非 PlayView)
            });
            // ===== 视图分发 (三 tab 共用播放状态) =====
            // ===== 视图分发 (三 tab 共用播放状态) =====
            match self.central_view {
                CentralView::PianoRoll => self.render_piano_roll(ui),
                CentralView::ChannelNotes => self.render_channel_notes(ui),
                CentralView::PlayView => self.render_playview(ui),
            }

    }

    /// 钢琴卷帘视图: 半音横带 + 绿条音符 + playhead 竖线.
    /// 从 central() 拆出 (PianoRoll 分支). 使用 ctx(ui.ctx) 加载背景贴图.
    fn render_piano_roll(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
            let rect = ui.available_rect_before_wrap();
            ui.allocate_rect(rect, egui::Sense::hover());
            ui.label("note placeholders (green)");
            let p = ui.painter();
            // ===== 背景贴图测试(John 旧项目卡死点)=====
            // 正确姿势: 用 painter.image() 直画, 不参与布局流(不占空间、不推乱控件)
            // 先画 = 在底层(egui 按 add 顺序 z 叠放, 后画在上层)
            // 这里故意在网格/音符之前画, 网格音符会盖在背景上, 布局不受影响
            let bg_tex = ctx.load_texture(
                "bg_texture",
                egui::ColorImage::from_rgba_unmultiplied(
                    [self.bg_side, 128],
                    &self.bg_pixels,
                ),
                egui::TextureOptions::NEAREST,
            );
            let uv_full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            let bg_rect = rect; // 背景铺满内容区
            p.image(bg_tex.id(), bg_rect, uv_full, egui::Color32::WHITE);
            // ===== 钢琴卷帘: 每一行(半音横带)不同背景 =====
            // 行高统一 = note_row_h(与网格横线、绿条行距一致), 每个绿条正好落在自己的色带行中
            let note_row_h = 70.0;
            let n_rows = ((rect.height() / note_row_h) as usize).max(1);
            for r in 0..n_rows {
                let y0 = rect.top() + r as f32 * note_row_h;
                let row_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.left(), y0),
                    egui::pos2(rect.right(), (y0 + note_row_h).min(rect.bottom())),
                );
                let base: (u8, u8, u8) = if r % 2 == 0 { (0x14, 0x22, 0x30) } else { (0x1c, 0x30, 0x44) };
                let lift = ((r / 2) % 5) as u8; // 每两设定亮度小台阶, 突显"每行不同"
                let c = egui::Color32::from_rgb(
                    (base.0 as u16 + lift as u16 * 3).min(255) as u8,
                    (base.1 as u16 + lift as u16 * 3).min(255) as u8,
                    (base.2 as u16 + lift as u16 * 3).min(255) as u8,
                );
                p.rect_filled(row_rect, 0.0, c);
            }
            let step = 24.0;
            let mut y = rect.top();
            while y < rect.bottom() {
                p.line_segment(
                    [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                    egui::Stroke::new(1.0, egui::Color32::from_gray(30)),
                );
                y += note_row_h;
            }
            let mut x = rect.left() + 60.0;
            while x < rect.right() {
                p.vline(x, rect.y_range(), egui::Stroke::new(1.0, egui::Color32::from_gray(25)));
                x += step * 4.0;
            }
            // 实时音符渲染: 遍历所有轨的真实音符数据 (pitch 高 → 靠上)
            // 行高 70px, 每行半音 → 音高跨 ~12 行
            let n_notes_rows = ((rect.height() - 20.0) / note_row_h) as usize;
            for t in &self.tracks {
                for n in &t.notes {
                    // 音高→行: pitch 0..127; 显示窗 pitch 范围 [pan_low, pan_low+rows)
                    let rows = n_notes_rows.max(1) as i32;
                    let low = 24; // C1
                    let high = low + rows;
                    if n.pitch < low as u8 || n.pitch >= high as u8 {
                        continue;
                    }
                    let row = (high - 1 - n.pitch as i32) as f32; // 高音→小行号(靠上)
                    let cy = rect.top() + row * note_row_h + note_row_h * 0.5;
                    // 时间→x: total 768 tick → 内容宽
                    let x0 = rect.left() + 60.0;
                    let w = (rect.width() - 60.0).max(1.0);
                    let nx = x0 + (n.start_tick as f32 / self.total_ticks.max(1) as f32) * w;
                    let nw = (n.dur_ticks as f32 / self.total_ticks.max(1) as f32) * w;
                    let ch = n.channel as usize;
                    let note_col = egui::Color32::from_rgb(
                        0x2e,
                        0xcc - (ch % 3) as u8 * 20,
                        0x40 + (ch % 2) as u8 * 30,
                    );
                    p.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(nx, cy - 6.0),
                            egui::vec2(nw.max(3.0), 12.0),
                        ),
                        2.0,
                        note_col,
                    );
                }
            }
            // playhead 竖线 (播放时跟随)
            if self.playing || self.playhead_tick > 0 {
                let x0 = rect.left() + 60.0;
                let w = (rect.width() - 60.0).max(1.0);
                let px = x0 + (self.playhead_tick as f32 / self.total_ticks.max(1) as f32) * w;
                p.vline(px, rect.y_range(), egui::Stroke::new(2.0, egui::Color32::from_rgb(0xff, 0xd7, 0x00)));
            }

    }

    /// Channel 音符指示视图: 每行 = 1 MIDI channel (行头 gutter: ChNN+音色+绿电平).
    /// 从 central() 拆出 (ChannelNotes 分支). zoom/scroll 时间映射全视图共用.
    fn render_channel_notes(&mut self, ui: &mut egui::Ui) {
                ui.label("Channel Notes (each row = 1 MIDI channel)");
                // 状态写入 DOM (#xg_state) 供 headless 截图验证 (用户不可见, 纯调试工具)
                #[cfg(target_arch = "wasm32")]
                if self.smf_is_dirty {
                    let n_all: usize = if self.smf.is_some() {
                        self.smf_views.iter().map(|v| v.notes.len()).sum()
                    } else {
                        self.tracks.iter().map(|t| t.notes.len()).sum()
                    };
                    let ch_active = if self.smf.is_some() {
                        self.smf_views.iter().filter(|v| !v.notes.is_empty()).count()
                    } else {
                        self.tracks.iter().filter(|t| !t.notes.is_empty()).count()
                    };
                    // 调试: 把状态写进 DOM (dump-dom 可读), 避免截图 GPU 回读不可靠
                    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                        if let Some(el) = doc.get_element_by_id("xg_state") {
                            let _ = el.set_text_content(Some(&format!(
                                "smf={} notes={} chActive={} zoom={:.4} scroll={} endTick={} load={}",
                                self.smf.is_some(), n_all, ch_active,
                                self.track_view_zoom, self.track_view_scroll_ticks, self.smf_end_tick,
                                self.smf_load_result,
                            )));
                        }
                    }
                    self.smf_is_dirty = false;
                }
                // 内容区
                let rect = ui.available_rect_before_wrap();
                ui.allocate_rect(rect, egui::Sense::hover());
                let p = ui.painter();

                // ===== 时间映射 (zoom/scroll) 全视图共用一份 =====
                let zoom = self.track_view_zoom.max(0.002);
                let end_tk = if self.smf.is_some() { self.smf_end_tick.max(1) } else { self.total_ticks.max(1) };
                let win_ticks = (end_tk.max(1) as f32 / zoom).round().max(1.0) as u64;
                let win_ticks = win_ticks.max(1);
                let scroll = self.track_view_scroll_ticks;

                // ===== 行头 gutter (channel 名 + 音色 + 绿电平) 固定宽 =====
                // 音色/电平从左侧 track 栏移入(John: 根治对齐) — 与音符区在同一个 rect 内,
                // 天然居中, 无跨栏 Y 错位可能.
                let gutter_w = 158.0;
                let notes_left = rect.left() + gutter_w;
                let notes_right = rect.right();
                let notes_width = (notes_right - notes_left).max(1.0);

                // ===== 顶部 bar/tick 标尺 =====
                let ruler_h = 22.0;
                let ruler_top = rect.top();
                let ruler_bot = ruler_top + ruler_h;
                p.rect_filled(
                    egui::Rect::from_min_max(egui::pos2(rect.left(), ruler_top), egui::pos2(rect.right(), ruler_bot)),
                    0.0, egui::Color32::from_rgb(0x0c, 0x14, 0x1e),
                );
                // 行网格顶 = 标尺之下(自洽, 不再依赖左栏绝对 Y)
                let grid_top = ruler_bot;
                // bar 刻线: 每小节 tick = ppq * 4 (4/4)
                let bar_ticks = 4 * self.ppq.max(1);
                let last_tick = scroll + win_ticks;
                if bar_ticks > 0 {
                    let first_bar = scroll / bar_ticks;
                    let last_bar = last_tick / bar_ticks + 1;
                    let mut bar_no = first_bar;
                    while bar_no <= last_bar {
                        let bt = bar_no * bar_ticks;
                        if bt <= last_tick {
                            let bx = notes_left + (bt - scroll) as f32 / win_ticks as f32 * notes_width;
                            // bar 起始竖线(只在标尺内亮, 全局淡)
                            p.line_segment(
                                [egui::pos2(bx, ruler_top + 8.0), egui::pos2(bx, ruler_bot)],
                                egui::Stroke::new(1.0, egui::Color32::from_rgb(0x66, 0x88, 0x99)),
                            );
                            // bar 号 (1-based)
                            p.text(
                                egui::pos2(bx + 3.0, ruler_top + 1.0),
                                egui::Align2::LEFT_TOP,
                                format!("{}", bar_no + 1),
                                egui::FontId::monospace(10.0),
                                egui::Color32::from_gray(210),
                            );
                            // beat 子刻线: 每小节内 4 拍 (bar 起点除外), 比 bar 淡, 只在标尺内
                            let beat_t = self.ppq.max(1);
                            let mut b = 1; // beat index 1..3 (0 = bar 起点已画)
                            while b < 4 {
                                let btk = bt + b * beat_t;
                                if btk <= last_tick {
                                    let bbx = notes_left + (btk - scroll) as f32 / win_ticks as f32 * notes_width;
                                    p.line_segment(
                                        [egui::pos2(bbx, ruler_top + 12.0), egui::pos2(bbx, ruler_bot)],
                                        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x3a, 0x4a, 0x58)),
                                    );
                                }
                                b += 1;
                            }
                        }
                        bar_no += 1;
                    }
                }

                // 通道行背景 + 行头 + 音符
                // SMF 加载后 16 行 (每行 = 1 MIDI channel); 否则默认 tracks rows
                let ch_rows = if self.smf.is_some() { 16 } else { self.tracks.len() };
                for i in 0..ch_rows {
                    let y0 = grid_top + i as f32 * CHANNEL_ROW_H;
                    let row_rect = egui::Rect::from_min_max(
                        egui::pos2(rect.left(), y0),
                        egui::pos2(rect.right(), (y0 + CHANNEL_ROW_H).min(rect.bottom())),
                    );
                    // 该行显示数据 (SMF: 16 ch, 音色/电平来自 live_*; 否则 tracks)
                    let (row_name, row_voice, row_level): (String, String, f32) = if self.smf.is_some() {
                        (
                            format!("Ch{:02}", i + 1),
                            self.live_voice_names.get(i).cloned().unwrap_or_default(),
                            self.live_levels.get(i).copied().unwrap_or(0.0),
                        )
                    } else {
                        match self.tracks.get(i) {
                            Some(t) => (t.name.clone(), t.voice.clone(), t.level),
                            None => (format!("Track {}", i + 1), String::new(), 0.0),
                        }
                    };
                    // 行背景色: 明显交错的深浅蓝灰, 每个channel一条可辨(John 要求)
                    let base: (u8, u8, u8) = if i % 2 == 0 { (0x12, 0x1e, 0x2e) } else { (0x1f, 0x2f, 0x45) };
                    p.rect_filled(row_rect, 0.0, egui::Color32::from_rgb(base.0, base.1, base.2));
                    // 行分隔线
                    p.line_segment(
                        [egui::pos2(rect.left(), y0), egui::pos2(rect.right(), y0)],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(30)),
                    );
                    // 行头 gutter: ChNN + 音色名 + 绿电平 + %
                    {
                        let gx = rect.left() + 6.0;
                        let cy_row = row_rect.center().y;
                        // Ch 号 (1..16)
                        p.text(
                            egui::pos2(gx, cy_row),
                            egui::Align2::LEFT_CENTER,
                            &row_name,
                            egui::FontId::monospace(12.0),
                            egui::Color32::from_gray(230),
                        );
                        // 音色名(截断, 压缩到 gutter 内不溢出)
                        let mut voice = row_voice;
                        if voice.chars().count() > 10 { voice.truncate(10); voice.push_str(".."); }
                        p.text(
                            egui::pos2(gx + 30.0, cy_row),
                            egui::Align2::LEFT_CENTER,
                            &voice,
                            egui::FontId::monospace(10.0),
                            egui::Color32::from_gray(150),
                        );
                        // 绿电平条 + %
                        let lvx = rect.left() + 98.0;
                        let lvw = 28.0;
                        p.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(lvx, cy_row - 4.0), egui::vec2(lvw, 8.0)),
                            2.0, egui::Color32::from_gray(60),
                        );
                        let lw = (row_level * lvw).max(2.0);
                        p.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(lvx, cy_row - 4.0), egui::vec2(lw, 8.0)),
                            2.0, egui::Color32::from_rgb(0x2e, 0xcc, 0x40),
                        );
                        // (电平条后不显示百分比数字 — John: 变动太快看不清)
                        // gutter 与音符区之间的分隔竖线
                        p.line_segment(
                            [egui::pos2(rect.left() + gutter_w, y0), egui::pos2(rect.left() + gutter_w, (y0 + CHANNEL_ROW_H).min(rect.bottom()))],
                            egui::Stroke::new(1.0, egui::Color32::from_gray(45)),
                        );
                    }
                    // 音符区 (gutter 右缘起, 同一时间映射)
                    let inner = egui::Rect::from_min_max(
                        egui::pos2(notes_left + 4.0, y0 + 4.0),
                        egui::pos2(notes_right - 4.0, (y0 + CHANNEL_ROW_H).min(rect.bottom()) - 4.0),
                    );
                    let ch = i + 1; // channel 1..16
                    // 低 zoom 音符过密(<2px)时合并成"密度带": 同一窗口内音符并成一段
                    let merge_gap = ((win_ticks as f32 / inner.width()) * 2.0) as u64; // 2px 对应 tick
                    let merge_gap = merge_gap.max(1);
                    // 无 SMF 时走 self.tracks 渲染
                    if self.smf.is_none() {
                        let def_notes = &self.tracks[i].notes;
                        for n in def_notes {
                            if n.start_tick < scroll || n.start_tick > scroll + win_ticks { continue; }
                            let nx = inner.left() + (n.start_tick - scroll) as f32 / win_ticks as f32 * inner.width();
                            let nw = (n.dur_ticks as f32 / win_ticks as f32 * inner.width()).max(2.0);
                            let ny = row_rect.center().y - CHANNEL_ROW_H * 0.25;
                            let nh = CHANNEL_ROW_H * 0.5;
                            let (gr, rr, bb) = if self.playing && (self.playhead_tick >= n.start_tick % (win_ticks + scroll).max(1) && self.playhead_tick <= (n.start_tick % (win_ticks + scroll).max(1)) + n.dur_ticks) {
                                (0xff, 0xd0, 0x30)
                            } else {
                                (0x2e, 0xcc - (i % 3) as u8 * 20, 0x40)
                            };
                            let note_col = egui::Color32::from_rgb(gr, rr, bb);
                            p.rect_filled(
                                egui::Rect::from_min_size(egui::pos2(nx, ny), egui::vec2(nw, nh)),
                                2.0,
                                note_col,
                            );
                        }
                    } else {
                        // SMF 视图: 每个 MIDI 事件(NoteOn)= 一个小方块点 (John 拍板:
                        // track view 看"哪里有事件", 不画长度/不要 note off; 长度在 piano roll 看)
                        let t_notes: &[smf::SmfNote] = self.smf_views.get(i).map(|v| v.notes.as_slice()).unwrap_or(&[]);
                        // 每通道一个事件 = 一个点; 只画在当前可视窗口内的
                        let dot_w = 6.0f32;
                        let dot_h = (CHANNEL_ROW_H * 0.55).clamp(6.0, 12.0);
                        let ny = row_rect.center().y - dot_h * 0.5;
                        for n in t_notes {
                            if n.start_tick < scroll || n.start_tick > scroll + win_ticks { continue; }
                            let nx = inner.left() + (n.start_tick - scroll) as f32 / win_ticks as f32 * inner.width();
                            // 同 tick 多音(和弦)时横向加 1px 位移, 避免完全重叠成一个点
                            let _ = nx;
                            let anchored = nx - dot_w * 0.5;
                            let (gr, rr, bb) = self.channel_note_color(i, n.vel);
                            let note_col = egui::Color32::from_rgb(gr, rr, bb);
                            p.rect_filled(
                                egui::Rect::from_min_size(egui::pos2(anchored, ny), egui::vec2(dot_w, dot_h)),
                                1.5,
                                note_col,
                            );
                        }
                    }
                    // 行尾标注 channel 号
                    let _ = ch;
                }
                // playhead 竖线 (Channel 视图, 跟随 zoom+scroll)
                let ph_x = if self.playhead_tick >= scroll && self.playhead_tick <= scroll + win_ticks {
                    notes_left + (self.playhead_tick - scroll) as f32 / win_ticks as f32 * notes_width
                } else {
                    notes_left - 4.0 // 视口外
                };
                if ph_x > notes_left && ph_x < notes_right {
                    p.vline(ph_x, egui::Rangef::new(grid_top, rect.bottom()), egui::Stroke::new(2.0, egui::Color32::from_rgb(0xff, 0xd0, 0x40)));
                }
                // bar 起始竖线贯穿全部行(淡, 辅助对齐)
                if bar_ticks > 0 {
                    let mut bt0 = (scroll / bar_ticks) * bar_ticks;
                    while bt0 <= last_tick {
                        if bt0 >= scroll {
                            let bxg = notes_left + (bt0 - scroll) as f32 / win_ticks as f32 * notes_width;
                            p.line_segment(
                                [egui::pos2(bxg, grid_top), egui::pos2(bxg, rect.bottom())],
                                egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0x50, 0x70, 0x80, 60)),
                            );
                        }
                        bt0 += bar_ticks;
                    }
                }

    }
}
