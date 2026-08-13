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

    /// 图标主色 (顶栏深底 #1f2f45 → 图标浅色)
    fn glyph_color(&self) -> Color32 {
        match self.kind {
            Transport::Play | Transport::Pause | Transport::Stop => {
                if self.active {
                    Color32::from_rgb(0x6f, 0xdd, 0x8b) // 播放中 → 亮绿 (深底可读)
                } else {
                    Color32::from_rgb(0xd0, 0xd8, 0xe2) // 常态浅灰蓝 (深底可读)
                }
            }
            Transport::Record => {
                if self.active {
                    Color32::from_rgb(0xff, 0x5f, 0x56) // armed → 亮红
                } else {
                    Color32::from_rgb(0xd0, 0xd8, 0xe2) // 常态浅灰蓝
                }
            }
        }
    }

    /// 在 rect 内绘制几何图标 (三按钮视觉均衡).
    /// 外接框统一 ~9px (k=s*0.42): 三角高0.8k宽0.9k, 方块0.86k, 圆0.86k径.
    fn draw_icon(&self, painter: &eframe::egui::Painter, r: Rect, color: Color32) {
        let c = r.center();
        let s = self.size;
        let stroke = Stroke::new((s / 16.0).max(1.2), color);
        let fill = color;
        let k = s * 0.42;               // 统一图标盒 (24px 按钮 → 盒 ~10px)
        match self.kind {
            Transport::Play => {
                // 实心右向三角: 高 0.8k, 宽 0.9k (外接≈方块/圆)
                let w = k * 0.90;
                let h = k * 0.80;
                let pts = vec![
                    Pos2::new(c.x - w / 2.0, c.y - h / 2.0),
                    Pos2::new(c.x - w / 2.0, c.y + h / 2.0),
                    Pos2::new(c.x + w / 2.0, c.y),
                ];
                painter.add(Shape::convex_polygon(pts, fill, stroke));
            }
            Transport::Pause => {
                // 两条粗竖杠: 高 0.8k (与 Play 三角同高)
                let w = k * 0.20;
                let h = k * 0.80;
                let gap = k * 0.14;
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
                // 实心方块: 边长 0.86k
                let sz = k * 0.86;
                let rect = Rect::from_center_size(c, Vec2::splat(sz));
                painter.rect_filled(rect, 2.0, fill);
            }
            Transport::Record => {
                // 实心圆: 直径 0.86k (与方块同尺寸)
                let rad = k * 0.43;
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
            // hover: 深底上提亮一档 (深色顶栏 hover 更亮)
            if response.hovered() {
                painter.rect_filled(rect, 5.0, Color32::from_rgb(0x2a, 0x3d, 0x58));
            }
            // 图标几何绘制 (同尺寸)
            self.draw_icon(&painter, rect, self.glyph_color());
            // Record armed: 额外细圆环强调 (亮红)
            if matches!(self.kind, Transport::Record) && self.active {
                painter.circle_stroke(
                    rect.center(),
                    self.size * 0.30,
                    Stroke::new(1.5, Color32::from_rgb(0xff, 0x5f, 0x56)),
                );
            }
        }
        resp
    }
}
