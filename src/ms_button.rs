//! Mute/Solo 自定义控件 (egui custom widget, John 2026-08-13 要求用 custom control 而非散装绘制).
//!
//! 一个「小方块 + 字母(M/S)」 toggle 按钮, 带:
//! - active 态配色 (Mute=红, Solo=琥珀)
//! - hover 提亮
//! - 点击切换 (调用方持有状态, 通过 Response::clicked/toggled 判定)
//!
//! 用法 (示意; 需在 egui 上下文内调用):
//! ```text
//! if ui.put(rect, MSButton::new(MSKind::Mute, active).size(18.0)).clicked() { /* toggle */ }
//! ```

use eframe::egui::{Align2, Color32, FontId, Rect, Response, Sense, Ui, Vec2};
use eframe::egui;

/// Mute / Solo 按钮类型
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MSKind {
    /// Mute (静音) — active 色: 红
    Mute,
    /// Solo (独奏) — active 色: 琥珀
    Solo,
}

impl MSKind {
    pub fn glyph(&self) -> &'static str {
        match self {
            MSKind::Mute => "M",
            MSKind::Solo => "S",
        }
    }

    /// active 状态下的底色 (M 红 / S 琥珀, DAW 惯例)
    pub fn active_color(&self) -> Color32 {
        match self {
            MSKind::Mute => Color32::from_rgb(0xe0, 0x35, 0x35),
            MSKind::Solo => Color32::from_rgb(0xff, 0xb0, 0x30),
        }
    }
}

/// M/S 自定义控件
#[derive(Clone, Copy)]
pub struct MSButton {
    kind: MSKind,
    active: bool,
    size: f32,
}

impl MSButton {
    pub fn new(kind: MSKind, active: bool) -> Self {
        Self {
            kind,
            active,
            size: 18.0,
        }
    }

    /// 按钮边长 (默认 18; 行高小时可传 min(row_h-2, 18) 收缩)
    pub fn size(mut self, s: f32) -> Self {
        self.size = s.max(10.0);
        self
    }
}

impl egui::Widget for MSButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let (rect, mut resp) =
            ui.allocate_exact_size(Vec2::splat(self.size), Sense::click());

        // 状态色: active → 类型色; hover → 提亮灰; 常态 → 灰
        let bg = if self.active {
            self.kind.active_color()
        } else if resp.hovered() {
            Color32::from_rgb(0x5c, 0x5c, 0x5c)
        } else {
            Color32::from_rgb(0x44, 0x44, 0x44)
        };
        // 文字色: active 用更亮白, 常态灰白
        let fg = if self.active {
            Color32::from_rgb(0xff, 0xff, 0xff)
        } else {
            Color32::from_gray(230)
        };

        let p = ui.painter();
        p.rect_filled(rect, 3.0, bg);
        // active 时加细描边, 增强辨识
        if self.active {
            p.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 90)));
        }
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            self.kind.glyph(),
            FontId::monospace((self.size * 0.62).max(10.0)),
            fg,
        );

        // 点击/悬停反馈
        if resp.clicked() {
            resp.mark_changed();
        }
        resp
    }
}
