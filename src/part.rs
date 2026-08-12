//! Part 状态唯一数据源 (PartState / SystemFx)
//!
//! 设计: reference/part-single-source-design-2026-08-12.md
//!
//! MU90 模型: 32 multi part 各含「音色(voice/msb/lsb/prog) + 混音参数(VOL..KEY 8 条)」。
//! Part 1-16 ← Port A ch1-16; Part 17-32 ← Port B ch1-16。
//! Rev/Cho/Var 效果器类型 = 全局系统效果 (SystemFx); part 只有 send 量。
//! Cutoff/Reso 是音色编辑参数 (不进 part, 留在面板音色编辑区)。

/// 8 条混音参数的索引常量 (与 LCD 底部标签 VOL EXP BRT PAN REV CHO VAR KEY 对齐)
#[repr(usize)]
pub enum P {
    Volume = 0,   // VOL
    Exp = 1,      // EXP (expression)
    Bright = 2,   // BRT
    Pan = 3,      // PAN
    Reverb = 4,   // REV (send)
    Chorus = 5,   // CHO (send)
    Variation = 6,// VAR (send)
    Key = 7,      // KEY
}

pub const N_PARAMS: usize = 8;

/// 单个 part 的完整状态 (1..=32)
#[derive(Clone, Debug, PartialEq)]
pub struct PartState {
    pub voice: String,     // 音色名 (LCD/矩阵显示, 如 "GrandPno")
    pub msb: u8,           // Bank Select MSB (CC0)
    pub lsb: u8,           // Bank Select LSB (CC32) — LCD 显示 bank 用 LSB
    pub prog: u8,          // Program 0-based (PC), LCD 显示 prog+1
    /// 8 条混音参数, 存原始控制目标值: VOL 0..127, EXP 0..127, BRT 0..127,
    /// PAN 0..127 (64=center), REV/CHO/VAR send 0..127, KEY 0..127 (64=0 shift)
    pub params: [f32; N_PARAMS],
}

impl PartState {
    pub fn default_voice(msb: u8, lsb: u8, prog: u8, name: &str) -> Self {
        Self {
            voice: name.to_string(),
            msb,
            lsb,
            prog,
            params: [
                100.0, // VOL (真机初值 ~100)
                127.0, // EXP
                0.0,   // BRT (初值 0)
                64.0,  // PAN center
                0.0,   // REV send 0
                0.0,   // CHO send 0
                0.0,   // VAR send 0
                64.0,  // KEY 0 shift
            ],
        }
    }

    pub fn set_voice(&mut self, msb: u8, lsb: u8, prog0: u8, name: String) {
        self.msb = msb;
        self.lsb = lsb;
        self.prog = prog0;
        self.voice = name;
    }
}

/// 全局系统效果 (Effect Bank 类型, 非 per-part)
#[derive(Clone, Debug, PartialEq)]
pub struct SystemFx {
    pub rev_type: String, // "Hall"
    pub cho_type: String, // "Chorus1"
    pub var_type: String, // "off"
}

impl Default for SystemFx {
    fn default() -> Self {
        Self {
            rev_type: "Hall".to_string(),
            cho_type: "Chorus1".to_string(),
            var_type: "off".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_default_is_xg_init_voice() {
        let p = PartState::default_voice(0, 0, 0, "GrandPno");
        assert_eq!(p.voice, "GrandPno");
        assert_eq!(p.msb, 0);
        assert_eq!(p.lsb, 0);
        assert_eq!(p.prog, 0);
        assert_eq!(p.params[P::Volume as usize], 100.0);
        assert_eq!(p.params[P::Pan as usize], 64.0);
    }

    #[test]
    fn set_voice_updates_all_axes() {
        let mut p = PartState::default_voice(0, 0, 0, "GrandPno");
        p.set_voice(0, 0, 58, "Tuba".to_string());
        assert_eq!(p.voice, "Tuba");
        assert_eq!(p.prog, 58);
        assert_eq!(p.params[P::Volume as usize], 100.0);
    }

    #[test]
    fn sys_fx_default() {
        let f = SystemFx::default();
        assert_eq!(f.rev_type, "Hall");
        assert_eq!(f.cho_type, "Chorus1");
        assert_eq!(f.var_type, "off");
    }
}
