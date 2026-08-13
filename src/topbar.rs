// TopBar 面板: menu + 标题 + tempo/count + transport + 连接状态 (TopBar 美化 2026-08-13 拆分).
// 独立文件自 panels.rs 抽出; 调试/设置控件收进 ☰ Menu 的 MIDI Setup / Tools 子菜单.

use crate::XgApp;
use crate::midi_topology;
use crate::midi_wasm;
use eframe::egui;

impl XgApp {
    /// 顶部工具栏 (组件化, TopBar 美化 2026-08-13).
    /// 结构: [☰ Menu] | 标题版本 | Tempo 拍号 | 播放 count | Transport | 连接状态 .
    /// 调试/设置控件收进 ☰ 菜单 (File / MIDI Setup / Tools); 顶栏只留高频项.
    pub fn top_bar(&mut self, ui: &mut egui::Ui) {
        // 顶栏背景 = #1f2f45 (Frame 已填, 见 lib.rs TopBottomPanel.frame — John 拍板同 Channel View 奇数通道色)
        // 底部一条深分隔线 (与中央区分界, 比底色略亮)
        let r = ui.max_rect();
        {
            let p = ui.painter();
            p.line_segment(
                [egui::pos2(r.min.x, r.max.y - 0.5), egui::pos2(r.max.x, r.max.y - 0.5)],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2b, 0x40, 0x5e)),
            );
        }
        // 顶栏深底 → 正文用浅色
        let mut style: egui::Style = (*ui.style()).as_ref().clone();
        style.visuals.widgets.inactive.fg_stroke.color = egui::Color32::from_rgb(0xd5, 0xdc, 0xe6);
        style.visuals.widgets.hovered.fg_stroke.color = egui::Color32::from_rgb(0xef, 0xf3, 0xf8);
        style.visuals.widgets.active.fg_stroke.color = egui::Color32::from_rgb(0xff, 0xff, 0xff);
        // DragValue 输入框: 深底配更深一档的输入框 + 浅色文字 (保持深色 bar 统一)
        style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(0x16, 0x23, 0x35);
        style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x20, 0x31, 0x48);
        style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(0x24, 0x38, 0x52);
        ui.set_style(style);
        ui.horizontal(|ui| {
            ui.menu_button("\u{2630}", |ui| {
                // ── File ──
                ui.menu_button("File", |ui| {
                    #[cfg(target_arch = "wasm32")]
                    if ui.button("Open MIDI...").clicked() {
                        midi_wasm::open_midi_file_dialog();
                        ui.close_menu();
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        ui.label("Open MIDI (native: drag .mid here)");
                    }
                });
                // ── MIDI Setup ──
                ui.menu_button("MIDI Setup", |ui| {
                    self.midi_setup_menu(ui);
                });
                // ── Tools (调试控件) ──
                ui.menu_button("Tools", |ui| {
                    self.tools_menu(ui);
                });
            });
            ui.separator();

            // ============ 标题 + 版本 ============
            // egui .strong() 只调色不改变粗细 (默认字体 Ubuntu-Light 无 bold 字形)
            // → 双次绘制 fake-bold (1px 偏移), 颜色显式亮白 (深底 #1f2f45)
            {
                let text_galley = ui
                    .painter()
                    .layout_no_wrap(
                        format!("XG Editor v{}", self.app_version),
                        egui::FontId::proportional(17.0),
                        egui::Color32::from_rgb(0xff, 0xff, 0xff),
                    );
                let pos = ui.cursor().min;
                let painter = ui.painter();
                // 第 1 遍 白字
                painter.galley(pos, text_galley.clone(), egui::Color32::from_rgb(0xff, 0xff, 0xff));
                // 第 2 遍 右移 1px → fake bold (粗体)
                painter.galley(
                    egui::pos2(pos.x + 1.0, pos.y),
                    text_galley.clone(),
                    egui::Color32::from_rgb(0xff, 0xff, 0xff),
                );
                // 占位: 让 horizontal 布局把内容宽度算进去
                ui.allocate_space(egui::vec2(text_galley.size().x + 1.0, text_galley.size().y));
            }
            ui.separator();

            // ============ Tempo + 拍号 (保留, 组件化, 字体稍大) ============
            ui.label(
                egui::RichText::new("Tempo")
                    .size(13.5)
                    .color(egui::Color32::from_rgb(0xd5, 0xdc, 0xe6)), // 深底强制浅色
            );
            ui.add(
                egui::DragValue::new(&mut self.tempo_bpm)
                    .speed(0.1)
                    .suffix(" bpm")
                    .range(30.0..=240.0),
            );
            ui.label(
                egui::RichText::new("4/4")
                    .size(13.5)
                    .color(egui::Color32::from_rgb(0xd5, 0xdc, 0xe6)), // 深底强制浅色
            );
            ui.separator();

            // ============ 播放 count (bar:beat:tick) 组件化 + 字体放大 + 开发者选色 ============
            let bb = self.playhead_bar_beat();
            ui.label(
                egui::RichText::new(format!("{:>3}:{:02}.{:03}", bb.0, bb.1, bb.2))
                    .size(18.0)
                    .monospace()
                    .color(egui::Color32::from_rgb(0xff, 0xc6, 0x4d)), // 亮金 (深底 #1f2f45 可读)
            );
            ui.separator();

            // ============ Transport (Play/Pause/Stop/Record) ============
            let playing = self.playing;
            if playing {
                if ui.add(crate::transport::TransportButton::new(crate::transport::Transport::Pause).active(true).size(24.0)).clicked() {
                    // Pause: 停表 + 清掉设备上挂音 (用户 2026-08-09: Pause 有长音悬挂问题)
                    let bb = self.playhead_bar_beat();
                    self.playing = false;
                    self.send_all_sound_off();
                    self.log_status(format!("Pause @ {}:{}:{}", bb.0, bb.1, bb.2));
                }
            } else {
                if ui.add(crate::transport::TransportButton::new(crate::transport::Transport::Play).size(24.0)).clicked() {
                    // Play: 从当前位置续播 (Stop 已把 playhead 归 0 → 从头).
                    // Pause 后不应重头 → play_resume 不清 playhead/不重建事件表
                    self.play_resume();
                    let bb = self.playhead_bar_beat();
                    self.log_status(format!("Play @ {}:{}:{}", bb.0, bb.1, bb.2));
                }
            }
            if ui.add(crate::transport::TransportButton::new(crate::transport::Transport::Stop).size(24.0)).clicked() {
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
            // Record: 功能预留, 仅 armed 视觉切换 (John 2026-08-13)
            if ui.add(crate::transport::TransportButton::new(crate::transport::Transport::Record).active(self.rec_armed).size(24.0)).clicked() {
                self.rec_armed = !self.rec_armed;
                self.log_status(if self.rec_armed { "Record armed (功能预留)" } else { "Record disarmed" });
            }
            ui.separator();

            // ============ 连接状态 (精简色点 + 文本, 保留顶栏) ============
            // ■ (U+25A0)/○ (U+25CB) 在 emoji-icon-font 有覆盖 (● U+25CF 缺失会 tofu)
            // 深底 #1f2f45 → 亮色文字
            if self.midi_connected {
                ui.colored_label(egui::Color32::from_rgb(0x4c, 0xd9, 0x64), "\u{25A0} Connected");
            } else {
                ui.colored_label(egui::Color32::from_rgb(0x9a, 0xa6, 0xb5), "\u{25CB} Not connected");
            }
            // 已加载 SMF 名 (若有, 尾部常驻显示)
            if !self.smf_name.is_empty() {
                ui.separator();
                ui.label(
                    egui::RichText::new(&self.smf_name)
                        .size(13.0)
                        .color(egui::Color32::from_rgb(0x8f, 0x9c, 0xad)), // 深底弱化灰蓝
                );
            }
        });
    }

    /// ☰ Menu ▸ MIDI Setup: 输出设备 A/B + mirror→B + 拓扑 + 端口清单.
    /// (从原 top_bar 顶栏横向区拆出, 2026-08-13 TopBar 美化)
    fn midi_setup_menu(&mut self, ui: &mut egui::Ui) {
        // 输出设备 A
        ui.label("Output A:");
        egui::ComboBox::from_id_salt("midi_devs")
            .selected_text(
                self.selected_midi
                    .map(|i| self.midi_devices[i].clone())
                    .unwrap_or("Select...".into()),
            )
            .width(180.0)
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
            });
        ui.separator();

        // Port B (MU90 Port B = parts 17-32)
        ui.label("Port B:");
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
    }

    /// ☰ Menu ▸ Tools: SysEx 捕获/分析 + Read Part/All/Bulk + Send Test + Bind Input + gap 调参.
    /// (从原 top_bar 顶栏横向区拆出, 2026-08-13 TopBar 美化)
    fn tools_menu(&mut self, ui: &mut egui::Ui) {
        // ── SysEx 捕获 ──
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
        ui.separator();

        // ── 发送测试: 选中设备后发 Program Change + Note ──
        if self.midi_connected && self.selected_midi.is_some() {
            ui.horizontal(|ui| {
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
                if let Some(s) = &self.midi_send_status {
                    ui.label(s);
                }
            });
            ui.separator();
        }

        // ── 双向通信: 读 part 音色 ──
        // 绑定输入: 自动绑同名 input (UX16 通常是 in/out 同名), 也支持手动
        let bound = midi_wasm::bound_input_names();
        if bound.is_empty() {
            ui.horizontal(|ui| {
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
                                    // 2) 兜底: 名截到空格/( 前
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
            });
        } else {
            ui.label(format!("In: {}", bound.join(", ")));
        }
        // 读当前 part 音色 (单 part)
        if !bound.is_empty() && self.selected_midi.is_some() {
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
            });
            if let Some(infl) = &self.read_request_inflight {
                ui.label(format!("reading {infl} ..."));
            }
        }
        // 读结果 (read partN: ...) 与原始 rx 已移到底部 status bar — 顶栏只保留操作按钮
    }
}
