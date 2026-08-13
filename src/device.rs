/// 目标硬件设备。音色快捷选择菜单按此过滤 —— 只显示该设备真实拥有的音色。
/// 权威依据: MU90 官方 Voice List (yamaha_mu90_voice_jp.txt)。
/// - MSB=000: XG 正常音色区 (LSB 携带 bank number)
/// - MSB=064: SFX 区
/// - MSB=126: SFX Kit (SFX Kit 1/2, 打击垫式音效套件)
/// - MSB=127: XG Drum Map (Program 选鼓组)
/// 注意: msb=48 (MU100 Native) 在 MU90 上会报 Silence, 不在支持集内。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    /// Yamaha MU90 (XG Level 1)
    Mu90,
}

impl Device {
    /// 该设备支持的 MSB 区集合 (快捷菜单过滤依据, 来自官方 voice list)
    pub fn supported_msbs(&self) -> &'static [u8] {
        match self {
            Device::Mu90 => &[0, 64, 126, 127],
        }
    }

    /// 该设备是否支持某 MSB
    pub fn supports_msb(&self, msb: u8) -> bool {
        self.supported_msbs().contains(&msb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mu90_supported_msbs_lock() {
        // John 2026-08-13: 分级音色选择器依赖此集过滤。
        // 踩坑: 漏了 126 (SFX Kit) 导致 SFX Kit 不显示。
        let m = Device::Mu90;
        assert!(m.supports_msb(0), "MSB0 XG 音色");
        assert!(m.supports_msb(64), "MSB64 SFX");
        assert!(m.supports_msb(126), "MSB126 SFX Kit (曾漏掉!)");
        assert!(m.supports_msb(127), "MSB127 Drum");
        assert!(!m.supports_msb(48), "MSB48 MU100 native 不支持");
    }
}

