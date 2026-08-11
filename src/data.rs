//! 数据字典层 — XG 音色表 / 效果表 / SysEx 地址映射
//! 对应 PLAN Phase 1 + 原 JS `src/data/*.json`(Rust 版)
//! 数据是跨语言 JSON, serde 直接加载, 无需重做解析。

use super::device::Device;
use serde::Deserialize;
use std::collections::HashMap;

/// MU90 voices JSON 包装格式 (含元数据)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Mu90VoicesFile {
    voices: Vec<Voice>,
    device: Option<String>,
    source: Option<String>,
}

/// 单个 XG 音色条目(来自 midi-db bank/xg.tsv, 1599 条)
#[derive(Debug, Clone, Deserialize)]
pub struct Voice {
    /// Bank Select MSB(0 / 48 = SFX / 64 = SFX kit / 126 / 127 = XG)
    pub msb: u8,
    /// Program Change(0-127)
    pub prg: u8,
    /// Bank Select LSB(变体号)
    pub lsb: u8,
    /// 音色名(如 GrandPno)
    pub name: String,
    /// 元素数(number of elements) —— 原表约 24% 条缺失(值为 null)
    pub elc: Option<u8>,
    /// 层级/优先级 —— 同理可为 null
    pub lvl: Option<u8>,
}

/// 数据字典: 加载后提供音色查找
pub struct VoiceBank {
    pub voices: Vec<Voice>,
    /// msb -> 该 bank 的索引范围(供快速筛选)
    msb_index: HashMap<u8, Vec<usize>>,
}

impl VoiceBank {
    /// 从 JSON 文件加载音色表 (native 用; wasm 无文件 IO)
    pub fn load(path: &str) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {e}"))?;
        Self::from_json(&json)
    }

    /// MU90 官方权威音色表 (来自 yamaha_mu90_voice_jp.txt, 解析器抽取)
    /// LCD/快捷菜单以此为准, 与真机一致。包装: {"voices":[...], ...}
    pub fn embedded_mu90() -> Result<Self, String> {
        let json = include_str!("data/mu90_voices.json");
        let wrapped: Mu90VoicesFile = serde_json::from_str(json)
            .map_err(|e| format!("MU90 JSON 解析失败: {e}"))?;
        Self::from_direct(wrapped.voices)
    }

    /// 从裸 voices 数组构建 (避免重复解析)
    fn from_direct(voices: Vec<Voice>) -> Result<Self, String> {
        if voices.is_empty() {
            return Err("音色表为空".into());
        }
        let mut msb_index: HashMap<u8, Vec<usize>> = HashMap::new();
        for (i, v) in voices.iter().enumerate() {
            msb_index.entry(v.msb).or_default().push(i);
        }
        Ok(Self { voices, msb_index })
    }

    /// 从 JSON 字符串加载(便于测试注入)
    pub fn from_json(json: &str) -> Result<Self, String> {
        let voices: Vec<Voice> =
            serde_json::from_str(json).map_err(|e| format!("JSON 解析失败: {e}"))?;
        Self::from_direct(voices)
    }

    /// 按 MSB 列出所有音色
    pub fn by_msb(&self, msb: u8) -> Vec<&Voice> {
        self.msb_index
            .get(&msb)
            .map(|idxs| idxs.iter().map(|&i| &self.voices[i]).collect())
            .unwrap_or_default()
    }

    /// 精确查找: MSB + PRG + LSB
    pub fn find(&self, msb: u8, prg: u8, lsb: u8) -> Option<&Voice> {
        if lsb > 127 {
            return None; // 7-bit bank select LSB 无法表达 >127 的 bank
        }
        self.msb_index
            .get(&msb)?
            .iter()
            .map(|&i| &self.voices[i])
            .find(|v| v.prg == prg && v.lsb == lsb)
    }

    /// XG 常规旋律音色区(MSB=0, 标准 bank, 用 prg 查名)
    /// 注意: 不要用 MSB=127 —— 那是 XG 鼓组区
    pub const XG_MSB: u8 = 0;

    /// 当前 (msb, prg) 下所有有效的 LSB 变体 (去重+升序) — 用于滑块"有效取值范围"
    /// 只返回 ≤127 的合法 7-bit bank select LSB。表里 lsb>127(如 128..152) 是
    /// XG Level2+ 音源的扩展 bank, MIDI CC32 无法表达(7-bit), MU90(XG Level1) 也选不到 → 过滤掉
    pub fn lsb_variants(&self, msb: u8, prg: u8) -> Vec<u8> {
        let mut v: Vec<u8> = self
            .msb_index
            .get(&msb)
            .map(|idxs| {
                idxs.iter()
                    .map(|&i| &self.voices[i])
                    .filter(|v| v.prg == prg && v.lsb <= 127)
                    .map(|v| v.lsb)
                    .collect()
            })
            .unwrap_or_default();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// 所有有效的 MSB 区 (去重+升序) — Bank 滑块只在这些值间步进
    pub fn msb_values(&self) -> Vec<u8> {
        let mut v: Vec<u8> = self.msb_index.keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// 给定设备下所有可发声的音色 (去重+按 msb/prg/lsb 排序) — 快捷选择菜单数据源。
    /// 只含 lsb<=127 且 msb 在该设备支持集内。
    pub fn voices_for_device(&self, device: Device) -> Vec<&Voice> {
        let mut v: Vec<&Voice> = self
            .voices
            .iter()
            .filter(|vo| vo.lsb <= 127 && device.supports_msb(vo.msb))
            .collect();
        v.sort_by(|a, b| {
            a.msb
                .cmp(&b.msb)
                .then(a.prg.cmp(&b.prg))
                .then(a.lsb.cmp(&b.lsb))
        });
        v
    }

    /// 当前 msb 区下所有有效 prg (去重+升序) — PC 滑块只在这些值间步进
    pub fn prg_values(&self, msb: u8) -> Vec<u8> {
        let mut v: Vec<u8> = self
            .msb_index
            .get(&msb)
            .map(|idxs| {
                idxs.iter()
                    .map(|&i| &self.voices[i])
                    .map(|v| v.prg)
                    .collect()
            })
            .unwrap_or_default();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// 按 prg 查 XG 旋律音色(MSB=0 标准区)
    pub fn xg_by_prg(&self, prg: u8) -> Option<&Voice> {
        self.find(Self::XG_MSB, prg, 0)
    }
}

/// MU90 鼓组 (msb=127) 的 LCD 8 字符显示短名, 按 program 0-based 索引。
/// **prg0=0 (Standard Kit) → "StandKit" 已 John 真机确认** (MU90 LCD 显示 StandKit, 非 XG 的 "Standard")。
/// 其余按 XG 标准 8 字符短名 (soundlist 第二列同款), 待 John 真机逐个复核。
pub fn drum_display_name(prg0: u8) -> &'static str {
    match prg0 {
        0 => "StandKit",      // Standard Kit (John 真机确认)
        1 => "Standrd2",      // Standard Kit 2
        2 => "Dry Kit",       // Dry Kit
        3 => "BrightKt",      // Bright Kit
        8 => "Room Kit",      // Room Kit
        9 => "DarkRoom",      // Dark Room Kit
        16 => "Rock Kit",     // Rock Kit
        17 => "RockKit2",     // Rock Kit 2
        24 => "ElectrKt",     // Electro Kit
        25 => "AnalogKt",     // Analog Kit
        26 => "AnalgKt2",     // Analog Kit 2
        27 => "DanceKit",     // Dance Kit
        28 => "HipHopKt",     // Hip Hop Kit
        29 => "JungleKt",     // Jungle Kit
        32 => "Jazz Kit",     // Jazz Kit
        33 => "JazzKit2",     // Jazz Kit 2
        40 => "BrushKit",     // Brush Kit
        48 => "SymphnKt",     // Symphony Kit
        _ => "Drum",          // 未知 (非 MU90 18 鼓组)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drum_display_names_locked() {
        // John 2026-08-09: MU90 LCD 显示 StandKit (真机), 非 XG 通用 "Standard"
        assert_eq!(drum_display_name(0), "StandKit");
        assert_eq!(drum_display_name(48), "SymphnKt");
        assert_eq!(drum_display_name(1), "Standrd2");
        assert_eq!(drum_display_name(16), "Rock Kit");
    }
    fn load_xg_voices_json() {
        // 指向仓库实际的 JSON(相对 Cargo.toml 的 tests 跑目录)
        let path = "../src/data/xg_voices.json";
        let bank = VoiceBank::load(path).expect("应能加载 xg_voices.json(含 24% null 字段)");
        assert_eq!(bank.voices.len(), 1599, "XG 音色表应有 1599 条");
        // 验证 null 字段被解析为 None
        let with_null = bank.voices.iter().filter(|v| v.elc.is_none()).count();
        assert!(with_null > 0, "应有部分音色 elc 为 None(null)");
    }

    #[test]
    fn embedded_mu90_loads_authoritative() {
        // wasm 环境无文件 IO → 必须走 embedded_mu90 (include_str 内嵌权威表)
        let bank = VoiceBank::embedded_mu90().expect("embedded_mu90() 应能内嵌加载 MU90 权威表");
        assert!(bank.voices.len() >= 250, "MU90 权威表应有 250+ 条, got {}", bank.voices.len());
        // 抽查: GrandPno 在, Glocken+ (msb48) 不在
        assert_eq!(bank.xg_by_prg(0).unwrap().name, "GrandPno");
        assert!(bank.find(48, 21, 8).is_none(), "MU90 权威表不应有 Glocken+ (msb48)");
        // bank0 pc1 变体 (MU90 权威): 0 GrandPno / 1 GrndPnoK / 18 MelloGrP / 40 PianoStr / 41 Dream
        let gp_lsbs = bank.lsb_variants(0, 0);
        assert_eq!(gp_lsbs, vec![0u8, 1, 18, 40, 41], "MU90 bank0 pc1 变体应为 0/1/18/40/41");
    }

    #[test]
    fn find_grand_piano() {
        // GrandPno = MSB 0(XG 标准区), prg 0, lsb 0
        let json = r#"[
          {"msb":0,"prg":0,"lsb":0,"name":"GrandPno","elc":0,"lvl":0}
        ]"#;
        let bank = VoiceBank::from_json(json).unwrap();
        let v = bank.xg_by_prg(0).expect("prg0 应有音色");
        assert_eq!(v.name, "GrandPno");
        assert_eq!(v.msb, VoiceBank::XG_MSB);
        assert_eq!(v.msb, 0);
    }

    #[test]
    fn by_msb_filters() {
        let json = r#"[
          {"msb":127,"prg":0,"lsb":0,"name":"A","elc":0,"lvl":0},
          {"msb":127,"prg":1,"lsb":0,"name":"B","elc":0,"lvl":0},
          {"msb":48,"prg":0,"lsb":0,"name":"SFX","elc":0,"lvl":0}
        ]"#;
        let bank = VoiceBank::from_json(json).unwrap();
        assert_eq!(bank.by_msb(127).len(), 2);
        assert_eq!(bank.by_msb(48).len(), 1);
        assert_eq!(bank.find(127, 1, 0).unwrap().name, "B");
    }

    #[test]
    fn empty_bank_is_error() {
        assert!(VoiceBank::from_json("[]").is_err());
    }

    #[test]
    fn lsb_variants_filters_over_127() {
        // 回归: msb=0 prg=21 (Acordion) 的变体原本含 128..132 (XG Level2+ 扩展 bank)
        // MIDI CC32 只能 7-bit, MU90 也选不到 → 必须过滤只留 ≤127
        // MU90 权威表: Acordion (bank0/pc22) 变体 = lsb 0 (Acordion) + lsb 32 (AccordIt, 来自 bank32 区)
        let bank = VoiceBank::embedded_mu90().unwrap();
        let v = bank.lsb_variants(0, 21);
        assert!(!v.is_empty(), "Acordion 应有变体");
        assert!(v.iter().all(|&l| l <= 127), "所有 lsb 变体必须 ≤127, got {v:?}");
        assert_eq!(v, vec![0, 32], "MU90 Acordion 变体应 = [0, 32] (lsb32=AccordIt)");
        // find: lsb=32 命中 AccordIt (bank32 区 pc22)
        assert_eq!(bank.find(0, 21, 32).map(|x| x.name.as_str()), Some("AccordIt"));
        assert!(bank.find(0, 21, 0).is_some(), "lsb=0 应正常命中 Acordion");
    }

    #[test]
    fn voices_for_device_filters_mu90() {
        // MU90 快捷菜单数据源: 只含 msb ∈ {0,64,127} 且 lsb<=127
        let bank = VoiceBank::embedded_mu90().unwrap();
        let v = bank.voices_for_device(Device::Mu90);
        assert!(!v.is_empty(), "MU90 应有不少音色");
        for vo in &v {
            assert!(Device::Mu90.supports_msb(vo.msb), "msb={} 不在 MU90 支持集", vo.msb);
            assert!(vo.lsb <= 127, "lsb={} 越界", vo.lsb);
        }
        // 关键: Glocken+ (msb=48) 必须被排除 (MU90 上 Silence)
        assert!(
            !v.iter().any(|vo| vo.name == "Glocken+"),
            "Glocken+ (msb=48) 不应出现在 MU90 菜单里"
        );
        // 关键: Acordion (msb=0 lsb=0 p22) 必须在
        assert!(
            v.iter().any(|vo| vo.name == "Acordion"),
            "Acordion (msb=0) 应在 MU90 菜单里"
        );
        // 检查 e.g. GrandPno 应在
        assert!(v.iter().any(|vo| vo.name == "GrandPno"));
        // 权威名锁定: SFX 名和 Drum 名必须用 MU90 官方手册名(xg 通用表名不符真机, John 2026-08-09 报告)
        let dog = bank.find(64, 48, 0).expect("SFX Dog (msb64 prg49) 应存在");
        assert_eq!(dog.name, "Dog", "MU90 LCD SFX 应显示 'Dog' 非 'Dog Woof'");
        let stdkit = bank.find(127, 0, 0).expect("Standard Kit (msb127 prg1) 应存在");
        assert_eq!(stdkit.name, "Standard Kit", "MU90 鼓组 1 应 'Standard Kit' 非 'Standard'");
        let sym = bank.find(127, 48, 0).expect("Symphony Kit (msb127 prg49) 应存在");
        assert_eq!(sym.name, "Symphony Kit", "MU90 鼓组 49 应 'Symphony Kit' 非 'SymphnKt'");
    }

    #[test]
    fn mu90_excludes_pwr_keel_family() {
        // 用户报告的 LCD 不一致 (2026-08-09): bank72/pc40 真机 MU90 = Silence/SynBass2,
        // Octavia 合并表却标 Pwr Keel. MU90 权威表必须排除 lsb 74..78 (XG Level2+ 扩展 bass 区,
        // Pwr Keel 家族所在). 注意 lsb 64..73 是 MU90 真实实现的变体(X WireBa/60sEl.P1 等), 不得排除.
        let bank = VoiceBank::embedded_mu90().unwrap();
        // bank0 pc40 基础音色 = SynBass2
        assert_eq!(bank.find(0, 39, 0).map(|v| v.name.as_str()), Some("SynBass2"));
        // bank 64..67 pc40 = X WireBa 家族 (MU90 手册变体页真实存在)
        assert_eq!(bank.find(0, 39, 64).map(|v| v.name.as_str()), Some("X WireBa"));
        assert_eq!(bank.find(0, 39, 65).map(|v| v.name.as_str()), Some("AtkPulse"));
        // Pwr Keel 家族所在 (lsb 74..78) 在 MU90 权威表必须为空
        for lsb in 74..=78u8 {
            assert!(
                bank.find(0, 39, lsb).is_none(),
                "MU90 权威表 bank{lsb}/pc40 不应存在 (Pwr Keel 家族), 但找到了 {:?}",
                bank.find(0, 39, lsb)
            );
        }
        // 若 bank72 真在权威表有 pc40, 应仅在确实有音色时出现 — MU90 官方表该区为空
        assert!(bank.find(48, 21, 8).is_none(), "Glocken+ 也不应在 MU90");
    }
}
