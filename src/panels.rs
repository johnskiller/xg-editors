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
    /// 中央面板: 三视图分发 (PianoRoll/ChannelNotes/PlayView). 原 update 内 CentralPanel 闭包体.
    pub fn central(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
            ui.horizontal(|ui| {
                ui.heading(self.central_view.label());
                ui.separator();
                // 三态 tab: 播放中可任意切换, 三种视图解读同一份播放事件流 (播放状态独立于视图)
                // 2026-08-12: Piano Roll 已移到底栏 (show_piano toggle); 中央剩 Channel/Play 两视图
                for (mode, label) in [
                    (CentralView::ChannelNotes, "Channel"),
                    (CentralView::PlayView, "Play"),
                ] {
                    if ui.selectable_label(self.central_view == mode, label).clicked() {
                        self.central_view = mode;
                    }
                }
                ui.separator();
                // Zoom/Scroll 只对 Channel Notes 有意义; PlayView 实时播放画面不需要
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
                // Channel View: 行高 zoom slider (16..64 px; 压缩钢琴卷帘时放大行内音高分辨用)
                if self.central_view == CentralView::ChannelNotes {
                    ui.separator();
                    ui.label("RowH");
                    ui.add(
                        egui::Slider::new(&mut self.channel_row_h, 16.0..=64.0)
                            .logarithmic(true)
                            .show_value(false)
                            .custom_formatter(|v, _| format!("{:.0}px", v))
                            .custom_parser(|s| s.trim_end_matches("px").parse::<f64>().ok()),
                    );
                }
            });
            // ===== 视图分发 (Channel/Play 共用播放状态; Piano Roll 已到底栏) =====
            match self.central_view {
                CentralView::PianoRoll => self.render_piano_roll(ui),
                CentralView::ChannelNotes => self.render_channel_notes(ui),
                CentralView::PlayView => self.render_playview(ui),
            }

    }

    /// 钢琴卷帘视图: 已抽到 src/piano_roll.rs (impl XgApp::render_piano_roll)。
    /// 这里只留 tab 分发调用。

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
                // 内容区: 铺深色底, 然后标尺(固定) + ScrollArea(内容滚动)
                // ★ 2016-08-13 John 反馈: "Ch01" 被裁/贴边根因 = 之前 outer.min.x=0.0 强制中央内容
                //   画到屏幕绝对 x=0, 但左侧 Tracks 栏(收起22px/展开160-400px)占住屏幕左缘,
                //   中央面板把所有绘制 clip 到中央区(侧栏右侧) → "Ch" 被裁、"01"贴边、侧栏宽度一变位置就错.
                //   修复: 用 available_rect 真实左缘(egui 自动避开侧栏) → 位置动态跟随侧栏宽度.
                let outer = ui.available_rect_before_wrap();
                let _clip = ui.clip_rect();
                // ★★ 2026-08-13 照抄 piano roll: 背景铺满到 clip_rect 真实面板边缘(覆盖 CentralPanel
                //    默认 8px inner-margin 露出的边距带 = 蓝灰 padding); 内容行/音符仍用 outer.
                //    piano: outer==clip(inner_margin 0), channel: outer 每边比 clip 窄 8px
                //    → 背景必须用 clip, 不能用 outer. (实测 outer.l=30 vs clip.l=22, outer.r=1315 vs clip.r=1323)
                let panel_left = ui.clip_rect().left();
                let panel_right = ui.clip_rect().right();
                let panel_p0 = ui.painter();
                // 深色底 flush 到 outer (available_rect) 全范围
                let bg_rect = egui::Rect::from_min_max(
                    egui::pos2(panel_left, outer.top()),
                    egui::pos2(panel_right, outer.bottom()),
                );
                panel_p0.rect_filled(bg_rect, 0.0, egui::Color32::from_rgb(0x0c, 0x14, 0x1e));

                // ===== 时间映射 (zoom/scroll) 全视图共用一份 =====
                let zoom = self.track_view_zoom.max(0.002);
                let end_tk = if self.smf.is_some() { self.smf_end_tick.max(1) } else { self.total_ticks.max(1) };
                let win_ticks = (end_tk.max(1) as f32 / zoom).round().max(1.0) as u64;
                let win_ticks = win_ticks.max(1);
                let scroll = self.track_view_scroll_ticks;

                // ===== 行头 gutter (channel 名 + 音色 + Mute/Solo + 绿电平) 固定宽 =====
                // 2026-08-13 mute/solo 加入: 158 → 192 (放得下 M/S 按钮)
                let gutter_w = 192.0;
                let notes_left = outer.left() + gutter_w;
                // ★ 右缘 = 中央面板真右缘 (clip_rect right) — 音符/数字画到 params 面板前为止
                let notes_right = panel_right;
                let notes_width = (notes_right - notes_left).max(1.0);

                // ===== 顶部 bar/tick 标尺 (共用 draw_time_ruler) — 固定不滚动 =====
                let ruler_h = 22.0;
                let ruler_top = outer.top();
                let ruler_bot = ruler_top + ruler_h;
                // ★ 标尺背景铺满中央面板真实右缘 (panel_right, 而非 outer.right() —
                //   available_rect 到 params 面板前会被截断) → bar rule 背景不露 3px 空隙 (John 2026-08-13)
                let ruler_rect = egui::Rect::from_min_max(egui::pos2(panel_left, ruler_top), egui::pos2(panel_right, ruler_bot));
                // 画标尺后推进 cursor → ScrollArea 从标尺下方开始, 不再覆盖标尺 (John 2026-08-13: 标尺消失)
                crate::draw_time_ruler(panel_p0, ruler_rect, notes_left, notes_width, win_ticks, scroll, self.ppq.max(1), 4 * self.ppq.max(1));
                ui.allocate_rect(ruler_rect, egui::Sense::hover()); // panel_p0 借用在此结束

                // 内容总高 (滚动区): ch_rows * row_h (row_h 可调 16..64, John 2026-08-13)
                let row_h = self.channel_row_h;
                let ch_rows = if self.smf.is_some() { 16 } else { self.tracks.len().max(1) };
                let total_h = ch_rows as f32 * row_h;

                // ===== 内容区 ScrollArea (纵向滚动, 复用 piano_roll 成熟模式) =====
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .id_salt("channel_view_scroll")
                    .show(ui, |ui| {
                        // 内部坐标系: 视口顶 = min_rect().top()
                        let c0 = ui.min_rect().top();
                        // 预定内容高度 (可滚动)
                        ui.allocate_space(egui::vec2(outer.width(), total_h));

                        // ★ 内容区边界: 统一用 outer(available_rect) 左缘做基准 — 这是屏幕坐标(避开侧栏),
                        //   ScrollArea 内部 painter 用相同坐标绘制, 与 gutter/notes 严格对齐.
                        //   (曾用 ui.min_rect().left() 导致与 notes_left(outer) 基准不一致 → 错位/贴边)
                        // ★★ 2026-08-13 John: bar 数字/音符右扩到 params 面板 (宽窗口 outer.right()
                        //     > 中央面板 clip_rect.right()) → 右缘统一 clamp 到 clip_rect.right() (面板真右缘),
                        //     否则 note 线段/bar 数字超出其深色背景 (John: "bar 9 数字超出 rule 背景").
                        //     行背景(c_right)铺满到面板真右缘 panel_right (John: "行背景不到 params 左缘"),
                        let c_left = outer.left();
                        let c_right = panel_right; // 行背景铺到面板真右缘 (clip_rect), 不留蓝灰边距带
                        let content_bottom = c0 + total_h;

                        // 通道行背景 + 行头 + 音符
                    // SMF: 16 行; 否则 tracks 行数
                    for i in 0..ch_rows {
                        let y0 = c0 + i as f32 * row_h;
                        let row_rect = egui::Rect::from_min_max(
                            egui::pos2(c_left, y0),
                            egui::pos2(c_right, (y0 + row_h).min(content_bottom)),
                        );
                    // 该行显示数据 (SMF: 16 ch, 音色/电平来自 live_*; 否则 tracks)
                    // Mute/Solo 静音的通道电平强制 0 (John 2026-08-13: mute 后电平表归零)
                    let row_level_raw = if self.smf.is_some() {
                        self.live_levels.get(i).copied().unwrap_or(0.0)
                    } else {
                        match self.tracks.get(i) {
                            Some(t) => t.level,
                            None => 0.0,
                        }
                    };
                    let row_level: f32 = if self.channel_is_effectively_muted(i) { 0.0 } else { row_level_raw };
                    let (row_name, row_voice): (String, String) = if self.smf.is_some() {
                        (
                            format!("Ch{:02}", i + 1),
                            self.voice_name_for_channel(i),  // 单源化: 从 parts 派生
                        )
                    } else {
                        match self.tracks.get(i) {
                            Some(t) => (t.name.clone(), t.voice.clone()),
                            None => (format!("Track {}", i + 1), String::new()),
                        }
                    };
                    // 行背景色: 明显交错的深浅蓝灰, 每个channel一条可辨(John 要求)
                    let base: (u8, u8, u8) = if i % 2 == 0 { (0x12, 0x1e, 0x2e) } else { (0x1f, 0x2f, 0x45) };
                    // 行背景 + 行头文字 (ChNN/音色名) 用独立 painter scope — p 借用需在 ui.put(custom widget) 前结束
                    {
                        let p = ui.painter();
                        p.rect_filled(row_rect, 0.0, egui::Color32::from_rgb(base.0, base.1, base.2));
                        // 行分隔线
                        p.line_segment(
                            [egui::pos2(c_left, y0), egui::pos2(c_right, y0)],
                            egui::Stroke::new(1.0, egui::Color32::from_gray(30)),
                        );
                        // 行头 gutter: ChNN + 音色名 + Mute/Solo 按钮 + 绿电平
                        {
                            let gx = c_left + 6.0;
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
                            if voice.chars().count() > 8 { voice.truncate(8); voice.push_str(".."); }
                            p.text(
                                egui::pos2(gx + 34.0, cy_row),
                                egui::Align2::LEFT_CENTER,
                                &voice,
                                egui::FontId::monospace(10.0),
                                egui::Color32::from_gray(150),
                            );
                        }
                    } // end row-bg/text painter scope (p borrow ends before ui.put)

                    // ===== Mute / Solo 自定义控件 (ChNN 名 与 电平表 之间, John 2026-08-13 定案)
                    // 用 egui custom widget (ms_button.rs) — 非散装手绘 (John 建议 custom 控件)
                    {
                        let btn_sz = 18.0f32.min(row_h - 2.0); // 行高小时收缩
                        let ms_x = c_left + 100.0;
                        let cy_row = row_rect.center().y;
                        let m_rect = egui::Rect::from_center_size(
                            egui::pos2(ms_x + btn_sz / 2.0, cy_row), egui::vec2(btn_sz, btn_sz));
                        let s_rect = egui::Rect::from_center_size(
                            egui::pos2(ms_x + btn_sz + 4.0 + btn_sz / 2.0, cy_row), egui::vec2(btn_sz, btn_sz));
                        // 点击: 立即生效; mute/solo 触发时对本该静音的通道清音 (DAW 行为)
                        let m_resp = ui.put(m_rect, crate::ms_button::MSButton::new(crate::ms_button::MSKind::Mute, self.channel_mutes[i]).size(btn_sz));
                        let s_resp = ui.put(s_rect, crate::ms_button::MSButton::new(crate::ms_button::MSKind::Solo, self.channel_solos[i]).size(btn_sz));
                        if m_resp.clicked() {
                            self.channel_mutes[i] = !self.channel_mutes[i];
                            self.sync_sound_off_for_muted_channels();
                        }
                        if s_resp.clicked() {
                            self.channel_solos[i] = !self.channel_solos[i];
                            self.sync_sound_off_for_muted_channels();
                        }
                    }

                    // 绿电平条 + % (mute 后 row_level=0 → 不画绿条, 视觉"这条是死的")
                    {
                        let p = ui.painter();
                        let cy_row = row_rect.center().y;
                        let lvx = c_left + 158.0;
                        let lvw = 26.0;
                        p.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(lvx, cy_row - 4.0), egui::vec2(lvw, 8.0)),
                            2.0, egui::Color32::from_gray(60),
                        );
                        if row_level > 0.001 {
                            let lw = (row_level * lvw).max(2.0);
                            p.rect_filled(
                                egui::Rect::from_min_size(egui::pos2(lvx, cy_row - 4.0), egui::vec2(lw, 8.0)),
                                2.0, egui::Color32::from_rgb(0x2e, 0xcc, 0x40),
                            );
                        }
                        // (电平条后不显示百分比数字 — John: 变动太快看不清)
                        // gutter 与音符区之间的分隔竖线
                        p.line_segment(
                            [egui::pos2(c_left + gutter_w, y0), egui::pos2(c_left + gutter_w, (y0 + row_h).min(content_bottom))],
                            egui::Stroke::new(1.0, egui::Color32::from_gray(45)),
                        );
                    }
                    // 音符区 (gutter 右缘起, 同一时间映射)
                    let inner = egui::Rect::from_min_max(
                        egui::pos2(notes_left + 4.0, y0 + 4.0),
                        egui::pos2(notes_right - 4.0, (y0 + row_h).min(content_bottom) - 4.0),
                    );
                    let ch = i + 1; // channel 1..16
                    // ★ 压缩 piano roll: 每个 note = 1px 水平线 (John 2026-08-13)
                    //   x = 时间, 长度 = 时长, 颜色 = 力度, y(行内) = 音高 0-127 映射到行高(高音在上)
                    //   → 同 tick 和弦因音高不同垂直分离, 重叠可见 (旧"点"渲染无法区分和弦)
                    let pitch_low = self.channel_view_pitch_low;
                    let pitch_high = self.channel_view_pitch_high;
                    let p_range = (pitch_high - pitch_low).max(1u8) as f32;
                    let p_y = |pitch: u8| -> f32 {
                        // 高音在上: inner.bottom() - 相对位置*inner.height()
                        inner.bottom() - ((pitch as f32 - pitch_low as f32) / p_range * inner.height()).clamp(0.0, inner.height())
                    };
                    let p = ui.painter();
                    if self.smf.is_none() {
                        let def_notes = &self.tracks[i].notes;
                        for n in def_notes {
                            if n.start_tick < scroll || n.start_tick > scroll + win_ticks { continue; }
                            let nx = inner.left() + (n.start_tick - scroll) as f32 / win_ticks as f32 * inner.width();
                            let nw = (n.dur_ticks as f32 / win_ticks as f32 * inner.width()).max(2.0);
                            let ny = p_y(n.pitch).clamp(inner.top(), inner.bottom() - 1.0);
                            let (gr, rr, bb) = self.channel_note_color(i, n.velocity);
                            p.line_segment(
                                [egui::pos2(nx, ny), egui::pos2(nx + nw, ny)],
                                egui::Stroke::new(1.0, egui::Color32::from_rgb(gr, rr, bb)),
                            );
                        }
                    } else {
                        // SMF 视图: 每个 NoteOn 用真实 dur_ticks 画线 (John 2026-08-13: 与 piano roll 同数据源,
                        // 时长必须一致; 旧实现画 3px 短线是 bug)
                        let t_notes: &[smf::SmfNote] = self.smf_views.get(i).map(|v| v.notes.as_slice()).unwrap_or(&[]);
                        for n in t_notes {
                            if n.start_tick < scroll || n.start_tick > scroll + win_ticks { continue; }
                            let nx = inner.left() + (n.start_tick - scroll) as f32 / win_ticks as f32 * inner.width();
                            let nw = (n.dur_ticks as f32 / win_ticks as f32 * inner.width()).max(2.0);
                            let ny = p_y(n.pitch).clamp(inner.top(), inner.bottom() - 1.0);
                            let (gr, rr, bb) = self.channel_note_color(i, n.vel);
                            p.line_segment(
                                [egui::pos2(nx, ny), egui::pos2(nx + nw, ny)],
                                egui::Stroke::new(1.0, egui::Color32::from_rgb(gr, rr, bb)),
                            );
                        }
                    }
                    // 行尾标注 channel 号
                    let _ = ch;
                }
                // playhead 竖线 (Channel 视图, 跟随 zoom+scroll; 收敛到内容区高度)
                let p = ui.painter(); // 独立 painter (行循环结束后, 画 playhead + bar 竖线)
                let ph_x = if self.playhead_tick >= scroll && self.playhead_tick <= scroll + win_ticks {
                    notes_left + (self.playhead_tick - scroll) as f32 / win_ticks as f32 * notes_width
                } else {
                    notes_left - 4.0 // 视口外
                };
                if ph_x > notes_left && ph_x < notes_right {
                    p.vline(ph_x, egui::Rangef::new(c0, content_bottom), egui::Stroke::new(2.0, egui::Color32::from_rgb(0xff, 0xd0, 0x40)));
                }
                // bar 起始竖线贯穿全部行(淡, 辅助对齐; 止于内容区底, 不再溢出到白底)
                let bar_ticks = 4 * self.ppq.max(1);
                let last_tick = scroll + win_ticks;
                if bar_ticks > 0 {
                    let mut bt0 = (scroll / bar_ticks) * bar_ticks;
                    while bt0 <= last_tick {
                        if bt0 >= scroll {
                            let bxg = notes_left + (bt0 - scroll) as f32 / win_ticks as f32 * notes_width;
                            p.line_segment(
                                [egui::pos2(bxg, c0), egui::pos2(bxg, content_bottom)],
                                egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0x50, 0x70, 0x80, 60)),
                            );
                        }
                        bt0 += bar_ticks;
                    }
                }
                    });  // end ScrollArea (channel_view_scroll)
    }  // end render_channel_notes
}
