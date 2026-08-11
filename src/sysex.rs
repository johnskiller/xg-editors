//! XG SysEx 组帧引擎 (Rust 版, 移植自 src/sysex/xg.js + sysex_map.json)
//! 纯逻辑, 无硬件依赖 → 全部可用单测/向量验证。
//!
//! 手册权威 (MU90 付表 2-1 / 8. MIDI 数据格式):
//! - Parameter Change (设备号 `1n` = 16-31): F0 43 1n 4C [addr3] [data] F7  — 无校验和, 无字节数
//! - Bulk Dump       (设备号 `0n` = 0-15): F0 43 0n 4C [bb bb] [addr3] [data...] [cs] F7 — 字节数+校验和
//! - Parameter Request (设备号 `3n` = 48-63): F0 43 3n 4C [addr3] F7 — 请求回传(需 IN 方向)
//! 校验和 (仅 bulk): cs = (~(Σ(addr+data) & 127) + 1) & 127

/// XG 设备号段: 参数修改用 `1n`(16-31), 批量用 `0n`(0-15), 请求用 `3n`(48-63)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    /// Bulk dump 设备号 (0n, 0-15)
    Bulk(u8),
    /// Parameter change 设备号 (1n, 16-31)
    Param(u8),
    /// Parameter request 设备号 (3n, 48-63)
    Request(u8),
    /// Dump request 设备号 (2n, 32-47): XG DUMP REQUEST → 回 BULK DUMP
    DumpRequest(u8),
}

impl Device {
    pub fn byte(self) -> u8 {
        match self {
            Device::Bulk(n) => 0x00 | (n & 0x0F),
            Device::Param(n) => 0x10 | (n & 0x0F),
            Device::Request(n) => 0x30 | (n & 0x0F),
            Device::DumpRequest(n) => 0x20 | (n & 0x0F),
        }
    }
}

/// 单条 XG 参数修改消息: F0 43 1n 4C [addr3] [data...] F7 — 无校验和
/// device: 常用 0 (≡ 0x10)
#[inline]
pub fn param_change(device: Device, addr: [u8; 3], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 3 + data.len() + 1);
    out.push(0xF0);
    out.push(0x43);
    out.push(device.byte());
    out.push(0x4C);
    out.extend_from_slice(&addr);
    out.extend_from_slice(data);
    out.push(0xF7);
    out
}

/// XG System On: 固定 F0 43 1n 4C 00 00 7E 00 F7
pub fn xg_system_on() -> Vec<u8> {
    param_change(
        Device::Param(0),
        [0x00, 0x00, 0x7E],
        &[0x00],
    )
}

/// All Parameter Reset: F0 43 1n 4C 00 00 7F 00 F7
pub fn all_reset() -> Vec<u8> {
    param_change(Device::Param(0), [0x00, 0x00, 0x7F], &[0x00])
}

/// 校验和 (仅 bulk dump 用): cs = (~(Σ(addr+data) & 127) + 1) & 127
pub fn checksum(addr: &[u8], data: &[u8]) -> u8 {
    let mut sum = 0u8;
    for &b in addr {
        sum = (sum + b) & 0x7F;
    }
    for &b in data {
        sum = (sum + b) & 0x7F;
    }
    (!sum).wrapping_add(1) & 0x7F
}

/// Bulk Data Dump: F0 43 0n 4C [bb bb] [addr3] [data...] [cs] F7
/// bb bb = 数据字节数 (data.len()), 大端 14-bit; cs = checksum(addr, data)
pub fn bulk_dump(device: Device, addr: [u8; 3], data: &[u8]) -> Vec<u8> {
    let byte_count = data.len() as u16;
    let cs = checksum(&addr, data);
    let mut out = Vec::with_capacity(4 + 2 + 3 + data.len() + 2);
    out.push(0xF0);
    out.push(0x43);
    out.push(device.byte());
    out.push(0x4C);
    out.push(((byte_count >> 7) & 0x7F) as u8);
    out.push((byte_count & 0x7F) as u8);
    out.extend_from_slice(&addr);
    out.extend_from_slice(data);
    out.push(cs);
    out.push(0xF7);
    out
}

/// Bulk dump 的多段拆分 (14-bit 字节数上限 16383):
/// 若 data 超过上限, 拆成多段 bulk; 否则返回单段
pub fn bulk_dump_chunked(device: Device, base_addr: [u8; 3], data: &[u8], per_chunk: usize) -> Vec<Vec<u8>> {
    const MAX_CHUNK: usize = 16383;
    let per = per_chunk.min(MAX_CHUNK);
    let mut out = Vec::new();
    let mut off = 0usize;
    let mut addr = base_addr;
    while off < data.len() {
        let take = (data.len() - off).min(per);
        let mut full_addr = addr;
        // 地址累加 (3 字节小端进位, 与 Yanaha 手册一致: 低字节先满)
        let chunk_start = off as u16;
        full_addr[2] = base_addr[2].wrapping_add((chunk_start & 0x7F) as u8);
        full_addr[1] = base_addr[1].wrapping_add(((chunk_start >> 7) & 0x7F) as u8);
        full_addr[0] = base_addr[0].wrapping_add(((chunk_start >> 14) & 0x7F) as u8);
        out.push(bulk_dump(device, full_addr, &data[off..off + take]));
        off += take;
    }
    out
}

/// Parameter Request: F0 43 3n 4C [addr3] F7 (请求回读)
pub fn param_request(device: Device, addr: [u8; 3]) -> Vec<u8> {
    let mut out = vec![0xF0, 0x43, device.byte(), 0x4C];
    out.extend_from_slice(&addr);
    out.push(0xF7);
    out
}

/// Dump Request: F0 43 2n 4C [addr3] F7 → 设备回一条 Bulk Dump (2n, 绕过 3n 冷却; 2026-08-09 John 实测 4C 可用)
pub fn dump_request(device: Device, addr: [u8; 3]) -> Vec<u8> {
    let mut out = vec![0xF0, 0x43, device.byte(), 0x4C];
    out.extend_from_slice(&addr);
    out.push(0xF7);
    out
}

/// ---------- 高层 API: Multi Part / System 参数 (地址表驱动) ----------

/// Multi Part (单声部) 参数。part = 0-31 (0 对应硬件 Part 1)。
/// offset 来自 sysex_map.json multi_part 表的 off 字段。
#[inline]
pub fn part_param(device: Device, part: u8, offset: u8, value: u8) -> Result<Vec<u8>, String> {
    if part > 31 {
        return Err(format!("part 超范围: {part} (0-31)"));
    }
    Ok(param_change(device, [0x08, part, offset], &[value]))
}

/// System 参数 (master volume 等)。addr = [h, m, l] 3 字节。
#[inline]
pub fn system_param(device: Device, addr: [u8; 3], value: u8) -> Vec<u8> {
    param_change(device, addr, &[value])
}

/// ---------- sysex_map 部分的编译期常量 (来自 sysex_map.json, 权威) ----------

/// Multi Part 参数偏移 (off, 十六进制)
pub mod mp {
    pub const ELEMENT_RESERVE:  u8 = 0x00;
    pub const BANK_SELECT_MSB:  u8 = 0x01;
    pub const BANK_SELECT_LSB:  u8 = 0x02;
    pub const PROGRAM_NUMBER:   u8 = 0x03;
    pub const RCV_CHANNEL:      u8 = 0x04;
    pub const MONO_POLY:        u8 = 0x05;
    pub const PART_MODE:        u8 = 0x07;
    pub const NOTE_SHIFT:       u8 = 0x08;
    pub const DETUNE:           u8 = 0x09;
    pub const VOLUME:           u8 = 0x0B;
    pub const VELOCITY_SENSE_DEPTH:   u8 = 0x0C;
    pub const VELOCITY_SENSE_OFFSET:  u8 = 0x0D;
    pub const PAN:              u8 = 0x0E;
    pub const NOTE_LIMIT_LOW:   u8 = 0x0F;
    pub const NOTE_LIMIT_HIGH:  u8 = 0x10;
    pub const DRY_LEVEL:        u8 = 0x11;
    pub const CHORUS_SEND:      u8 = 0x12;
    pub const REVERB_SEND:      u8 = 0x13;
    pub const VARIATION_SEND:   u8 = 0x14;
    pub const VIBRATO_RATE:     u8 = 0x15;
    pub const VIBRATO_DEPTH:    u8 = 0x16;
    pub const VIBRATO_DELAY:    u8 = 0x17;
    pub const CUTOFF_FREQ:      u8 = 0x18;
    pub const RESONANCE:        u8 = 0x19;
    pub const EG_ATTACK_TIME:   u8 = 0x1A;
    pub const EG_DECAY_TIME:    u8 = 0x1B;
    pub const EG_RELEASE_TIME:  u8 = 0x1C;
    pub const MW_PITCH_CTRL:    u8 = 0x1D;
    pub const BEND_PITCH_CTRL:  u8 = 0x23;
    pub const RCV_PITCH_BEND:   u8 = 0x30;
    pub const RCV_CH_AFTER_TOUCH: u8 = 0x31;
    pub const RCV_PROGRAM_CHANGE: u8 = 0x32;
    pub const RCV_CONTROL_CHANGE: u8 = 0x33;
}

/// System block 地址 (来自 sysex_map.json system 表)
pub mod sys {
    pub const MASTER_TUNE:  [u8; 3] = [0x00, 0x00, 0x00];
    pub const MASTER_VOLUME:[u8; 3] = [0x00, 0x00, 0x04];
    pub const MASTER_ATT:   [u8; 3] = [0x00, 0x00, 0x05];
    pub const TRANSPOSE:    [u8; 3] = [0x00, 0x00, 0x06];
    pub const XG_SYSTEM_ON: [u8; 3] = [0x00, 0x00, 0x7E];
    pub const ALL_RESET:    [u8; 3] = [0x00, 0x00, 0x7F];
}

/// 音色选择三件套 — MIDI Channel Message (非 SysEx)
/// 返回 3 条消息 (按推荐顺序: MSB → LSB → PC):
///   [B0 00 MSB]  Bank Select MSB (CC0)
///   [B0 20 LSB]  Bank Select LSB (CC32)
///   [C0 PC]      Program Change
/// channel: 0-based (0..15)
pub fn voice_select_messages(channel: u8, msb: u8, lsb: u8, pc: u8) -> Vec<Vec<u8>> {
    let ch = channel & 0x0F;
    vec![
        vec![0xB0 | ch, 0x00, msb & 0x7F],
        vec![0xB0 | ch, 0x20, lsb & 0x7F],
        vec![0xC0 | ch, pc & 0x7F],
    ]
}

/// 把三件套按顺序拼成一条连续发送缓冲 (含"无 SysEx"的纯通道消息)
pub fn voice_select_bytes(channel: u8, msb: u8, lsb: u8, pc: u8) -> Vec<u8> {
    voice_select_messages(channel, msb, lsb, pc).into_iter().flatten().collect()
}

/// 编辑器音色选择 (XG Multi-Part SysEx, port-agnostic):
/// 直接设定 part (0-based 0-31) 的 Bank MSB / LSB / Program, 不依赖 MIDI channel 路由。
/// 等价于在 MU90 面板上选音色 — 任何 part (含 port B 未接的 17-32) 都能设。
/// 地址: F0 43 3n 4C 08 nn 01 msb / 08 nn 02 lsb / 08 nn 03 pc F7
pub fn part_voice_select_messages(part: u8, msb: u8, lsb: u8, pc: u8, device: Device) -> Vec<Vec<u8>> {
    let part = part.min(31);
    [mp::BANK_SELECT_MSB, mp::BANK_SELECT_LSB, mp::PROGRAM_NUMBER]
        .iter()
        .zip([msb & 0x7F, lsb & 0x7F, pc & 0x7F])
        .map(|(&off, val)| param_change(device, [0x08, part, off], &[val]))
        .collect()
}

/// 读 part 的单个参数地址 (握手状态机逐地址调用): 发一条 **XG PARAMETER REQUEST** (3n)。
///   `F0 43 3n 4C 08 nn off F7` → MU90 回一条 DT1: `F0 43 1n 4C 08 nn off val F7`
///
/// [2026-08-09 真机铁证]
///   - **3n PARAMETER REQUEST 是唯一被 MU90 响应的请求类型** (v56 首条回包 = DT1)。
///   - 连续 3 条 3n (08 01/02/03, 80ms 间隔) 实测 **只答第一条** → 不能连发。
///   - **2n DUMP REQUEST 完全不被响应** (v60 `08 00 00` + v64 `08 00 01` 均无回包) → 已排除。
///   - 唯一可靠方案: **逐地址请求 + 等回包再发下一条** (由上层握手状态机驱动)。
///
/// `off` = part 参数偏移 (08 nn xx): mp::BANK_SELECT_MSB / BANK_SELECT_LSB / PROGRAM_NUMBER。
/// 返回单条请求消息 (Vec<u8>)。
pub fn read_part_voice_param(part: u8, off: u8, device: Device) -> Vec<u8> {
    let part = part.min(31);
    let addr = [0x08, part, off];
    let mut out = vec![0xF0, 0x43, device.byte(), 0x4C];
    out.extend_from_slice(&addr);
    out.push(0xF7);
    out
}

/// RQ1 校验和: 对 (byte_count 两字节 + addr 三字节) 求 7-bit 补码和. (仅 BULK DUMP 请求需, 当前未用)
#[allow(dead_code)]
fn checksum_for_request(addr: &[u8], byte_count: u16) -> u8 {
    let mut sum = 0u8;
    sum = (sum + ((byte_count >> 7) & 0x7F) as u8) & 0x7F;
    sum = (sum + (byte_count & 0x7F) as u8) & 0x7F;
    for &b in addr {
        sum = (sum + b) & 0x7F;
    }
    (!sum).wrapping_add(1) & 0x7F
}

/// 收集器: 累积一条 Parameter Request 的回包, 拼出 (part, msb, lsb, pc).
/// 支持两种回包:
///   1. bulk dump: F0 43 0n 4C bb bb 08 nn 01 msb lsb pc cc F7 (一条到齐, 推荐)
///   2. 三条 DT1 逐条回: F0 43 1n 4C 08 nn 01/02/03 val F7 (兼容旧式)
#[derive(Debug, Clone, Default)]
pub struct PartVoiceCollector {
    part: Option<u8>,
    msb: Option<u8>,
    lsb: Option<u8>,
    pc: Option<u8>,
}

impl PartVoiceCollector {
    pub fn new() -> Self { Self::default() }

    /// 喂一条输入 SysEx (完整 F0..F7). 返回 Some=True 表示已凑齐该 part 的 msb+lsb+pc.
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        // 优先 bulk dump: F0 43 0n 4C [bb bb] [08 nn 01] [msb lsb pc] [cs] F7
        if let Some(r) = Self::try_bulk_dump(bytes) {
            let (part, msb, lsb, pc) = r;
            self.part = Some(part); self.msb = Some(msb); self.lsb = Some(lsb); self.pc = Some(pc);
            return true;
        }
        // 回退 DT1 逐条: F0 43 1n 4C [aa bb cc] [dd] F7
        if let Some((part, off, val)) = Self::try_dt1(bytes) {
            if self.part != Some(part) {
                self.part = Some(part);
                self.msb = None; self.lsb = None; self.pc = None;
            }
            match off {
                mp::BANK_SELECT_MSB => self.msb = Some(val),
                mp::BANK_SELECT_LSB => self.lsb = Some(val),
                mp::PROGRAM_NUMBER  => self.pc  = Some(val),
                _ => return false,
            }
            return self.msb.is_some() && self.lsb.is_some() && self.pc.is_some();
        }
        false
    }

    /// bulk dump 解析: F0 43 0n 4C [bb bb] [08 nn xx] [data: msb(off0) lsb(off1) pc(off2)...] [cs] F7
    /// 接受 xx=00 (part 区起始=Element Reserve) 或 xx=01 (Bank MSB 起始, 我们的 DUMP REQUEST)。
    /// 无论起始偏移, 恢复出的数据开头都是 [msb lsb pc ...] (若从 00 起前 3 字节含 Element Reserve, 则跳过)。
    /// 返回 Some((part, msb, lsb, pc))。
    pub fn try_bulk_dump(bytes: &[u8]) -> Option<(u8, u8, u8, u8)> {
        if bytes.len() < 12 || bytes[0] != 0xF0 || bytes[1] != 0x43 || bytes[3] != 0x4C
            || bytes[bytes.len() - 1] != 0xF7 || (bytes[2] & 0xF0) != 0x00 {
            return None;
        }
        let addr_major = bytes[6];
        if addr_major != 0x08 { return None; }
        let part = bytes[7];
        let off = bytes[8];
        // 数据区 = index 9 .. len-2 (去掉 cs 和 F7)
        let data = &bytes[9..bytes.len() - 2];
        // 起始偏移决定前导: 如果从 00 起, 第一个字节是 Element Reserve (跳过);
        // 从 01 起 → data[0..3] = msb/lsb/pc; 从 00 起 → data[1..4] = msb/lsb/pc
        let msb_off = match off {
            0x00 => 1, // Element Reserve 在 data[0], msb 从 data[1]
            0x01 => 0, // Bank MSB 即 data[0]
            _ => return None,
        };
        if data.len() < msb_off + 3 { return None; }
        Some((part, data[msb_off], data[msb_off + 1], data[msb_off + 2]))
    }

    /// DT1 解析: F0 43 1n 4C [aa bb cc] [dd] F7 → Some((part, offset, val))
    pub fn try_dt1(bytes: &[u8]) -> Option<(u8, u8, u8)> {
        if bytes.len() != 9 || bytes[0] != 0xF0 || bytes[1] != 0x43 || bytes[3] != 0x4C
            || bytes[8] != 0xF7 || (bytes[2] & 0xF0) != 0x10 || bytes[4] != 0x08 {
            return None;
        }
        Some((bytes[5], bytes[6], bytes[7]))
    }

    /// 已收集的 (part, msb, lsb, pc). pc 为 0-based program (XG: value 0-127 → 0-based).
    pub fn result(&self) -> Option<(u8, u8, u8, u8)> {
        Some((self.part?, self.msb?, self.lsb?, self.pc?))
    }

    pub fn reset(&mut self) {
        self.part = None; self.msb = None; self.lsb = None; self.pc = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Device::{Bulk, Param, Request};

    #[test]
    fn mu90_real_dt1_reply_feed() {
        // John 真机 (2026-08-09): status bar 收到 YAMAHA UX16 回包
        // F0 43 10 4C 08 00 01 00 F7 = part1(part0) Bank MSB=0 的 DT1 回包.
        let mut c = PartVoiceCollector::new();
        // 第 1 条: MSB
        assert!(!c.feed(&[0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x01, 0x00, 0xF7]));
        assert_eq!(c.result(), None, "只凑齐 MSB 不应出结果");
        // 第 2 条: LSB
        assert!(!c.feed(&[0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x02, 0x00, 0xF7]));
        assert_eq!(c.result(), None);
        // 第 3 条: PC
        assert!(c.feed(&[0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x03, 0x00, 0xF7]), "三条凑齐应返回 done");
        assert_eq!(c.result(), Some((0, 0, 0, 0)), "part0 msb0 lsb0 pc0");
    }

    #[test]
    #[allow(unused_braces)]
    fn mu90_real_dt1_reply_part17() {
        // part17 (0-based 16) 回包: 地址 08 10 → collector 应识别 part=16, 且 pc 0-based
        let mut c = PartVoiceCollector::new();
        assert!(!c.feed(&[0xF0, 0x43, 0x10, 0x4C, 0x08, 0x10, 0x01, 0x00, 0xF7]));
        assert!(!c.feed(&[0xF0, 0x43, 0x10, 0x4C, 0x08, 0x10, 0x02, 0x29, 0xF7])); // lsb=41
        assert!(c.feed(&[0xF0, 0x43, 0x10, 0x4C, 0x08, 0x10, 0x03, 0x24, 0xF7])); // pc=36
        assert_eq!(c.result(), Some((16, 0, 41, 36)), "part17 msb0 lsb41 pc36");
    }

    #[test]
    fn device_bytes() {
        assert_eq!(Bulk(0).byte(), 0x00);
        assert_eq!(Bulk(3).byte(), 0x03);
        assert_eq!(Param(0).byte(), 0x10);
        assert_eq!(Param(1).byte(), 0x11);
        assert_eq!(Param(15).byte(), 0x1F);
        assert_eq!(Request(0).byte(), 0x30);
        assert_eq!(Request(7).byte(), 0x37);
    }

    #[test]
    fn part_voice_select_multi_part() {
        // 编辑器音色选择: XG Multi-Part SysEx (port-agnostic, part 1-32 都能设)
        // part 17 (0-based 16), Dream (msb0 lsb41 pc0):
        // F0 43 10 4C 08 10 01 00 F7 / 08 10 02 29 F7 / 08 10 03 00 F7
        let msgs = part_voice_select_messages(16, 0, 41, 0, Param(0));
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0], vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x10, 0x01, 0x00, 0xF7]);
        assert_eq!(msgs[1], vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x10, 0x02, 0x29, 0xF7]);
        assert_eq!(msgs[2], vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x10, 0x03, 0x00, 0xF7]);
        // part 1 (0-based 0)
        let msgs1 = part_voice_select_messages(0, 0, 0, 40, Param(0));
        assert_eq!(msgs1[2], vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x03, 40, 0xF7]);
    }

    #[test]
    fn param_change_byte_exact() {
        // 基准向量 (手册 2.1.4.1): F0 43 10 4C + addr + data + F7, 无校验和
        let msg = param_change(Param(0), [0x00, 0x00, 0x04], &[0x7F]);
        assert_eq!(msg, vec![0xF0, 0x43, 0x10, 0x4C, 0x00, 0x00, 0x04, 0x7F, 0xF7]);
    }

    #[test]
    fn system_on_exact() {
        let msg = xg_system_on();
        assert_eq!(msg, vec![0xF0, 0x43, 0x10, 0x4C, 0x00, 0x00, 0x7E, 0x00, 0xF7]);
        // 设备号可用非 0 变体
        let m2 = xg_system_on();
        assert_eq!(m2[2], Param(0).byte());
    }

    #[test]
    fn all_reset_exact() {
        assert_eq!(all_reset(), vec![0xF0, 0x43, 0x10, 0x4C, 0x00, 0x00, 0x7F, 0x00, 0xF7]);
    }

    #[test]
    fn checksum_vectors() {
        // 手册 worked example (bulk): 地址=00 00 00, 数据=[04 7f] → cs 0x7D
        assert_eq!(checksum(&[0x00, 0x00, 0x00], &[0x04, 0x7f]), 0x7D);
        // 空数据: sum=0 → cs = (~0+1)&127 = 0
        assert_eq!(checksum(&[0x00, 0x00, 0x00], &[]), 0x00);
        // 累积 >127 的进位
        {
            // 累积 >127: 0x7f*4 = 508; 508 & 0x7f = 124 (508 = 3*128 + 124)
            let s: u8 = ((0x7f + 0x7f + 0x7f + 0x7f) as i32 & 0x7f) as u8;
            assert_eq!(s, 124, "sanity: 508 & 0x7f should be 124");
            let expected: u8 = (!s).wrapping_add(1) & 0x7f;
            // (~124+1)&127 = 4 (Python 验证)
            assert_eq!(expected, 4, "sanity: checksum of 4x0x7f");
            assert_eq!(checksum(&[0x7f, 0x7f, 0x7f], &[0x7f]), expected);
        }
    }

    #[test]
    fn bulk_dump_exact() {
        // 手册 bulk 格式: F0 43 0n 4C bb bb aa aa aa dd... dd cc F7
        let msg = bulk_dump(Bulk(0), [0x00, 0x00, 0x00], &[0x04, 0x7f]);
        assert_eq!(msg, vec![0xF0, 0x43, 0x00, 0x4C, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x7F, 0x7D, 0xF7]);
    }

    #[test]
    fn bulk_byte_count_14bit() {
        // 32767 数据 → bb bb = 高 7 位+低 7 位
        let data: Vec<u8> = vec![0u8; 100];
        let msg = bulk_dump(Bulk(1), [0x01, 0x02, 0x03], &data);
        assert_eq!(msg[4], (100 >> 7) as u8);
        assert_eq!(msg[5], (100 & 0x7F) as u8);
        // 校验和位置在数据后
        let cs_expected = checksum(&[0x01, 0x02, 0x03], &data);
        assert_eq!(msg[msg.len() - 2], cs_expected);
        assert_eq!(*msg.last().unwrap(), 0xF7);
    }

    #[test]
    fn part_param_addr() {
        // Part 0, VOLUME(0x0B): 地址 08 00 0B
        let msg = part_param(Param(0), 0, mp::VOLUME, 100).unwrap();
        assert_eq!(msg, vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x0B, 100, 0xF7]);
        // Part 15, CUTOFF(0x18): 08 0F 18
        let msg = part_param(Param(0), 15, mp::CUTOFF_FREQ, 64).unwrap();
        assert_eq!(msg, vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x0F, 0x18, 64, 0xF7]);
        // part 超范围 → Err
        assert!(part_param(Param(0), 32, mp::VOLUME, 1).is_err());
    }

    #[test]
    fn full_voice_select_sequence() {
        // 完整 XG 音色选择 = MSB + LSB + PC 三连发 (用户实测过, 切 000 041 Violin)
        // 用 SysEx part 参数来选, 地址 08 nn 01(msb) 02(lsb) 03(prg)
        let msb = part_param(Param(0), 0, mp::BANK_SELECT_MSB, 0x00).unwrap();
        let lsb = part_param(Param(0), 0, mp::BANK_SELECT_LSB, 0x00).unwrap();
        let pc  = part_param(Param(0), 0, mp::PROGRAM_NUMBER, 40).unwrap(); // Violin (prg 40, 0-based)
        assert_eq!(msb, vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x01, 0x00, 0xF7]);
        assert_eq!(lsb, vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x02, 0x00, 0xF7]);
        assert_eq!(pc , vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x03, 40, 0xF7]);
    }

    #[test]
    fn param_request_exact() {
        // F0 43 3n 4C addr3 F7
        let msg = param_request(Request(0), [0x08, 0x00, 0x0B]);
        assert_eq!(msg, vec![0xF0, 0x43, 0x30, 0x4C, 0x08, 0x00, 0x0B, 0xF7]);
    }

    #[test]
    fn read_part_voice_param_requests() {
        // 读 part3 (0-based 2) 音色 → 每条一个地址的 XG PARAMETER REQUEST (3n)
        // 2026-08-09 John WedMIDI 实测: device 号 1 (0x31) 三条全回; 之前 0 (0x30) 常不回/只回1条
        // part3 → F0 43 31 4C 08 02 01 F7 (MSB)
        let m = read_part_voice_param(2, mp::BANK_SELECT_MSB, Request(1));
        assert_eq!(m, vec![0xF0, 0x43, 0x31, 0x4C, 0x08, 0x02, 0x01, 0xF7]);
        // LSB 地址
        let l = read_part_voice_param(2, mp::BANK_SELECT_LSB, Request(1));
        assert_eq!(l, vec![0xF0, 0x43, 0x31, 0x4C, 0x08, 0x02, 0x02, 0xF7]);
        // PC 地址
        let p = read_part_voice_param(2, mp::PROGRAM_NUMBER, Request(1));
        assert_eq!(p, vec![0xF0, 0x43, 0x31, 0x4C, 0x08, 0x02, 0x03, 0xF7]);
        // part1 (0-based 0): addr 08 00 01
        let m1 = read_part_voice_param(0, mp::BANK_SELECT_MSB, Request(1));
        assert_eq!(m1, vec![0xF0, 0x43, 0x31, 0x4C, 0x08, 0x00, 0x01, 0xF7]);
    }

    #[test]
    fn collector_parses_part_block_bulk_dump() {
        // MU90 对 DUMP REQUEST (08 00 00) 的 bulk 回包: part 区块一条到齐
        // F0 43 0n 4C [bb bb] 08 00 00 [msb lsb pc ...18B] [cs] F7
        // part1: msb=0 lsb=0 pc=0 (GrandPno)
        let mut block = vec![0u8; 18]; // part 参数区 18 字节 (08 00 00 + 多字节)
        block[8] = 0x40; // 演示非零 (不影响前 3 字节)
        let mut reply = vec![0xF0, 0x43, 0x00, 0x4C, 0x00, 18];
        reply.extend_from_slice(&[0x08, 0x00, 0x00]);
        reply.extend_from_slice(&block);
        reply.push(0x74); // cs (占位, collector 不校验)
        reply.push(0xF7);
        let mut c = PartVoiceCollector::new();
        assert!(c.feed(&reply), "part 区块 bulk 一条应解析出结果");
        assert_eq!(c.result(), Some((0, 0, 0, 0)));
    }

    #[test]
    fn collector_prefers_bulk_over_dt1() {
        // 同一 collector: bulk dump 一条到齐; DT1 逐条也兼容
        // part5 (0-based 4) bulk 回包, 起始 08 04 01 (Bank MSB): msb=0 lsb=41(Dream) pc=0x24
        // F0 43 00 4C 00 03 08 04 01 [00 29 24 00] cs F7 (request 08 04 01 → 回块从 msb 起)
        let bulk = vec![0xF0, 0x43, 0x00, 0x4C, 0x00, 0x03, 0x08, 0x04, 0x01, 0x00, 0x29, 0x24, 0x00, 0x00, 0xF7];
        let mut c = PartVoiceCollector::new();
        assert!(c.feed(&bulk), "bulk (off=01) 应解析出结果");
        assert_eq!(c.result(), Some((4, 0, 0x29, 0x24)));
        // part16 (0-based 15) pc=0x0F
        let bulk16 = vec![0xF0, 0x43, 0x00, 0x4C, 0x00, 0x03, 0x08, 0x0F, 0x01, 0x00, 0x00, 0x0F, 0x00, 0xF7];
        let mut c2 = PartVoiceCollector::new();
        assert!(c2.feed(&bulk16), "bulk16 (off=01) 应解析出结果");
        assert_eq!(c2.result(), Some((15, 0, 0, 0x0F)));
        let _ = (c, c2);
    }

    #[test]
    fn part_voice_collector_assembles_dt1() {
        // 模拟 MU90 对 part 5 读请求的 3 条 DT1 回包: 先 msb=0, 再 lsb=41(Dream), 再 pc=0
        let mut c = PartVoiceCollector::new();
        let msb = vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x05, 0x01, 0x00, 0xF7];
        let lsb = vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x05, 0x02, 0x29, 0xF7];
        let pc  = vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x05, 0x03, 0x00, 0xF7];
        assert!(!c.feed(&msb));
        assert!(!c.feed(&lsb));
        assert!(c.feed(&pc), "第三条凑齐 → 返回 true");
        assert_eq!(c.result(), Some((5, 0, 41, 0)), "part5 = Dream(lsb41) pc0");
        // 乱序喂: 重发 msb, pc 已齐 → 仍 true (msb 更新)
        let msb2 = vec![0xF0, 0x43, 0x10, 0x4C, 0x08, 0x05, 0x01, 0x40, 0xF7];
        assert!(c.feed(&msb2));
        assert_eq!(c.result().unwrap().1, 0x40);
    }

    #[test]
    fn part_voice_collector_rejects_noise() {
        // 非 DT1 / 非 0x08 区 / 长度不对 → false 不污染
        let mut c = PartVoiceCollector::new();
        assert!(!c.feed(&[0xF0, 0x43, 0x40, 0x4C, 0x08, 0x00, 0x01, 0x00, 0xF7])); // 0x40 非 1n
        assert!(!c.feed(&[0xF0, 0x43, 0x10, 0x4C, 0x00, 0x00, 0x00, 0x7E, 0x00, 0xF7])); // addr 00 区
        assert!(!c.feed(&[0xF0, 0x43, 0x10, 0x4C, 0x08, 0x00, 0x01])); // 截断
        assert_eq!(c.result(), None);
    }

    #[test]
    fn bulk_chunk_split() {
        // 大数据拆段, 每段地址累加正确
        let data: Vec<u8> = (0..100u8).collect();
        let chunks = bulk_dump_chunked(Bulk(0), [0x30, 0x0D, 0x00], &data, 50);
        assert_eq!(chunks.len(), 2);
        // 第一段地址 = 30 0D 00
        assert_eq!(&chunks[0][6..9], &[0x30, 0x0D, 0x00]);
        // 第二段地址低字节 +50
        assert_eq!(&chunks[1][6..9], &[0x30, 0x0D, 0x32]); // 0x00+50=0x32
    }

    #[test]
    fn voice_select_sequence() {
        // GrandPno = msb0 lsb0 pc0, ch0 → B0 00 00 | B0 20 00 | C0 00
        // DreamPno = msb0 lsb41 pc0
        let msgs = voice_select_messages(0, 0, 41, 0);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0], vec![0xB0, 0x00, 0x00]);
        assert_eq!(msgs[1], vec![0xB0, 0x20, 41]);
        assert_eq!(msgs[2], vec![0xC0, 0x00]);
        // 拼连续字节
        let flat = voice_select_bytes(0, 0, 41, 0);
        assert_eq!(flat, vec![0xB0, 0x00, 0x00, 0xB0, 0x20, 41, 0xC0, 0x00]);
        // channel 编码: ch15 → BF / CF
        let msgs15 = voice_select_messages(15, 0x7F, 0x7F, 0x7F);
        assert_eq!(msgs15[0][0], 0xBF);
        assert_eq!(msgs15[2][0], 0xCF);
        // 值都掩到 7bit
        assert!(msgs15.iter().all(|m| m[1..].iter().all(|&b| b < 0x80)));
    }
}
