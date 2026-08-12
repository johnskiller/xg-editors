//! 钢琴卷帘视图 (Piano Roll)
//!
//! 2026-08-12: 从 panels.rs 抽出为独立文件, 并完善为真实 piano roll:
//!   - 左侧黑白钢琴键: MIDI 0-127 全范围 (用户定案: 不用计算范围), 固定行高, 随内容区垂直滚动
//!   - 顶部 bar/beat 标尺 (ppq 换算, bar 刻度 + beat 细分) — 固定不随垂直滚动
//!   - 只显示一个 channel 的音符 (顶部工具栏 Channel 选择器; 高音在上、低音在下)
//!   - 音符体现时长: 宽度 = dur_ticks / 总长 (note on→off 间隔, 数据源已带 dur_ticks)
//!
//! 数据源 (用户 2026-08-12 定案: 只显示单个 channel):
//!   - SMF 已加载: self.smf_views[ch-1].notes (真实音符)
//!   - 未加载: self.tracks[ch-1].notes (默认演示 pattern)
//! 只读播放状态, 不改播放状态 (与 AGENTS.md 数据流单向约定一致)。
//!
//! 布局: 标尺 (allocate 顶部) → ScrollArea(纵向): 左琴键 + 时间轴。
//! 注意 ScrollArea 闭包内坐标是内部坐标系 (视口顶 = ui.min_rect().top()),
//! 不能直接用外部 content_rect 的坐标 (否则画到错误位置)。

use crate::XgApp;
use eframe::egui;

/// 左侧琴键宽度 (px)
const KEY_W: f32 = 52.0;
/// 顶部 bar/beat 标尺高度 (px)
const RULER_H: f32 = 22.0;
/// 每半音行高 (px)
const ROW_H: f32 = 12.0;
/// MIDI 音高范围 (用户定案: 固定 0-127)
const MIDI_LOW: i32 = 0;
const MIDI_HIGH: i32 = 128;
/// 白键半音集合 (标准钢琴: C D E F G A B 为白键)
const WHITE_PC: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];

/// MIDI 音符名 (C-1..G9); 0-127 全范围
fn midi_name(p: i32) -> String {
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let oct = p / 12 - 1;
    format!("{}{oct}", names[(p.rem_euclid(12)) as usize])
}

impl XgApp {
    /// 当前 channel 的音符列表 (SMF 优先, 无则演示 tracks): (start_tick, dur_ticks, pitch)
    fn pr_notes(&self, ch: u8) -> Vec<(u64, u64, u8)> {
        let idx = (ch.saturating_sub(1)) as usize;
        // smf_views 在 Default 时是 16 个空 view (notes 为空, 见 lib.rs 1614),
        // 所以先检查 notes 非空才用它, 否则回退到默认 pattern tracks
        if let Some(view) = self.smf_views.get(idx) {
            if !view.notes.is_empty() {
                return view
                    .notes
                    .iter()
                    .map(|n| (n.start_tick, n.dur_ticks, n.pitch))
                    .collect();
            }
        }
        if let Some(t) = self.tracks.get(idx) {
            return t
                .notes
                .iter()
                .map(|n| (n.start_tick, n.dur_ticks, n.pitch))
                .collect();
        }
        Vec::new()
    }

    /// 钢琴卷帘: 左琴键(0-127) + 顶 bar/beat + 单 channel 音符 + playhead
    pub(crate) fn render_piano_roll(&mut self, ui: &mut egui::Ui) {
        let ch = self.cur_pr_channel; // 1..16
        let outer = ui.available_rect_before_wrap();
        // 注意: 不要用 ui.allocate_rect(outer) 抢占空间 — 会让后续 ScrollArea 视口塌缩为 0 (内容全不可见)

        let t_end = if self.smf.is_some() {
            self.smf_end_tick.max(1)
        } else {
            self.total_ticks.max(1)
        };
        let ppq = self.ppq.max(1);
        let beats_per_bar = 4u64; // 默认 4/4 (复杂拍号后续)

        // ===== 时间轴换算 (与 Channel 视图 panels.rs 499-715 完全一致) =====
        // zoom 放大 → win_ticks 变小 → bar/音符横向变宽 → bar/beat 自动重画
        let zoom = self.pr_zoom.max(0.002);
        let win_ticks = (t_end.max(1) as f32 / zoom).round().max(1.0) as u64;
        let scroll = self.pr_scroll_ticks;
        let last_tick_win = scroll + win_ticks;
        let bar_ticks = beats_per_bar * ppq; // 每小节 tick (4/4)

        // ===== 顶部 bar/beat 标尺 (共用函数 draw_time_ruler, 与 Channel 视图一致) =====
        let (ruler_rect, _) =
            ui.allocate_exact_size(egui::vec2(outer.width(), RULER_H), egui::Sense::hover());
        let ruler_p = ui.painter();
        let ruler_time_rect = egui::Rect::from_min_max(
            egui::pos2(ruler_rect.left() + KEY_W, ruler_rect.top()),
            ruler_rect.max,
        );
        crate::draw_time_ruler(
            ruler_p,
            ruler_rect,
            ruler_time_rect.left(),
            ruler_time_rect.width(),
            win_ticks,
            scroll,
            ppq,
            bar_ticks,
        );

        // ===== 内容区 (ScrollArea 纵向) : 左琴键 + 时间轴 =====
        let total_h = (MIDI_HIGH - MIDI_LOW) as f32 * ROW_H; // 128 行
        // 仅首帧设定 initial 垂直滚动到音符中位区, 之后完全交还用户 (egui vertical_scroll_offset
        // 每帧都 apply → 若一直 Some 会把用户滚动弹回, 无法滚到 0-127 全部琴键; 用户 2026-08-12)
        let need_init = !self.pr_scrolled_once;
        let mut init_off = 0.0;
        if need_init {
            let notes = self.pr_notes(ch);
            let mut pitches: Vec<i32> = notes.iter().map(|(_, _, p)| *p as i32).collect();
            pitches.sort_unstable();
            let median_pitch = pitches.get(pitches.len() / 2).copied().unwrap_or(60);
            // 目标行 (像素) = (127 - pitch)*ROW_H; 偏移到让该行居中
            let target_y = (MIDI_HIGH - 1 - median_pitch) as f32 * ROW_H;
            init_off = (target_y - 60.0).max(0.0);
            self.pr_scrolled_once = true;
        }
        let mut scroll_area = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .id_salt("piano_roll_scroll");
        // 仅首帧传 initial offset; 后续帧不传 → 用户滚动不被弹回, 可滚遍 0-127 全部琴键
        if need_init && init_off > 0.0 {
            scroll_area = scroll_area.vertical_scroll_offset(init_off);
        }
        scroll_area.show(ui, |ui| {
                // 内部坐标系: 视口顶 = ui.min_rect().top()
                let c0 = ui.min_rect().top();
                // 预留内容高度 (可滚动)
                ui.allocate_space(egui::vec2(outer.width(), total_h));
                let p = ui.painter();

                // 内容左右范围 (ScrollArea 内部, 相对 c0)
                let c_left = outer.left();
                let c_right = outer.right();

                // 背景半音行 (白键行亮, 黑键行暗)
                for p_ in MIDI_LOW..MIDI_HIGH {
                    let y0 = c0 + (MIDI_HIGH - 1 - p_) as f32 * ROW_H;
                    let row = egui::Rect::from_min_max(
                        egui::pos2(c_left, y0),
                        egui::pos2(c_right, y0 + ROW_H),
                    );
                    let is_white = WHITE_PC.contains(&(p_.rem_euclid(12)));
                    let base = if is_white {
                        (0x16, 0x22, 0x34)
                    } else {
                        (0x0f, 0x17, 0x22)
                    };
                    p.rect_filled(row, 0.0, egui::Color32::from_rgb(base.0, base.1, base.2));
                }

                // ===== 左侧黑白琴键 (0-127, 固定宽) =====
                let key_rect = egui::Rect::from_min_max(
                    egui::pos2(c_left, c0),
                    egui::pos2(c_left + KEY_W, c0 + total_h),
                );
                for p_ in MIDI_LOW..MIDI_HIGH {
                    let y0 = c0 + (MIDI_HIGH - 1 - p_) as f32 * ROW_H;
                    let row = egui::Rect::from_min_max(
                        egui::pos2(key_rect.left(), y0),
                        egui::pos2(key_rect.right(), y0 + ROW_H),
                    );
                    let is_white = WHITE_PC.contains(&(p_.rem_euclid(12)));
                    if is_white {
                        p.rect_filled(row, 1.0, egui::Color32::from_rgb(0xe6, 0xe6, 0xe6));
                    } else {
                        p.rect_filled(row, 1.0, egui::Color32::from_rgb(0x1a, 0x1a, 0x20));
                    }
                    p.rect_stroke(row, 1.0, egui::Stroke::new(1.0, egui::Color32::from_gray(90)));
                    // C 标注
                    if p_.rem_euclid(12) == 0 {
                        let col = if is_white {
                            egui::Color32::from_gray(70)
                        } else {
                            egui::Color32::from_gray(170)
                        };
                        p.text(
                            egui::pos2(row.left() + 3.0, row.center().y),
                            egui::Align2::LEFT_CENTER,
                            midi_name(p_),
                            egui::FontId::proportional(9.0),
                            col,
                        );
                    }
                }

                // ===== 时间轴区 (琴键右侧) =====
                let time_rect = egui::Rect::from_min_max(
                    egui::pos2(key_rect.right(), c0),
                    egui::pos2(c_right, c0 + total_h),
                );
                // 行分隔细线
                for i in 0..=(MIDI_HIGH - MIDI_LOW) {
                    let y = c0 + i as f32 * ROW_H;
                    p.hline(time_rect.x_range(), y, egui::Stroke::new(1.0, egui::Color32::from_gray(28)));
                }
                // ===== bar 竖线 (贯穿时间轴内容区, 随 zoom/scroll 重画) =====
                let time_w = (time_rect.width()).max(1.0);
                if bar_ticks > 0 {
                    let mut bt0 = (scroll / bar_ticks) * bar_ticks;
                    while bt0 <= last_tick_win {
                        if bt0 >= scroll {
                            let bxg = time_rect.left() + (bt0 - scroll) as f32 / win_ticks.max(1) as f32 * time_w;
                            p.vline(
                                bxg,
                                time_rect.y_range(),
                                egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0x50, 0x70, 0x80, 60)),
                            );
                        }
                        bt0 += bar_ticks;
                    }
                }

                // ===== 单 channel 音符 (宽度 = 时长 dur/t_end) =====
                let notes = self.pr_notes(ch);
                for (start, dur, pitch) in &notes {
                    let p_ = *pitch as i32;
                    if p_ < MIDI_LOW || p_ >= MIDI_HIGH {
                        continue;
                    }
                    // 只在窗口 [scroll, scroll+win_ticks] 内显示 (与 Channel 一致)
                    if *start + *dur < scroll || *start > last_tick_win {
                        continue;
                    }
                    let row_idx = (MIDI_HIGH - 1 - p_) as usize;
                    let vy = c0 + row_idx as f32 * ROW_H;
                    let sx = time_rect.left() + (start.checked_sub(scroll).unwrap_or(0)) as f32 / win_ticks.max(1) as f32 * time_w;
                    let sw = (*dur as f32 / win_ticks.max(1) as f32 * time_w).max(2.0);
                    let note_rect = egui::Rect::from_min_max(
                        egui::pos2(sx, vy),
                        egui::pos2(sx + sw, vy + ROW_H),
                    );
                    let ci = (ch - 1) as usize;
                    let (r, g, b) = self.channel_note_color(ci, 100);
                    p.rect_filled(note_rect, 2.0, egui::Color32::from_rgb(r, g, b));
                }

                // ===== playhead =====
                if self.playing || self.playhead_tick > 0 {
                    if self.playhead_tick >= scroll && self.playhead_tick <= last_tick_win {
                        let px = time_rect.left() + (self.playhead_tick - scroll) as f32 / win_ticks.max(1) as f32 * time_w;
                        p.vline(px, time_rect.y_range(), egui::Stroke::new(2.0, egui::Color32::from_rgb(0xff, 0xd7, 0x00)));
                    }
                }
            });
    }
}
