//! Transport 控制组件 (TopBar 一行, 组件化).
//!
//! egui custom widget: `TransportButton` 实现 `egui::Widget`，
//! 复用 M/S 按钮(ms_button.rs)的 custom widget 模式.
//! 图标**手绘几何** (painter 画三角/竖杠/方块/圆), 不依赖字体字形 →
//! 四个按钮同尺寸、无字体字号差异 (John 2026-08-13: ⏺ 字形偏大/按钮尺寸不一).

use eframe::egui::{Align2, Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, Widget};

/// Transport 按钮类型
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Play,    // ▶  (播放中显示 Pause)
    Pause,   // ⏸  (播放中)
    Stop,    // ⏹
    Record,  // ⏺  (功能预留, 仅视觉 armed)
}

/// Transport 按钮的 custom widget.
pub struct TransportButton {
    kind: Transport,
    /// Record: armed 态 (红点亮); 其余: 是否激活 (Pause 激活=播放中)
    active: bool,
    size: f32,
}

impl TransportButton {
    pub fn new(kind: Transport) -> Self {
        Self { kind, active: false, size: 22.0 }
    }
    /// 是否激活 (Pause 激活=播放中; Record 激活=armed)
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
    pub fn size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }

    /// 图标主色
    fn glyph_color(&self) -> Color32 {
        match self.kind {
            Transport::Play | Transport::Pause | Transport::Stop => {
                if self.active {
                    Color32::from_rgb(0x1e, 0x8a, 0x3e) // 播放中 → 深绿 (浅色主题可读)
                } else {
                    Color32::from_rgb(0x33, 0x33, 0x33) // 常态深灰黑
                }
            }
            Transport::Record => {
                if self.active {
                    Color32::from_rgb(0xcc, 0x22, 0x22) // armed → 深红
                } else {
                    Color32::from_rgb(0x33, 0x33, 0x33) // 常态深灰黑
                }
            }
        }
    }

    /// 在 rect 内绘制几何图标 (所有类型同尺寸, 居中于 r).
    /// 图标占 rect 的 ~60%, 保证四按钮视觉一致.
    fn draw_icon(&self, painter: &eframe::egui::Painter, r: Rect, color: Color32) {
        let c = r.center();
        let s = self.size;
        let stroke = Stroke::new((s / 10.0).max(1.5), color);
        let fill = color;
        match self.kind {
            Transport::Play => {
                // 实心三角: 顶点偏右
                let sz = s * 0.30;
                let pts = vec![
                    Pos2::new(c.x - sz * 0.5, c.y - sz),
                    Pos2::new(c.x - sz * 0.5, c.y + sz),
                    Pos2::new(c.x + sz, c.y),
                ];
                painter.add(Shape::convex_polygon(pts, fill, stroke));
            }
            Transport::Pause => {
                // 两条竖杠
                let w = s * 0.11;
                let h = s * 0.42;
                let gap = s * 0.13;
                let left = Rect::from_center_size(
                    Pos2::new(c.x - gap / 2.0 - w / 2.0, c.y),
                    Vec2::new(w, h),
                );
                let right = Rect::from_center_size(
                    Pos2::new(c.x + gap / 2.0 + w / 2.0, c.y),
                    Vec2::new(w, h),
                );
                painter.rect_filled(left, 1.0, fill);
                painter.rect_filled(right, 1.0, fill);
            }
            Transport::Stop => {
                // 实心方块
                let sz = s * 0.36;
                let rect = Rect::from_center_size(c, Vec2::splat(sz));
                painter.rect_filled(rect, 2.0, fill);
            }
            Transport::Record => {
                // 实心圆 (常态灰, armed 红, 与 Stop 方块同视觉重量)
                let rad = s * 0.24;
                painter.circle_filled(c, rad, fill);
            }
        }
    }
}

impl Widget for TransportButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(self.size), Sense::click());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let response = &resp;
            // hover: 浅灰底
            if response.hovered() {
                painter.rect_filled(rect, 5.0, Color32::from_rgb(0xd8, 0xd8, 0xd8));
            }
            // 图标几何绘制 (同尺寸)
            self.draw_icon(&painter, rect, self.glyph_color());
            // Record armed: 额外细圆环强调
            if matches!(self.kind, Transport::Record) && self.active {
                painter.circle_stroke(
                    rect.center(),
                    self.size * 0.30,
                    Stroke::new(1.5, Color32::from_rgb(0xcc, 0x22, 0x22)),
                );
            }
        }
        resp
    }
}
