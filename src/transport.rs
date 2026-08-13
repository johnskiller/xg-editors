//! Transport 控制组件 (TopBar 一行, 组件化).
//!
//! egui custom widget: `TransportButton` 实现 `egui::Widget`,
//! 复用 M/S 按钮(ms_button.rs)的 custom widget 模式.
//! 图标用 emoji-icon-font 字形 (egui 0.29 内置, 已实测覆盖 ▶⏸⏹⏺☰, 零字体依赖).

use eframe::egui::{Align2, Color32, FontId, Response, Sense, Stroke, Ui, Vec2, Widget};

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

    pub(crate) fn glyph(&self) -> &'static str {
        match self.kind {
            Transport::Play => "\u{25B6}",    // ▶
            Transport::Pause => "\u{23F8}",   // ⏸
            Transport::Stop => "\u{23F9}",    // ⏹
            Transport::Record => "\u{23FA}",  // ⏺
        }
    }

    fn glyph_color(&self) -> Color32 {
        match self.kind {
            Transport::Play | Transport::Pause | Transport::Stop => {
                if self.active {
                    Color32::from_rgb(0x5a, 0xd0, 0x7a) // 播放中 → 绿
                } else {
                    Color32::from_rgb(0xe8, 0xe8, 0xe8) // 常态灰白
                }
            }
            Transport::Record => {
                if self.active {
                    Color32::from_rgb(0xff, 0x3b, 0x30) // armed → 红
                } else {
                    Color32::from_rgb(0xe8, 0xe8, 0xe8) // 常态灰
                }
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
            let (bg, fg) = if response.hovered() {
                // hover 提亮: 半透明浅色底 + 亮字形
                (Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0x14),
                 self.glyph_color().gamma_multiply(1.3))
            } else {
                (Color32::TRANSPARENT, self.glyph_color())
            };
            // 背景圆角方块 (hover)
            painter.rect_filled(rect, 4.0, bg);
            // 字形
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                self.glyph(),
                FontId::proportional(self.size * 1.1),
                fg,
            );
            // Record armed: 额外画一个细圆环强调
            if matches!(self.kind, Transport::Record) && self.active {
                painter.circle_stroke(rect.center(), self.size / 2.0 + 1.0, Stroke::new(1.5, Color32::from_rgb(0xff, 0x3b, 0x30)));
            }
        }
        resp
    }
}
