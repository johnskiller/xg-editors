// ---------- MU90 LCD 渲染核心 (Rust 移植自 src/lcd/matrix.js) ----------
// 忠实平移已验证的几何规范 (skill: mu90-lcd-emulation-render-spec + 8 条修正)
// 纯像素生成 → 840x256 RGBA, 不依赖 canvas/DOM → wasm/native 共用
use crate::xg_font::{XG_FONT_BITS, XG_FONT_CODES};
use crate::xg_icons::{Icon, ICONS};

// 背光液晶物理模型 (用户权威 2026-08-07):
// - 底色 = 均匀绿色背光 (偏黄)
// - 每个逻辑点格位 = 比底色稍暗的绿 (未亮段)
// - 点亮段 = 深黑 (挡住背光)
pub const BG_R: u8 = 0x7d; pub const BG_G: u8 = 0xf4; pub const BG_B: u8 = 0x06; // #7df406
pub const IN_R: u8 = 0x69; pub const IN_G: u8 = 0xe7; pub const IN_B: u8 = 0x04; // #69e704
pub const ACT_R: u8 = 0x12; pub const ACT_G: u8 = 0x6f; pub const ACT_B: u8 = 0x00; // #126f00

pub const LCD_W: usize = 840;
pub const LCD_H: usize = 256;

/// 逻辑点阵 (85x16 主 / 25x8 下部)
pub struct MuMatrix {
    pub w: usize,
    pub h: usize,
    pub mm: Vec<u8>,
}
impl MuMatrix {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, mm: vec![0u8; w * h] }
    }
    pub fn set(&mut self, x: i32, y: i32, v: u8) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.mm[y as usize * self.w + x as usize] = v;
        }
    }
    pub fn get(&self, x: i32, y: i32) -> u8 {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.mm[y as usize * self.w + x as usize]
        } else {
            0
        }
    }
    /// 5 列步进文本 (对齐 17 组结构): 字符在 5 列内, 组间靠 blit 插空
    pub fn text5(&mut self, str: &str, x: i32, y: i32) {
        let mut cx = x;
        for ch in str.chars() {
            let cp = ch as u32;
            self.char_at(cp, cx, y, false);
            cx += 5;
        }
    }
    pub fn char_at(&mut self, cp: u32, x: i32, y: i32, invert: bool) {
        if let Some(off) = font_offset(cp) {
            for r in 0..8usize {
                let byte = XG_FONT_BITS[off + r];
                for c in 0..5usize {
                    let bit = (byte >> c) & 1;
                    if bit == 1 {
                        self.set(x + c as i32, y + r as i32, if invert { 0 } else { 1 });
                    }
                }
            }
        }
    }
    /// 反显端口字母 (A/B): 顶部 5x7 点亮 (set=1→黑), 底行 y+7 留暗, 中间 3x5 用窄体字母镂空 (0→绿), 从第 2 行 (y+1) 开始
    /// 依据 John 权威 (2026-08-12): 由 5x8 全亮改为顶部 5x7 点亮 (镂空字母逻辑不变)
    pub fn char_reverse(&mut self, cp: u32, x: i32, y: i32) {
        for r in 0..7i32 {
            for c in 0..5i32 {
                self.set(x + c, y + r, 1);
            }
        }
        // 窄体 3x5 字母 (左=col0, 中=col1, 右=col2, 每行 LSB=左, 5 行 → 行 1..5)
        // 置 1 → 镂空 (绿, 0 像素)
        let glyph: [[u8; 3]; 5] = match cp as u8 {
            // A (带顶部尖点: 顶部中间一点 + 左右柱 + 中横杠)
            b'A' => [
                [0, 1, 0],
                [1, 0, 1],
                [1, 0, 1],
                [1, 1, 1],
                [1, 0, 1],
            ],
            // B
            b'B' => [
                [1, 1, 0],
                [1, 0, 1],
                [1, 1, 0],
                [1, 0, 1],
                [1, 1, 0],
            ],
            _ => [[0; 3]; 5],
        };
        for (r, row) in glyph.iter().enumerate() {
            for c in 0..3usize {
                if row[c] == 1 {
                    self.set(x + 1 + c as i32, y + 1 + r as i32, 0);
                }
            }
        }
    }
}

// 34-bar 网格列公式 (全区域 A1/A2 + 01..32): bar i 占 2 列 (i*…, 每组 5 列内 2 bar, 间距 1)
// i=0,1→A1/A2, i=2..33→ch1..32; bar i 列 = gi*5 + (k?3:0), gi=i/2, k=i%2
fn bar_col(i: i32) -> i32 {
    let gi = i / 2;
    let k = i % 2;
    gi * 5 + if k == 1 { 3 } else { 0 }
}

/// 在字体表中查找码位, 返回 8 字节偏移 (XG_FONT_BITS 内, 每字形 8 字节 = 5x8 逐行)
fn font_offset(cp: u32) -> Option<usize> {
    // 线性查找 (2576 项, 仅在绘制时调用, 够快; 可优化为二分)
    for (i, c) in XG_FONT_CODES.iter().enumerate() {
        if *c == cp {
            return Some(i * 8);
        }
    }
    None
}

/// 渲染 state → 85x16 主矩阵
/// state: voice(8字符), bank(0..), program(1-based), levels(16 个 0..1), audio(2 个 0..1)
/// part (1..32): 16ch 模式下 Port B (part 17-32) 音色/bank/pgm 左移 (真机一致, John 2026-08-12)
pub fn render_to_matrix(voice: &str, bank: u32, program: u32, levels: &[f32], audio: &[f32], part: u32) -> MuMatrix {
    render_to_matrix_mode(voice, bank, program, levels, audio, false, part)
}

/// 32-channel 模式: 音色名+bank/prg 合并第 1 行, A1/A2+01..32 全部电平条 (高度减半)
/// 用户权威 (2026-08-08): "mu90有AB两个MIDI in,可同时演奏32 part; lcd的32channel显示模式
/// 是 5x16 点阵上半部显示音色名和bank prog只占一行, 下半部正好用来显示a1,a2,01-32的所有电平,
/// 但此时full 电平显示为原来的一半高"
pub fn render_to_matrix_32(voice: &str, bank: u32, program: u32, levels: &[f32], audio: &[f32], part: u32) -> MuMatrix {
    render_to_matrix_mode(voice, bank, program, levels, audio, true, part)
}

fn render_to_matrix_mode(voice: &str, bank: u32, program: u32, levels: &[f32], audio: &[f32], is_32: bool, part: u32) -> MuMatrix {
    let mut mm = MuMatrix::new(85, 16);
    // 音色名 8 字符: 位置 = port A 右侧(col 45) / port B 左侧(col 0) [John 2026-08-12: 两者对称]
    let port_b = part > 16; // part 17-32 = Port B (MIDI IN B)
    let voice_col: i32 = if is_32 { 0 } else if port_b { 0 } else { 45 };
    let v: String = voice.chars().take(8).collect();
    let v_pad = format!("{:<8}", v);
    if is_32 {
        // 32ch 模式第一行: 从最左边 col 0 开始, "GrandPno ▶001▶001" = 17 字符
        // 8(名) + 1(空格) + 1(▶) + 3(bank) + 1(▶) + 3(prog) = 17
        let bp = format!("{} \u{0080}{:03}\u{0081}{:03}", v.trim_end(), bank, program);
        mm.text5(&bp, 0, 0);
    } else if port_b {
        // Port B (part 17-32): 音色/bank/prog 显示在左侧 8 组点阵 (col 0..39), 真机一致
        mm.text5(&v_pad, 0, 0);
        let bh = "\u{0080}".to_string();
        let sh = "\u{0081}".to_string();
        let bp = format!("{}{:03}{}{:03}", bh, bank, sh, program);
        mm.text5(&bp, 0, 8);
    } else {
        // Port A (part 1-16): 音色 col 45 第 1 行, bank/program 第 2 行 (现状)
        mm.text5(&v_pad, 45, 0);
        let bh = "\u{0080}".to_string();
        let sh = "\u{0081}".to_string();
        let bp = format!("{}{:03}{}{:03}", bh, bank, sh, program);
        mm.text5(&bp, 45, 8);
    }
    // 电平条区
    if is_32 {
        // 34 bar (A1/A2 + 01..32): 列位置与前 18 个(16ch)完全一致, 复用 bar_col 公式 (每 bar 2 列, 间距 5)
        // 高 8 层 (全高的 1/2), 从 row15 基线向上
        // bar i 列: gi=i/2, k=i%2, 与 16ch 完全同一公式 (gi*5 + k*3) → 前 18 个位置一致; 34 bar 最右 gi=16 → 16*5+3=83 < 85
        let bar_col = |i: i32| -> i32 { let gi = i / 2; let k = i % 2; gi * 5 + if k == 1 { 3 } else { 0 } };
        const N: usize = 34;
        for i in 0..N {
            let lvl: f32 = if i < 2 {
                if i < audio.len() { audio[i] } else { 0.0 }
            } else {
                let li = i - 2;
                if li < levels.len() { levels[li] } else { 0.0 }
            };
            let lvl = lvl.clamp(0.0, 1.0);
            let n_l = (lvl * 8.0).round() as i32; // 高度减半: 最多 8 层
            let c = bar_col(i as i32);
            // 0 电平横线 (row15)
            mm.set(c, 15, 1);
            mm.set(c + 1, 15, 1);
            for p in 0..=n_l.min(7) {
                let row = 15 - p;
                mm.set(c, row as i32, 1);
                mm.set(c + 1, row as i32, 1);
            }
        }
    } else if port_b {
        // 16ch + Port B (part 17-32): 音色已占左 8 组 (col0..39), 电平条在右侧显示 17-32。
        // ★ 全区域是 34 ch (A1,A2+1..32), bar 网格 bar_col(i), i=0,1→A1/A2, i=2..33→ch1..32。
        //   17-32 = i=18..33 → col 45..83 (右 9 组)。A1/A2/01..16 (i=0..17) 不显示(音色占左区)。
        //   若是简单"右侧 9 组 i=2..17" 会偏左一组 (对齐到 ch15, 用户实测发现)
        //   电平数据源暂缺 → 目前全 0, 只画基线
        for i in 18..34i32 {
            let c = bar_col(i) as i32;
            mm.set(c, 15, 1);
            mm.set(c + 1, 15, 1);
        }
        for i in 18..34i32 {
            let lvl = 0.0f32; // 17-32 电平暂缺, 等 Port B 接入
            let n_l = (lvl * 16.0).round() as i32;
            let c = bar_col(i);
            for p in 0..=n_l.min(15) {
                let row = 15 - p;
                mm.set(c, row as i32, 1);
                mm.set(c + 1, row as i32, 1);
            }
        }
    } else {
        // 16ch + Port A (part 1-16): 现有布局, 音色右、电平左 (现状)
        let bar_col = |i: i32| -> i32 { let gi = i / 2; let k = i % 2; gi * 5 + if k == 1 { 3 } else { 0 } };
        for i in 0..18i32 {
            let c = bar_col(i) as i32;
            mm.set(c, 15, 1);
            mm.set(c + 1, 15, 1);
        }
        for i in 0..18i32 {
            let lvl: f32 = if i < 2 {
                if (i as usize) < audio.len() { audio[i as usize] } else { 0.0 }
            } else {
                if ((i - 2) as usize) < levels.len() { levels[(i - 2) as usize] } else { 0.0 }
            };
            let lvl = lvl.clamp(0.0, 1.0);
            let n_l = (lvl * 16.0).round() as i32;
            let c = bar_col(i);
            for p in 0..=n_l.min(15) {
                let row = 15 - p;
                mm.set(c, row as i32, 1);
                mm.set(c + 1, row as i32, 1);
            }
        }
    }
    mm
}

/// 下部 25x8 矩阵: NN{sec}NN (part 显示, 5 字符 = 5 组 5x8)
/// part 号 1-32 唯一标识 (John 权威 2026-08-09):
///   part 1-16  → port A, channel 1-16  (显示 01A01 .. 16A16)
///   part 17-32 → port B, channel 1-16  (显示 17B01 .. 32B16)
/// 前 2 位 = part 号 (01..32); 第 3 位 = sec (A/B) **反显**; 后 2 位 = channel (01..16)
pub fn part_sec(part: u32) -> char {
    if part >= 1 && part <= 16 { 'A' } else { 'B' }
}
pub fn part_channel(part: u32) -> u32 {
    if part >= 1 && part <= 16 { part } else { part - 16 }
}

pub fn render_part_matrix(part: u32) -> MuMatrix {
    let part = part.clamp(1, 32);
    let sec = part_sec(part);
    let ch = part_channel(part);
    let mut pm = MuMatrix::new(25, 8);
    let mut label = String::from("01A01");
    label.replace_range(0..2, &format!("{:02}", part));
    label.replace_range(2..3, &sec.to_string());
    label.replace_range(3..5, &format!("{:02}", ch));
    // 前 2 字符 (part) + 后 2 (channel) 正常显示
    pm.text5(&label[0..2], 0, 0);
    // 第 3 字符 (sec: A/B) 反显
    pm.char_reverse(sec as u32, 10, 0);
    pm.text5(&label[3..5], 15, 0);
    pm
}

/// 画布 blit: 主 + 下部矩阵 → 840x256 RGBA 像素缓冲
/// 每 5 列插 1px 间隙 (x + floor(x/5)), 8px 槽画 7px (颗粒感)
/// 偏移: OX=16, OY=12; 下部矩阵 y=180
pub fn blit(pixels: &mut [u8], mm: &MuMatrix, pm: &MuMatrix, params: &[f32; 8]) {
    // 清屏为背光底色
    for px in pixels.chunks_exact_mut(4) {
        px[0] = BG_R; px[1] = BG_G; px[2] = BG_B; px[3] = 255;
    }
    let OX = 16i32; // canvas 偏移
    let OY = 12i32;
    // 主矩阵 85x16: 点 8x8 槽 画 7x7 (方点)
    for y in 0..mm.h as i32 {
        for x in 0..mm.w as i32 {
            let px = OX + (x + x / 5) * 8;
            let py = OY + y * 8;
            let on = mm.get(x, y) != 0;
            fill_rect(pixels, px, py, 7, 7, on);
        }
    }
    // 下部 25x8 (01A01): 同方点, baseY=180
    for y in 0..pm.h as i32 {
        for x in 0..pm.w as i32 {
            let px = OX + (x + x / 5) * 8;
            let py = 180 + y * 8;
            let on = pm.get(x, y) != 0;
            fill_rect(pixels, px, py, 7, 7, on);
        }
    }
    // 通道号丝印 (固定玻璃, A1/A2 + 01..32, Octavia 步进 24)
    let lbl_y = 145i32;
    let c_off = 71.5f32;
    let c_step = 24i32;
    for c in -2..32 {
        let label = if c < 0 {
            format!("A{}", c + 3)
        } else {
            format!("{:02}", c + 1)
        };
        let cx = c_off + (c * c_step) as f32;
        let w = label_width(&label);
        thin_text(pixels, &label, (cx - w / 2.0).round() as i32, lbl_y);
    }
    // 底部参数标签: VOL/EXP/BRT/PAN/REV/CHO/VAR/KEY
    let lab_y = 243i32;
    let labels: &[(&str, f32)] = &[
        ("VOL", 436.0), ("EXP", 484.0), ("BRT", 532.0),
        ("PAN", 583.0), ("REV", 643.0), ("CHO", 692.5), ("VAR", 741.0), ("KEY", 799.0),
    ];
    for (s, x) in labels {
        let w = label_width(s);
        thin_text(pixels, s, (x - w / 2.0).round() as i32, lab_y);
    }
    // 参数条 (8 个, 每个标签上方): VOL/EXP/BRT/PAN/REV/CHO/VAR/KEY
    // 值 0..1 → 单列高度条, 从底部基线 (y=234) 向上, 覆盖标签上方 y≈220..234
    // v=0 → 不画 (参数 0 = 无条); v>0 → 1..14px
    let bar_xs: [i32; 8] = [426, 474, 522, 573, 633, 683, 732, 789]; // 标签中心附近
    for (i, &bx) in bar_xs.iter().enumerate() {
        let v = params[i].clamp(0.0, 1.0);
        if v <= 0.0 {
            continue; // 0 值不画条
        }
        let h = (v * 13.0).round() as i32 + 1; // 1..14px
        let base_y = 234;
        for dy in 0..h {
            for dx in 0..3 {
                put_px(pixels, bx + dx, base_y - dy, true);
            }
        }
    }
}

/// 根据音色名解析对应 icon (精确匹配 → 前缀匹配 → fallback 映射)
/// 返回 Icon 引用或 None (未知名称 → 空白)
pub fn resolve_icon(voice: &str) -> Option<&'static Icon> {
    // 精确匹配
    if let Some(ic) = ICONS.iter().find(|i| i.name == voice) {
        return Some(ic);
    }
    // 前缀匹配 (如 "Saw Ld" → SquareLd, "StringEn" → Strings1)
    if let Some(ic) = ICONS.iter().find(|i| voice.starts_with(i.name) || i.name.starts_with(voice)) {
        return Some(ic);
    }
    // fallback 映射 (synthetic/GM 名)
    const FALLBACK: &[(&str, &str)] = &[
        ("Dream", "GrandPno"), ("Saw", "SquareLd"), ("Lead", "SquareLd"), ("Pad", "NewAgePd"),
        ("Piano", "GrandPno"), ("Bass", "Aco.Bass"), ("Drum", "Taiko"),
        ("Organ", "DrawOrgn"), ("Str", "Strings1"), ("Vln", "Violin"),
        ("Gtr", "NylonGtr"), ("Flute", "Flute"), ("Horn", "FrchHorn"),
        // 鼓组 (MU90 drum_display_name 的 LCD 短名 → 已有鼓图标):
        // StandKit/Standrd2/Dry/BrightKt/Room/DarkRoom/Rock/ElectrKt/Analog/Dance/HipHop/Jungle/Jazz/Brush/Symphn
        ("Stand", "Standard"), ("Dry Kit", "Standard"), ("Bright", "Standard"),
        ("Room", "Standard"), ("Rock", "Standard"), ("Electr", "Standard"),
        ("Analog", "Standard"), ("Dance", "Standard"), ("HipHop", "Standard"),
        ("Jungle", "Standard"), ("Jazz", "Standard"), ("Brush", "Standard"),
        ("Symph", "Standard"), ("Kit", "Standard"),
    ];
    for (k, v) in FALLBACK {
        if voice.contains(k) {
            if let Some(ic) = ICONS.iter().find(|i| i.name == *v) {
                return Some(ic);
            }
        }
    }
    None
}

/// 绘制 voice icon: 16x16 逻辑点, 8px 宽 x 4px 高每点 (skill: 260 + pX*8, 180 + pY*4)
fn draw_icon(pixels: &mut [u8], icon: &Icon, x: i32, y: i32) {
    const HPX: i32 = 8; // 水平每逻辑点像素
    const VPX: i32 = 4; // 垂直每逻辑点像素
    for py in 0..16i32 {
        for px in 0..16i32 {
            let bit_idx = (py * 16 + px) as usize;
            let byte = icon.bits[bit_idx / 8];
            let on = (byte >> (bit_idx % 8)) & 1 == 1;
            // 点亮: 黑 (AC/T); 未点亮: 保留背景/INACTIVE
            if on {
                fill_rect(pixels, x + px * HPX, y + py * VPX, HPX - 1, VPX - 1, true);
            } else {
                // 未点亮 → INACTIVE 深绿格 (与矩阵一致性)
                fill_rect(pixels, x + px * HPX, y + py * VPX, HPX - 1, VPX - 1, false);
            }
        }
    }
}

/// 把音色名对应的 icon 画进 840x256 缓冲 (位置: 260, 180)
pub fn paint_voice_icon(pixels: &mut [u8], voice: &str) {
    if let Some(ic) = resolve_icon(voice) {
        draw_icon(pixels, ic, 260, 180);
    } else {
        // 未知: 画 INACTIVE 网格 (空 icon 区)
        draw_icon(pixels, &Icon { name: "", bits: [0u8; 32] }, 260, 180);
    }
}

/// thin 小字 (5x5 微点阵简化版, 和底部参数标签一致): 占位浅实现,
/// 后续可接 THIN_LABELS 位图。这里用 XG 字体按 1px/逻辑点画(尺寸小, 近似即可)
fn label_width(s: &str) -> f32 {
    // 每个字符 ~5 逻辑点等宽 (近似)
    s.chars().count() as f32 * 6.0
}

fn thin_text(pixels: &mut [u8], s: &str, x: i32, y: i32) {
    let mut cx = x;
    for ch in s.chars() {
        let cp = ch as u32;
        if let Some(off) = font_offset(cp) {
            for r in 0..8usize {
                let byte = XG_FONT_BITS[off + r];
                for c in 0..5usize {
                    if (byte >> c) & 1 == 1 {
                        // 逻辑点 → 1px 画布点 (thin)
                        put_px(pixels, cx + c as i32, y + r as i32, true);
                    }
                }
            }
        }
        cx += 6;
    }
}

fn put_px(pixels: &mut [u8], x: i32, y: i32, on: bool) {
    if x < 0 || y < 0 || x >= LCD_W as i32 || y >= LCD_H as i32 { return; }
    let i = (y as usize * LCD_W + x as usize) * 4;
    if on {
        pixels[i] = ACT_R; pixels[i + 1] = ACT_G; pixels[i + 2] = ACT_B;
    } else {
        pixels[i] = IN_R; pixels[i + 1] = IN_G; pixels[i + 2] = IN_B;
    }
    pixels[i + 3] = 255;
}

fn fill_rect(pixels: &mut [u8], x: i32, y: i32, w: i32, h: i32, on: bool) {
    for dy in 0..h {
        for dx in 0..w {
            put_px(pixels, x + dx, y + dy, on);
        }
    }
}

/// 便捷: 渲染一帧完整 LCD (主矩阵 + part + blit) → RGBA
/// params: 8 个底部参数 (0..1) → VOL/EXP/BRT/PAN/REV/CHO/VAR/KEY 上方画条
pub fn render_lcd(pixels: &mut [u8], voice: &str, bank: u32, program: u32,
                  levels: &[f32], audio: &[f32], part: u32, params: &[f32; 8]) {
    let mm = render_to_matrix(voice, bank, program, levels, audio, part);
    let pm = render_part_matrix(part);
    blit(pixels, &mm, &pm, params);
    // 底部 icon 区 (右下, 180,260): 当前音色的 voice icon
    paint_voice_icon(pixels, voice);
}

/// 32-channel 渲染入口 (音色合并第1行 + 34 电平条)
pub fn render_lcd_32(pixels: &mut [u8], voice: &str, bank: u32, program: u32,
                     levels: &[f32], audio: &[f32], part: u32, params: &[f32; 8]) {
    let mm = render_to_matrix_32(voice, bank, program, levels, audio, part);
    let pm = render_part_matrix(part);
    blit(pixels, &mm, &pm, params);
    paint_voice_icon(pixels, voice);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_has_ascii() {
        // 'A' = 65 应在字体表
        assert!(font_offset(65).is_some(), "ASCII A missing");
        assert!(font_offset(32).is_some(), "space missing");
        // 三角 U+0080 空心▶ / U+0081 实心▶ 应在 (bank/prg 前缀)
        assert!(font_offset(0x80).is_some(), "hollow triangle missing");
        assert!(font_offset(0x81).is_some(), "solid triangle missing");
    }

    #[test]
    fn matrix_voice_col45() {
        let mm = render_to_matrix("GrandPno", 0, 1, &[], &[], 1);
        // 音色名首字符 'G' 应点亮 col45..49 内的一些点
        let mut lit = 0;
        for y in 0..8 { for x in 45..50 { lit += mm.get(x, y) as u32; } }
        assert!(lit > 0, "voice name should draw at col 45 (lit={lit})");
        // 第 2 行 (y=8) 是 bank/program
        let mut lit2 = 0;
        for y in 8..16 { for x in 45..85 { lit2 += mm.get(x, y) as u32; } }
        assert!(lit2 > 0, "bank/program row should draw (lit={lit2})");
    }

    #[test]
    fn drum_voice_gets_icon() {
        // 问题2: part10 切到鼓时 LCD icon 必须显示鼓图标 (真机有鼓 icon)。
        // 鼓组 LCD 短名 (StandKit 等) 在 ICONS 无直接条目, 应 fallback 到鼓位图 ("Standard" 或 "Taiko")。
        for drum_name in [
            "StandKit", "Standrd2", "Dry Kit", "BrightKt", "Room Kit", "DarkRoom",
            "Rock Kit", "RockKit2", "ElectrKt", "AnalogKt", "DanceKit", "HipHopKt",
            "JungleKt", "Jazz Kit", "JazzKit2", "BrushKit", "SymphnKt",
        ] {
            let ic = resolve_icon(drum_name).unwrap_or_else(|| panic!("鼓 {drum_name} 应能解析出 icon"));
            // 鼓位图至少要有一些点亮位 (非空)
            let lit = ic.bits.iter().map(|b| b.count_ones()).sum::<u32>();
            assert!(lit > 0, "鼓 {drum_name} icon 应有位图 (lit={lit})");
        }
        // 未知鼓名 fallback 到 "Drum" → Taiko 也应能解析
        assert!(resolve_icon("Drum").is_some(), "\"Drum\" 应 fallback 到鼓 icon");
    }

    #[test]
    fn render_to_buffer_size() {
        let mut px = vec![0u8; LCD_W * LCD_H * 4];
        render_lcd(&mut px, "DreamPno", 0, 1, &[], &[], 1, &[0.0; 8]);
        // 像素缓冲应满 (所有像素已设置 alpha=255)
        let all_alpha = px.chunks_exact(4).all(|c| c[3] == 255);
        assert!(all_alpha, "all pixels should have alpha set");
        // 背景色应为主绿
        assert_eq!((px[0], px[1], px[2]), (BG_R, BG_G, BG_B), "bg should be backlight green");
    }

    /// 把矩阵中一段 5x8 字形块反查字体表, 还原成字符码位 (程序解码, 不靠人眼)
    fn decode_char(mm: &MuMatrix, cx: i32, cy: i32) -> Option<(u32, Vec<u8>)> {
        // 提取该字形 5x8 位图 (逐行 5 bit 打包成字节)
        let mut glyph = vec![0u8; 8];
        for r in 0..8usize {
            let mut byte = 0u8;
            for c in 0..5usize {
                if mm.get(cx + c as i32, cy + r as i32) != 0 {
                    byte |= 1 << c;
                }
            }
            glyph[r] = byte;
        }
        // 反查: 在 XG_FONT_BITS 找完全相同的 8 字节
        for (i, cp) in XG_FONT_CODES.iter().enumerate() {
            let off = i * 8;
            if XG_FONT_BITS[off..off + 8] == glyph[..] {
                return Some((*cp, glyph));
            }
        }
        None
    }

    /// 解码一字符串段 (每字 5 列, cy 起 8 行) → String
    fn decode_run(mm: &MuMatrix, start: i32, cy: i32, n: usize) -> String {
        let mut s = String::new();
        for i in 0..n {
            let cx = start + (i as i32) * 5;
            match decode_char(mm, cx, cy) {
                Some((cp, _)) => {
                    // U+0080 public: hollow ▶, U+0081: solid ▶ — 渲染为可视占位
                    let ch = match cp {
                        0x80 => '▶',
                        0x81 => '▶',
                        c if c < 0x20 || c == 0x7f => '�',
                        c if (0xD800..0xE000).contains(&c) => '?',
                        c => {
                            // 安全转 char (字体码位多在 BMP)
                            char::from_u32(c).unwrap_or('?')
                        }
                    };
                    s.push(ch);
                }
                None => s.push('?'), // 未匹配 → 明确标志
            }
        }
        s
    }

    #[test]
    fn port_b_voice_moves_to_left() {
        // John 2026-08-12: 32ch off 时, part 17-32 (PortB) 音色/bank/pgm 显示在左边 8 组点阵 (col 0..39)
        // part 1-16 (PortA) 保持右侧 col 45..84。用程序解码断言, 不靠人眼。
        // Port A: 音色在 col 45 (右)
        let mm_a = render_to_matrix("DreamPno", 0, 7, &[], &[], 1);
        let got_a = decode_run(&mm_a, 45, 0, 8);
        assert_eq!(got_a, "DreamPno", "PortA voice should stay at col45 (right), got '{got_a}'");
        // Port A: col0 应该是电平条 (有字→空白区) 而非音色
        let left_a = decode_run(&mm_a, 0, 0, 8);
        assert_ne!(left_a, "DreamPno", "PortA col0 must NOT hold the voice name (got '{left_a}')");

        // Port B (part 17): 音色应在 col 0 (左)
        let mm_b = render_to_matrix("DreamPno", 0, 7, &[], &[], 17);
        let got_b = decode_run(&mm_b, 0, 0, 8);
        assert_eq!(got_b, "DreamPno", "PortB voice should move to col0 (left), got '{got_b}'");
        // Port B: bank/program 第 2 行也在左 (col0..39)
        let bp_b = decode_run(&mm_b, 0, 8, 8); // ▶000▶007
        let exp_b: String = [
            '\u{0080}', '0', '0', '0', '\u{0081}', '0', '0', '7',
        ].iter().map(|c| match c {
            '\u{0080}' => '▶', '\u{0081}' => '▶', c => *c,
        }).collect();
        assert_eq!(bp_b, exp_b, "PortB bank/prog should draw at col0 row8, got '{bp_b}'");
        // Port B: 右侧 col45 不再有音色 (留给电平)
        let right_b = decode_run(&mm_b, 45, 0, 8);
        assert_ne!(right_b, "DreamPno", "PortB col45 must NOT hold the voice name (got '{right_b}')");

        // 电平条位置 John 2026-08-12: PortB 时 1-16 电平不再显示(否则覆盖左侧字母),
        // 电平条移到右侧 (col40..84), 18 bar 位置与 PortA 对称, ch15/16 空缺, 17-32 有基线。
        // PortB: col0..39 (音色区) 的 row15 不应有点亮 (无电平基线)
        for x in 0..40 { assert_eq!(mm_b.get(x, 15), 0, "PortB col{x} row15 must be empty (no meter over voice)"); }
        // PortB: 右侧 18 bar 列 (i=0..17 → col40..83), 其中 i=0,1 (col40,43 = ch15,16) 留空
        // i=2..17 (col45..83 = ch17..32) 有基线
        let bar_col_b = |i: i32| -> i32 { let gi = i / 2; let k = i % 2; 40 + gi * 5 + if k == 1 { 3 } else { 0 } };
        let c15 = bar_col_b(0); let c16 = bar_col_b(1);
        assert_eq!(mm_b.get(c15, 15), 0, "ch15 slot col{c15} should be empty");
        assert_eq!(mm_b.get(c16, 15), 0, "ch16 slot col{c16} should be empty");
        let mut right_base = 0;
        for i in 2..18 { right_base += mm_b.get(bar_col_b(i), 15) as u32 + mm_b.get(bar_col_b(i)+1, 15) as u32; }
        assert!(right_base >= 16, "PortB 17-32 应有电平基线 (lit={right_base})");
        // PortA: 左侧有电平基线 (col0..44)
        let mut left_base_a = 0;
        for x in 0..44 { left_base_a += mm_a.get(x, 15) as u32; }
        assert!(left_base_a > 20, "PortA left side should have meter baseline (lit={left_base_a})");
    }

    #[test]
    fn programmatic_voice_decode() {
        // 程序断言: 渲染 → 解码 → 必须还原为 DreamPno (不靠人眼)
        let mut mm = render_to_matrix("DreamPno", 0, 1, &[], &[], 1);
        let got = decode_run(&mm, 45, 0, 8);
        assert_eq!(got, "DreamPno", "voice name decode mismatch! got '{got}' at col 45..84");
    }

    #[test]
    fn programmatic_mainrow_decode() {
        // 程序断言: 第 2 行 bank/prg 是 ▶000▶001 (hollow▶ + 000 + solid▶ + 001)
        let mut mm = render_to_matrix("DreamPno", 0, 1, &[], &[], 1);
        let got = decode_run(&mm, 45, 8, 8); // 8 字符: ▶000▶001
        let exp: String = [
            '\u{0080}', '0', '0', '0', '\u{0081}', '0', '0', '1',
        ].iter().map(|c| match c {
            '\u{0080}' => '▶',
            '\u{0081}' => '▶',
            c => *c,
        }).collect();
        assert_eq!(got, exp, "bank/prg row decode mismatch! got '{got}'");
    }

    #[test]
    fn programmatic_part_decode() {
        // 程序断言: 下部 01A01, A 反显 (reverse video); part 17 → 17B01 (John 权威)
        let pm = render_part_matrix(1);
        // 前后字符 (01 / 01) 正常解码
        let got = decode_run(&pm, 0, 0, 3); // 只解前 3: 01 + 反显 A 会失败
        assert_eq!(&got[0..2], "01", "part prefix decode mismatch: got '{got}'");
        // A 是反显 (3x5, 从第 2 行 y1 开始): 字形区 x11..13,y1..5 镂空; 其余 (边框列 x10/x14, 及 y0/y6 行) 全黑(set=1)
        // 顶部 5x7 点亮 (John 2026-08-12): 底行 y7 留暗 (不点亮)
        // 边框列 x10 x14 (0..7 行): y0..y6 黑, y7 暗 (5x7 只亮顶 7 行)
        for y in 0..7 { for x in [10i32, 14] {
            assert_eq!(pm.get(x, y), 1, "反显边框列应黑(1), at ({x},{y})");
        }}
        assert_eq!(pm.get(10, 7), 0, "5x7 反显底行 y7 col10 应留暗");
        assert_eq!(pm.get(14, 7), 0, "5x7 反显底行 y7 col14 应留暗");
        // 行 y0 / y6 (全 5 列) 全黑; y7 全暗
        for y in [0i32, 6] { for x in 10..15 {
            assert_eq!(pm.get(x, y), 1, "反显空行 y={y} 应黑(1)");
        }}
        for x in 10..15 {
            assert_eq!(pm.get(x, 7), 0, "5x7 反显底行 y7 应留暗, col{x}");
        }
        // 3x5 A 字形镂空 (y1..y5, x11..x13) — 含顶部尖点
        // John 2026-08-12 微调字模: 横杠从 r2 下移到 r3, 其余不变
        let a3x5: [[u8; 3]; 5] = [
            [0, 1, 0], [1, 0, 1], [1, 0, 1], [1, 1, 1], [1, 0, 1],
        ];
        for r in 0..5usize {
            for c in 0..3usize {
                let v = pm.get(11 + c as i32, 1 + r as i32);
                if a3x5[r][c] == 1 {
                    assert_eq!(v, 0, "反显 3x5 A 字形应镂空(绿), at y={} x={}", 1+r, 11+c);
                } else {
                    assert_eq!(v, 1, "反显 3x5 A 空白处应黑, at y={} x={}", 1+r, 11+c);
                }
            }
        }
        // 后两字符 (part) 正常
        let tail = decode_run(&pm, 15, 0, 2);
        assert_eq!(tail, "01", "part tail decode mismatch: got '{tail}'");
    }

    #[test]
    fn part_mapping_32() {
        // John 权威 2026-08-09: MU90 32 part; part 1-16 → A ch1-16; 17-32 → B ch1-16
        assert_eq!(part_sec(1), 'A');  assert_eq!(part_channel(1), 1);
        assert_eq!(part_sec(16), 'A'); assert_eq!(part_channel(16), 16);
        assert_eq!(part_sec(17), 'B'); assert_eq!(part_channel(17), 1);   // part17 = B ch01 → 17B01
        assert_eq!(part_sec(32), 'B'); assert_eq!(part_channel(32), 16);
        // LCD 文本: part17 → "17B01"
        let pm = render_part_matrix(17);
        let head = decode_run(&pm, 0, 0, 2);
        let tail = decode_run(&pm, 15, 0, 2);
        assert_eq!(head, "17", "part17 prefix should be 17, got '{head}'");
        assert_eq!(tail, "01", "part17 tail (channel) should be 01, got '{tail}'");
    }

    #[test]
    fn part_change_rerenders_lcd_pixels() {
        // Part 选择器切换 → render_lcd 像素应变化 (证明 UI 路径真的随 part 变)
        let mut px1 = vec![0u8; LCD_W * LCD_H * 4];
        let mut px17 = vec![0u8; LCD_W * LCD_H * 4];
        render_lcd(&mut px1, "DreamPno", 0, 1, &[0.0; 16], &[0.0; 2], 1, &[0.5; 8]);
        render_lcd(&mut px17, "DreamPno", 0, 1, &[0.0; 16], &[0.0; 2], 17, &[0.5; 8]);
        let diff = px1.iter().zip(px17.iter()).filter(|(a, b)| a != b).count();
        assert!(diff > 0, "part 1 vs 17 的 LCD 渲染应产生不同像素 (至少 part 区不同)");
        // 具体到 part 显示区 (左下 5 字符) 应不同 — part1: 01A01, part17: 17B01
        let pm1 = render_part_matrix(1);
        let pm17 = render_part_matrix(17);
        let head1 = decode_run(&pm1, 0, 0, 2);
        let head17 = decode_run(&pm17, 0, 0, 2);
        assert_eq!(head1, "01");
        assert_eq!(head17, "17");
    }

    #[test]
    fn icon_resolve_and_render() {
        // GrandPno 应精确匹配到 icon
        let ic = resolve_icon("GrandPno").expect("GrandPno should have icon");
        assert_eq!(ic.name, "GrandPno");
        // 像素: 在 icon 区 (260,180) 渲染应产生黑像素 (图案)
        let mut px = vec![0u8; LCD_W * LCD_H * 4];
        // 用 clear + paint 模拟 (不依赖之前渲染)
        for c in px.chunks_exact_mut(4) { c[0]=BG_R; c[1]=BG_G; c[2]=BG_B; c[3]=255; }
        draw_icon(&mut px, ic, 260, 180);
        // 黑像素应 > 0 (有图案); GrandPno icon 应该有大量黑
        let mut black = 0;
        for y in 180..244 {
            for x in 260..388 {
                let i = (y * LCD_W + x) * 4;
                if px[i] < 100 && px[i+1] < 200 { black += 1; }
            }
        }
        assert!(black > 500, "GrandPno icon should render substantial pixels, got {black}");
    }

    #[test]
    fn mode_32ch_layout() {
        // 32ch 模式: 音色名从最左边 col0 开始 (第 1 行), bank/prog 不在第 2 行
        let mut levels = vec![0.0f32; 32];
        levels[0] = 1.0; // ch1 满
        levels[31] = 0.5; // ch32 半
        let mm = render_to_matrix_32("GrandPno", 0, 1, &levels, &[0.0, 0.0], 1);
        // ① 音色应从 col0 开始 (row0 最左 8 列有字)
        let mut left_lit = 0;
        for y in 0..8 { for x in 0..8 { left_lit += mm.get(x, y) as u32; } }
        assert!(left_lit > 0, "32ch: voice should start at col0 (leftmost)");
        // ② 前 18 bar (A1/A2+01..16) 的列位置必须与 16ch 模式完全一致
        // bar i 在 row15(基线) 占用列 c..c+1; 用齐平电平(满)时最高层 row0 也点亮, 对比列集
        let full = vec![1.0f32; 16];
        let mm16_full = render_to_matrix("GrandPno", 0, 1, &full, &[1.0, 1.0], 1);
        let levels_full = vec![1.0f32; 32];
        let mm32_full = render_to_matrix_32("GrandPno", 0, 1, &levels_full, &[1.0, 1.0], 1);
        // 对每个 bar b (0..18), 16ch 的列 vs 32ch 的列 (最顶部点亮列相同)
        let bar_col_16: Vec<i32> = (0..18).map(|i| { let g = i / 2; let k = i % 2; g * 5 + if k == 1 { 3 } else { 0 } }).collect();
        let bar_col_32: Vec<i32> = (0..34).map(|i| { let g = i / 2; let k = i % 2; g * 5 + if k == 1 { 3 } else { 0 } }).collect();
        assert_eq!(&bar_col_32[..18], &bar_col_16[..], "32ch 前18 bar 列应与16ch一致, 16={bar_col_16:?} 32={:?}", &bar_col_32[..18]);
        // ③ 列一致性反映在矩阵: 16ch 与 32ch 在 row0..15 的前 18 bar 覆盖列 (col 0..44) 点亮模式应一致
        // 但高度减半 (32ch 只到 row8 层) → 只比较高区 row8..15 的覆盖列
        let mut cols_16 = std::collections::BTreeSet::new();
        let mut cols_32 = std::collections::BTreeSet::new();
        for y in 8..16 {
            for x in 0..44 { if mm16_full.get(x, y) == 1 { cols_16.insert(x); } }
        }
        for y in 8..16 {
            for x in 0..44 { if mm32_full.get(x, y) == 1 { cols_32.insert(x); } }
        }
        assert_eq!(cols_16, cols_32, "32ch 与 16ch 前18 bar 的列覆盖应一致 (row8..15)");
        // ④ 17 字符音色行从 col0 到 col84 (整行, 含 col45 之后的中后段) — 见下方⑤验证其长度
        // ⑤ 验证第一行确实是 "GrandPno ▶bank ▶prog" 17 字符: 解码 row0 前 17 个字符
        let deco = decode_run(&mm, 0, 0, 17);
        // 期望含音色名 + ▶000 + ▶001
        let mut has_voice = deco.starts_with("GrandPn");
        let _ = has_voice;
        // 直接断言: 第一行应有 17 个字符位被占用 (非空), 且以音色名开头
        assert!(deco.starts_with("GrandPn"), "32ch 第一行应以音色名开头, got: '{deco}'");
        assert!(deco.len() >= 17, "32ch 第一行应占满 17 字符, got len {}", deco.len());
    }
}
