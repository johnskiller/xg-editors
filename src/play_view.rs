// PlayView 播放画面 (CentralView::PlayView) 渲染 + 视图模式定义.
// 从 lib.rs 拆出 (Step 2): CentralView enum + render_playview 方法.
// 依赖: XgApp 结构体字段 (共享状态, 单 struct 多文件 impl).

use crate::XgApp;
use eframe::egui;

/// PlayView 右侧垂直滚动条宽度
const SCROLLBAR_W: f32 = 8.0;

/// 中央视图模式 (三 tab): PianoRoll 静态时间轴 / ChannelNotes 每行1通道 / PlayView 播放画面.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CentralView {
    #[default]
    ChannelNotes,
    PianoRoll,
    PlayView,
}

impl CentralView {
    pub fn label(self) -> &'static str {
        match self {
            CentralView::PianoRoll => "Piano Roll",
            CentralView::ChannelNotes => "Channel Notes",
            CentralView::PlayView => "PlayView",
        }
    }
}

use crate::CHANNEL_ROW_H;

impl XgApp {
    /// PlayView 播放画面渲染: 顶部硬件式信息栏 + 左通道矩阵 + 中央 note 瀑布.
    /// 复刻 Octavia Cambiare 播放画面. 数据源全为现有字段 (active_notes/metre/cc_live/live_*),
    /// 渲染只读; 垂直滚动 (pview_scroll) 由本方法处理.
    pub fn render_playview(&mut self, ui: &mut egui::Ui) {
                // ---- 顶部信息栏 (一行, 仿硬件前面板, 全 ASCII) ----
                let top_rect = ui.available_rect_before_wrap();
                ui.allocate_rect(top_rect, egui::Sense::hover());
                let p = ui.painter();
                let bar_h = 22.0;
                let bar_rect = egui::Rect::from_min_max(
                    egui::pos2(top_rect.left(), top_rect.top()),
                    egui::pos2(top_rect.right(), (top_rect.top() + bar_h).min(top_rect.bottom())),
                );
                p.rect_filled(bar_rect, 0.0, egui::Color32::from_rgb(0x0c, 0x14, 0x1e));
                // mm:ss 时间码 (基于 playhead 秒)
                let ppq_eff = if self.smf.is_some() { self.smf.as_ref().unwrap().ppq as f64 } else { self.ppq as f64 };
                let tsec = if let Some(tm) = &self.tempo_map {
                    tm.tick_to_sec(self.playhead_tick, ppq_eff as u32)
                } else {
                    let bpm = self.tempo_bpm.max(1.0);
                    self.playhead_tick as f64 * 60.0 / (ppq_eff.max(1.0) * bpm)
                };
                let m = (tsec / 60.0) as u64;
                let s = (tsec % 60.0) as u64;
                let timecode = format!("{m:03}:{s:02}");
                let cur_poly: u64 = self.active_notes.iter().map(|m| m.len() as u64).sum();
                let (bar, beat, _) = self.playhead_bar_beat();
                let tsig = "4/4";
                let tempo_fmt = format!("{:.2}", self.tempo_bpm);
                let vol_fmt = format!("{:.2}%", self.live_master_vol * 100.0);
                let mode = "Yamaha XG";
                // 事件计数 (3位)
                let evt_fmt = format!("{}", self.play_evt_count.min(999));
                // 顶部字段 (全 ASCII, 与截图布局一致):
                // 000 | 012:025 | TSig 4/4 | Bar 42/1 | Tempo 138.00 | Vol 100.00% | Mode Yamaha XG
                //      Rev XG Hall 2 | Cho XG Chorus 1 | Var XG Cross Delay | Ins XG Through
                let fields: Vec<String> = vec![
                    format!("{evt_fmt}/{}", cur_poly.min(999)),
                    timecode,
                    format!("TSig {tsig}"),
                    format!("Bar {bar}/{beat}"),
                    format!("Tempo {tempo_fmt}"),
                    format!("Vol {vol_fmt}"),
                    format!("Mode {mode}"),
                    format!("Rev XG Hall 2"),
                    format!("Cho XG Chorus 1"),
                    format!("Var XG Cross Delay"),
                    format!("Ins XG Through"),
                ];
                let mut fx = bar_rect.left() + 10.0;
                let label_w = 11.0f32 * 11.0; // 估算宽度可容纳 "Vol 100.00%"
                let font = egui::FontId::monospace(11.0);
                for (i, f) in fields.iter().enumerate() {
                    // 前 7 个字段是主区, 用下划线分隔; 效果器区用淡色
                    let col = if i < 7 { egui::Color32::from_gray(230) } else { egui::Color32::from_gray(140) };
                    p.text(
                        egui::pos2(fx, bar_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        f,
                        font.clone(),
                        col,
                    );
                    fx += label_w;
                    // 主区字段间加竖线分隔
                    if i < 6 {
                        p.line_segment(
                            [egui::pos2(fx + 4.0, bar_rect.top() + 3.0), egui::pos2(fx + 4.0, bar_rect.bottom() - 3.0)],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2a, 0x3a, 0x4a)),
                        );
                        fx += 10.0;
                    }
                }
                // Title (右对齐或第二行? 依截图在后段; 这里放效果器区之后)
                fx += 20.0;
                let title = &self.smf_name;
                p.text(
                    egui::pos2(fx, bar_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("Title {title}"),
                    egui::FontId::monospace(11.0),
                    egui::Color32::from_rgb(0x9f, 0xbf, 0xdf),
                );

                // ---- 主区: 左通道矩阵 + 中央 note 瀑布 (同一 Y 原点, 严格对齐) ----
                let body = egui::Rect::from_min_max(
                    egui::pos2(top_rect.left(), bar_rect.bottom()),
                    egui::pos2(top_rect.right(), top_rect.bottom()),
                );
                // 每通道 2 行高 = 2*CHANNEL_ROW_H (行1 主信息 + 行2 精细信息) — Cambiare attach 结构
                let cell_h = 2.0 * CHANNEL_ROW_H;
                let left_w = 320.0; // 通道矩阵固定宽
                let left_rect = egui::Rect::from_min_max(
                    egui::pos2(body.left(), body.top()),
                    egui::pos2((body.left() + left_w).min(body.right()), body.bottom()),
                );
                let note_rect = egui::Rect::from_min_max(
                    egui::pos2(left_rect.right(), body.top()),
                    egui::pos2(body.right() - SCROLLBAR_W, body.bottom()),
                );

                // ---- 垂直滚动 (16 通道超出视口时; 左矩阵与瀑布共用 c_top 同步滚) ----
                // 总内容高 = 16 行; 视口高 = body 高; 滚轮(smooth_scroll_delta)驱动, clamp 边界
                // egui 滚轮: 向下滚 = smooth_scroll_delta.y < 0 (eframe 取反) → 内容下移=滚轮向下
                let scroll_total_h = 16.0 * cell_h;
                let scroll_view_h = body.height();
                let scroll_max = (scroll_total_h - scroll_view_h).max(0.0);
                let wheel = ui.input(|i| -i.smooth_scroll_delta.y); // 向下滚 = 正 (内容上移)
                if wheel.abs() > 0.0 && scroll_max > 0.0 {
                    self.pview_scroll = (self.pview_scroll + wheel).clamp(0.0, scroll_max);
                }
                // 绘制时内容区顶部 (滚动后)。信息栏/标尺固定, 行内容随滚动。
                let c_top = body.top() - self.pview_scroll;

                // ---- PlayView 右侧垂直滚动条 (可视 + 可点击/拖拽, scroll_max=0 时隐藏) ----
                if scroll_max > 0.0 {
                    let sb_rect = egui::Rect::from_min_max(
                        egui::pos2(body.right() - SCROLLBAR_W, body.top()),
                        egui::pos2(body.right(), body.bottom()),
                    );
                    // 交互: 点击轨道定位 + 按住拖动 thumb 滚动
                    let sb_resp = ui.interact(sb_rect, ui.id().with("pview_sb"), egui::Sense::click_and_drag());
                    if sb_resp.dragged() || sb_resp.clicked() {
                        if let Some(pos) = sb_resp.interact_pointer_pos() {
                            let frac = ((pos.y - body.top()) / body.height().max(1.0)).clamp(0.0, 1.0);
                            self.pview_scroll = frac * scroll_max;
                        }
                    }
                    // 轨道底色
                    p.rect_filled(sb_rect, 0.0, egui::Color32::from_rgb(0x0a, 0x10, 0x18));
                    // thumb (位置 ∝ scroll 比例; 尺寸 ∝ 视口/内容)
                    let thumb_span = body.height().max(1.0);
                    let thumb_h = (scroll_view_h / scroll_total_h * thumb_span).clamp(24.0, thumb_span);
                    let thumb_travel = (thumb_span - thumb_h).max(0.0);
                    let thumb_y = body.top() + (self.pview_scroll / scroll_max) * thumb_travel;
                    p.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(sb_rect.left() + 1.0, thumb_y),
                            egui::vec2(SCROLLBAR_W - 2.0, thumb_h),
                        ),
                        2.0,
                        egui::Color32::from_rgb(0x3a, 0x50, 0x66),
                    );
                    // 与瀑布的淡分隔线
                    p.line_segment(
                        [egui::pos2(sb_rect.left(), body.top()), egui::pos2(sb_rect.left(), body.bottom())],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(35)),
                    );
                }
                // 瀑布区深色背景 (避免露出 CentralPanel 白底; Cambiare 暗色墙纸风格)
                // v92+: 优先铺 Horsehead 星云纹理 (暗色墙纸), 纹理未加载时退回首图纯色
                let starfield_tex = match &self.starfield_tex {
                    Some(t) => Some(t.id()),
                    None => {
                        let img = egui::ColorImage::from_rgba_unmultiplied(
                            [crate::starfield::WIDTH, crate::starfield::HEIGHT],
                            crate::starfield::BYTES,
                        );
                        let tex = ui.ctx().load_texture("starfield", img, egui::TextureOptions::LINEAR);
                        let id = tex.id();
                        self.starfield_tex = Some(tex);
                        Some(id)
                    }
                };
                if let Some(id) = starfield_tex {
                    // 背景墙纸: 固定全屏 (不随 panel resize 缩放).
                    // 缩放/锚定绑定 ctx.screen_rect (总画布, 与 body/panel 无关):
                    // panel resize 只改变可见窗口(遮挡), 星云本身不变; 只有整窗口 resize 才等比变化.
                    // scale 保证星云至少铺满整个 screen → body(⊆screen) 永不露深色底.
                    // 关键: 用 body clip 画星云 → 不覆盖顶部信息栏区域; 但锚定/缩放仍用 screen_rect.
                    let p_bg = p.with_clip_rect(body);
                    p_bg.rect_filled(body, 0.0, egui::Color32::from_rgb(0x0e, 0x17, 0x22));
                    let screen = ui.ctx().screen_rect();
                    let star_w = crate::starfield::WIDTH as f32;
                    let star_h = crate::starfield::HEIGHT as f32;
                    let scale = (screen.width() / star_w).max(screen.height() / star_h);
                    let bg_rect = egui::Rect::from_min_size(
                        egui::pos2(screen.left(), screen.top()),
                        egui::vec2(star_w * scale, star_h * scale),
                    );
                    p_bg.image(
                        id,
                        bg_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                } else {
                    p.rect_filled(body, 0.0, egui::Color32::from_rgb(0x0e, 0x17, 0x22));
                }
                // 瀑布区顶部标尺: 音高方向 128 等分的参照 (C4 附近标 C4)
                p.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(note_rect.left(), note_rect.top()),
                        egui::pos2(note_rect.right(), (note_rect.top() + 14.0).min(note_rect.bottom())),
                    ),
                    0.0,
                    egui::Color32::from_rgb(0x0c, 0x14, 0x1e),
                );
                p.line_segment(
                    [egui::pos2(note_rect.left(), note_rect.top() + 14.0), egui::pos2(note_rect.right(), note_rect.top() + 14.0)],
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2a, 0x3a, 0x4a)),
                );
                // Pitch 数标 (每 Octave: C 刻度; 0,12,24,...,120) — 每个八度都画 tick 竖线
                for oct in 0..=10 {
                    let p0 = oct * 12;
                    let xp = note_rect.left() + (p0 as f32 / 128.0 * note_rect.width());
                    let midi = p0;
                    // tick 竖线 (每个八度): 从标尺带底边向上 5px (0.5 在带内, 略凸出到下沿)
                    // 暗色低调 (gray 70), 比横线(40)略亮可辨, 不喧宾夺主
                    p.line_segment(
                        [
                            egui::pos2(xp, note_rect.top() + 14.0),
                            egui::pos2(xp, (note_rect.top() + 14.0 - 5.0).max(note_rect.top() + 1.0)),
                        ],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(70)),
                    );
                    let label = if midi % 12 == 0 {
                        format!("C{}", (midi / 12) as i32 - 1)
                    } else { String::new() };
                    if oct % 2 == 1 || oct == 0 {
                        p.text(
                            egui::pos2(xp + 2.0, note_rect.top() + 1.0),
                            egui::Align2::LEFT_TOP,
                            label,
                            egui::FontId::monospace(9.0),
                            egui::Color32::from_gray(120),
                        );
                    }
                }
                // 底部信息行 (poly / maxPoly / 说明) 在瀑布区下沿反之
                _ = &cur_poly;

                // 16 通道行 (只有 16 行, 与 active_notes 一致; 32 part 显示未来加)
                // 行内容用 clip 到 body 的 painter: 滚动时上下溢出部分被裁掉, 不影响信息栏/标尺
                let p_body = ui.painter().with_clip_rect(body);
                let n_rows = 16usize;
                for i in 0..n_rows {
                    let y0 = c_top + i as f32 * cell_h;
                    // 行完全在视口上方/下方时跳过 (预剪裁, 只画可见行)
                    if y0 + cell_h <= body.top() { continue; }
                    if y0 > body.bottom() { break; }
                    let row_rect = egui::Rect::from_min_max(
                        egui::pos2(left_rect.left(), y0),
                        egui::pos2(left_rect.right(), y0 + cell_h),
                    );
                    // 行背景 (交错深浅, 同 Channel 视图色系) — 半透明透出星云墙纸
                    let base: (u8, u8, u8) = if i % 2 == 0 { (0x12, 0x1e, 0x2e) } else { (0x1f, 0x2f, 0x45) };
                    p_body.rect_filled(row_rect, 0.0, egui::Color32::from_rgba_unmultiplied(base.0, base.1, base.2, 150));
                    // 行分隔线
                    p_body.line_segment(
                        [egui::pos2(left_rect.left(), y0), egui::pos2(note_rect.right(), y0)],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(40)),
                    );
                    // 八度 tick 小刻度 (每个八度边界, 只在行顶部画小短竖线, 同顶部标尺风格)
                    // 暗色低调 (gray 70), 比横线(40)略亮可辨, 不喧宾夺主
                    let tick_nw = note_rect.width();
                    for oct in 0..=10 {
                        let tx = note_rect.left() + (oct as f32 * 12.0 / 128.0 * tick_nw);
                        p_body.line_segment(
                            [egui::pos2(tx, y0 + 1.0), egui::pos2(tx, (y0 + 1.0 + 5.0).min(y0 + cell_h))],
                            egui::Stroke::new(1.0, egui::Color32::from_gray(70)),
                        );
                    }

                    // ===== 左矩阵 行1: [CH] [voice + 白metre条] [绿ccVis] =====
                    let cy1 = y0 + CHANNEL_ROW_H * 0.5;
                    // CH 号
                    p_body.text(
                        egui::pos2(left_rect.left() + 6.0, cy1),
                        egui::Align2::LEFT_CENTER,
                        format!("{:02}", i + 1),
                        egui::FontId::monospace(13.0),
                        egui::Color32::from_gray(245),
                    );
                    // voice 名 (当前通道音色)
                    let ch_voice = if self.smf.is_some() {
                        self.live_voice_names.get(i).cloned().unwrap_or_default()
                    } else {
                        String::new()
                    };
                    // 实时音色名优先用 live_program/live_bank 映射 (播放时 bank/prog 动态)
                    let voice_text = if self.smf.is_some() && self.active_notes[i].len() > 0 || self.live_program[i] != 0 {
                        self.voice_name_for_channel(i)
                    } else {
                        ch_voice
                    };
                    let vx0 = left_rect.left() + 26.0;
                    let vw = 150.0; // voice 名区宽 (后面 metre 条覆盖其上)
                    let mut vtext = voice_text;
                    if vtext.chars().count() > 12 { let cut: String = vtext.chars().take(11).collect(); vtext = format!("{cut}."); }
                    p_body.text(
                        egui::pos2(vx0, cy1),
                        egui::Align2::LEFT_CENTER,
                        &vtext,
                        egui::FontId::monospace(10.0),
                        egui::Color32::from_gray(200),
                    );
                    // 白 metre 条 (覆盖在 voice 名上 — Cambiare: L(b.voice,[b.metre.canvas]))
                    let metre = self.smooth_meter_target(i).clamp(0.0, 1.0);
                    let mw = (metre * vw).clamp(0.0, vw);
                    if mw > 1.0 {
                        p_body.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(vx0, cy1 - 5.0), egui::vec2(mw, 10.0)),
                            0.0,
                            egui::Color32::from_white_alpha(90),
                        );
                    }
                    // 绿 ccVis 竖条 (voice 右侧固定区): CC 候选 [7,11,1,91,93,94,74,5]
                    let cc_list = [7u8, 11, 1, 91, 93, 94, 74, 5];
                    let ccx0 = left_rect.left() + 178.0;
                    for (ci, cc) in cc_list.iter().enumerate() {
                        let ccv = self.cc_live[i][*cc as usize];
                        let cch = (ccv as f32 / 127.0 * (2.0 * CHANNEL_ROW_H)).clamp(1.0, 2.0 * CHANNEL_ROW_H - 2.0);
                        let cx = ccx0 + ci as f32 * 7.0;
                        // 半透明绿竖条, 底部对齐行1中段
                        p_body.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(cx, y0 + 2.0 * CHANNEL_ROW_H - cch), egui::vec2(4.0, cch)),
                            0.5,
                            egui::Color32::from_rgba_unmultiplied(0x2e, 0xcc, 0x40, 200),
                        );
                    }

                    // ===== 左矩阵 行2: [type] [std] [msb prg lsb] [pan] =====
                    let cy2 = y0 + 1.5 * CHANNEL_ROW_H;
                    let (bmsb, blsb) = self.live_bank[i];
                    let prg = self.live_program[i];
                    // type: VX (normal) / DX (drum); 判定: msb==127 → DX
                    let vtype = if bmsb == 127 { "DX" } else { "VX" };
                    let std_s = "XG";
                    let msb_fmt = format!("{bmsb:03}");
                    let prg_fmt = format!("{:03}", prg + 1); // XG 显示 1-based? 规格书说补零; 旧代码 cur_prog 1-based 显示
                    let lsb_fmt = format!("{blsb:03}");
                    // Pan: 简单横条 (中间=center)
                    let panv = self.cc_live[i][10];
                    let ptext = format!("Pan {panv:03}");
                    p_body.text(
                        egui::pos2(left_rect.left() + 26.0, cy2),
                        egui::Align2::LEFT_CENTER,
                        format!("{vtype} {std_s} {msb_fmt} {prg_fmt} {lsb_fmt}"),
                        egui::FontId::monospace(10.0),
                        egui::Color32::from_gray(150),
                    );
                    p_body.text(
                        egui::pos2(left_rect.left() + 236.0, cy2),
                        egui::Align2::LEFT_CENTER,
                        &ptext,
                        egui::FontId::monospace(10.0),
                        egui::Color32::from_gray(160),
                    );
                    // pan 横条 (min=0 left, 64=center, 127=right)
                    let pan_f = (panv as f32).clamp(0.0, 127.0);
                    let pan_ac = if panv == 64 { egui::Color32::from_gray(220) } else { egui::Color32::from_rgb(0x6f, 0xcf, 0x97) };
                    p_body.text(
                        egui::pos2(left_rect.left() + 268.0, cy2),
                        egui::Align2::LEFT_CENTER,
                        if pan_f < 60.0 { "<" } else if pan_f > 68.0 { ">" } else { "=" },
                        egui::FontId::monospace(12.0),
                        pan_ac,
                    );
                    // 左矩阵与瀑布之间的分隔竖线
                    p_body.line_segment(
                        [egui::pos2(left_rect.right(), body.top()), egui::pos2(left_rect.right(), body.bottom())],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(45)),
                    );

                    // ===== 中央 note 瀑布 (每通道一行, 与左矩阵同 Y) =====
                    // X 轴 = 音高 0..127 → 128 等分; 竖条固定半音宽; 白键满高, 黑键 2/3
                    let nw = note_rect.width();
                    let nh = cell_h - 2.0;
                    let active = &self.active_notes[i];
                    for (&pitch, &vel) in active.iter() {
                        let sx = note_rect.left() + (pitch as f32 / 128.0 * nw).round();
                        let ex = note_rect.left() + (((pitch + 1) as f32) / 128.0 * nw).round();
                        let dx = (ex - sx).max(2.0);
                        let is_black = matches!(pitch % 12, 1 | 3 | 6 | 8 | 10);
                        let h = if is_black { (nh * 2.0 / 3.0).round().max(1.0) } else { nh };
                        let alpha = ((vel as f32 / 127.0) * 255.0) as u8;
                        let col = if is_black {
                            // 黑键 = 通道 accent 色 (HSV 均匀分布, 同 channel_note_color)
                            let (r, g, b) = self.channel_note_color(i, vel);
                            egui::Color32::from_rgba_premultiplied(r, g, b, alpha)
                        } else {
                            // 白键 = 前景白 + alpha(力度)
                            egui::Color32::from_white_alpha(alpha)
                        };
                        // 竖条从行顶开始 (Cambiare: fillRect(sx, 0, dx, h), 顶对齐)
                        p_body.rect_filled(
                            egui::Rect::from_min_size(egui::pos2(sx, y0 + 1.0), egui::vec2(dx, h)),
                            0.0,
                            col,
                        );
                    }
                }
                // 底部 poly/maxPoly 信息行
                let poly_total: u64 = self.active_notes.iter().map(|m| m.len() as u64).sum();
                if body.bottom() >= top_rect.bottom() - 2.0 {
                    p.text(
                        egui::pos2(note_rect.left() + 6.0, top_rect.bottom() - 2.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("Poly {} / max {}", poly_total, self.max_poly),
                        egui::FontId::monospace(9.0),
                        egui::Color32::from_gray(110),
                    );
                }
    }
}

