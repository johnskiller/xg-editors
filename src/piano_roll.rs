//! 钢琴卷帘视图 (Piano Roll)
//!
//! 2026-08-12: 从 panels.rs 抽出为独立文件, 为后续完善 PianoRoll 做准备。
//! 当前是"空壳"实现: 半音横带背景 + 通道色音符条 + playhead 竖线。
//! 后续迭代计划 (见 reference / TASKS.md):
//!   - 真实钢琴键左侧栏 (C2..C7 键盘)
//!   - 缩放/滚动联动轨道
//!   - 音符选中/拖拽编辑
//!   - MIDI 音符增删改 + 回写 SMF
//!
//! 依赖: XgApp 字段 bg_pixels/bg_side(背景) + tracks(音符) + playhead_tick/total_ticks(定位)。
//! 数据源单向: 只读播放状态, 不改播放状态 (与 AGENTS.md 数据流单向约定一致)。

use crate::XgApp;
use eframe::egui;

impl XgApp {
    /// 钢琴卷帘视图: 半音横带 + 彩色音符条 + playhead 竖线。
    /// 从 panels.rs central() 拆出后独立成文件, 便于后续完善。
    pub(crate) fn render_piano_roll(&mut self, ui: &mut egui::Ui) {
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
}
