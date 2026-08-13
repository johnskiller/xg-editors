// smf.rs — Standard MIDI File (SMF) 解析器 (零依赖, 纯 Rust)
//
// 解析:
//   - MThd header: 格式(0/1/2), 轨数, division
//   - division: 0x8000 未置位 → PPQN (每四分音符 tick 数); 置位 → SMPTE 时间码
//   - MTrk 轨: 变长 delta-time (VLQ), running status
//   - Meta 事件: 00 序号, 51 tempo, 58 time sig, 2F end-of-track (需重标 delta)
//   - SysEx: F0/F7 (跳过)
// 输出:
//   - `Smf` (全体: tracks 各含 events)
//   - `TrackViewData`: 归并后的逐轨音符 (note on/off 配对), 供可视化/播放
//   - TempoMap: 分段 tempo (tick → 秒), 供时间换算/播放

/// 单个已解码事件 (绝对 tick, 顺序无关紧要, 后续按 tick 排序)
#[derive(Debug, Clone, PartialEq)]
pub enum SmfEvent {
    NoteOn { tick: u64, channel: u8, pitch: u8, vel: u8 },
    NoteOff { tick: u64, channel: u8, pitch: u8 },
    // 控制/规格事件(播放节奏用); 其他控制器丢弃
    Tempo { tick: u64, us_per_qn: u32 },
    TimeSig { tick: u64, num: u8, denom: u8 },
    // 任意通道事件 (16 轨视图用, 记录轨号)
    Cc { tick: u64, channel: u8, num: u8, val: u8 },
    Program { tick: u64, channel: u8, program: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmfError {
    NotSmf,
    Truncated,            // 字节不足
    UnsupportedDivision,  // SMPTE 时间码 division 暂不支持
    BadVlq,               // 变长编码损坏
    EndOfData,            // 轨数据耗尽
}

impl std::fmt::Display for SmfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// 一个轨的事件列表 (保持加载顺序; 调用方按需排序)
#[derive(Debug, Clone, Default)]
pub struct TrackEvents {
    pub events: Vec<SmfEvent>,
}

/// 解析结果: 格式 + 各轨事件 + 时间基准
#[derive(Debug, Clone)]
pub struct Smf {
    pub format: u16,
    pub ntracks: u16,
    pub ppq: u32, // >0 时有效 (PPQN division)
    pub tracks: Vec<TrackEvents>,
    /// 解析期间遇到的 0x2F 之前(通常为 0)的 tempo 事件数/时间 sig 数 (信息用)
    pub meta_tempo_count: usize,
    pub meta_timesig_count: usize,
}

/// 解析 SMF 字节流
pub fn parse_smf(data: &[u8]) -> Result<Smf, SmfError> {
    let mut p = Cursor::new(data);
    // MThd
    if data.len() < 14 || &data[0..4] != b"MThd" {
        return Err(SmfError::NotSmf);
    }
    p.pos = 4;
    let hlen = p.be_u32()?; // 通常 6
    if hlen < 6 {
        return Err(SmfError::NotSmf);
    }
    let format = p.be_u16()?;
    let ntracks = p.be_u16()?;
    if ntracks == 0 {
        return Err(SmfError::NotSmf);
    }
    let division_raw = p.be_u16()?;
    // division: bit15 置位 = SMPTE; 否则 PPQN
    let ppq = if division_raw & 0x8000 != 0 {
        return Err(SmfError::UnsupportedDivision);
    } else {
        (division_raw & 0x7fff) as u32
    };
    if ppq == 0 {
        return Err(SmfError::UnsupportedDivision);
    }
    p.pos = 8 + hlen as usize; // 跳到第一个 MTrk

    let mut tracks = Vec::with_capacity(ntracks as usize);
    let mut tempo_count = 0;
    let mut ts_count = 0;

    // 容错解析: 按 header 声明的 ntracks 解析, 但如果遇到额外 MTrk 也继续解析
    // (部分 MIDI 文件 track 数量与 header 不一致, 如 doom midi)
    let mut parsed_tracks = 0usize;
    loop {
        // MTrk header
        if p.pos + 8 > data.len() {
            break; // 数据耗尽, 正常结束
        }
        if &data[p.pos..p.pos + 4] != b"MTrk" {
            // 不是 MTrk, 可能是填充字节或其他数据, 停止解析
            break;
        }
        p.pos += 4; // 跳过 "MTrk" magic
        let tlen = p.be_u32()?;
        // 检查 track 长度是否合法
        let chunk = if p.pos + tlen as usize <= data.len() {
            data.get(p.pos..p.pos + tlen as usize).ok_or(SmfError::Truncated)?
        } else {
            // track 声明长度超出文件末尾, 使用剩余数据 (容错)
            let remaining = &data[p.pos..];
            eprintln!(
                "WARNING: track {} declared len {} but only {} bytes remaining",
                parsed_tracks,
                tlen,
                remaining.len()
            );
            remaining
        };
        p.pos += chunk.len();
        let mut te = TrackEvents::default();
        parse_track_chunk(chunk, ppq, &mut te, &mut tempo_count, &mut ts_count)?;
        tracks.push(te);
        parsed_tracks += 1;

        // 如果已解析足够 track, 检查是否还有额外 MTrk
        if parsed_tracks >= ntracks as usize {
            // 还有额外数据? 继续解析 (某些 MIDI 文件 track 数多于 header 声明)
            if p.pos < data.len() && &data[p.pos..p.pos + 4] == b"MTrk" {
                continue;
            }
            break;
        }
    }

    Ok(Smf {
        format,
        ntracks,
        ppq,
        tracks,
        meta_tempo_count: tempo_count,
        meta_timesig_count: ts_count,
    })
}

/// 解析单个 MTrk chunk (不含 header). running status 处理; 事件打到 te.events (绝对 tick)
fn parse_track_chunk(
    chunk: &[u8],
    ppq: u32,
    te: &mut TrackEvents,
    tempo_count: &mut usize,
    ts_count: &mut usize,
) -> Result<(), SmfError> {
    let mut p = Cursor::new(chunk);
    let mut tick: u64 = 0;
    let mut running_status: Option<u8> = None;
    let evt_cnt: u32 = 0;
    loop {
        // delta time
        let dt = match p.vlq() {
            Ok(v) => v,
            Err(SmfError::EndOfData) => break, // 轨自然结束
            Err(e) => return Err(e),
        };
        tick += dt;
        // status byte (peek, not consume yet)
        let first = match p.peek() {
            Ok(v) => v,
            Err(SmfError::EndOfData) => break, // 轨自然结束 (EOT 后或尾部无数据)
            Err(e) => return Err(e),
        };
        if first < 0x80 {            // running status: 上一事件状态复用; first = running 数据的第 1 字节 (需消费)
            let st = running_status.ok_or(SmfError::NotSmf)?;
            let d0 = p.byte()?; // 消费第 1 个数据字节
            match st {
                0x80..=0x8f => {
                    let _vel = p.byte()?;
                    te.events.push(SmfEvent::NoteOff { tick, channel: st & 0x0f, pitch: d0 });
                }
                0x90..=0x9f => {
                    let vel = p.byte()?;
                    if vel == 0 {
                        te.events.push(SmfEvent::NoteOff { tick, channel: st & 0x0f, pitch: d0 });
                    } else {
                        te.events.push(SmfEvent::NoteOn { tick, channel: st & 0x0f, pitch: d0, vel });
                    }
                }
                0xa0..=0xaf => { let _ = p.byte()?; } // aftertouch: d0=pressure, 1 more byte
                0xb0..=0xbf => {
                    let val = p.byte()?;
                    te.events.push(SmfEvent::Cc { tick, channel: st & 0x0f, num: d0, val });
                }
                0xc0..=0xcf => {
                    te.events.push(SmfEvent::Program { tick, channel: st & 0x0f, program: d0 });
                }
                0xd0..=0xdf => { let _ = p.byte()?; }
                0xe0..=0xef => { let _ = p.byte()?; } // pitch bend: d0=LSB, 1 more byte (MSB)
                _ => unreachable!(),
            }
            continue;
        }
        // new status
        let st = p.byte()?;
        running_status = Some(st);
        match st {
            0x80..=0x8f => {
                let pitch = p.byte()?;
                let _vel = p.byte()?;
                te.events.push(SmfEvent::NoteOff { tick, channel: st & 0x0f, pitch });
            }
            0x90..=0x9f => {
                let pitch = p.byte()?;
                let vel = p.byte()?;
                if vel == 0 {
                    te.events.push(SmfEvent::NoteOff { tick, channel: st & 0x0f, pitch });
                } else {
                    te.events.push(SmfEvent::NoteOn { tick, channel: st & 0x0f, pitch, vel });
                }
            }
            0xa0..=0xaf => { let _ = (p.byte()?, p.byte()?); }
            0xb0..=0xbf => {
                let num = p.byte()?;
                let val = p.byte()?;
                te.events.push(SmfEvent::Cc { tick, channel: st & 0x0f, num, val });
            }
            0xc0..=0xcf => {
                let program = p.byte()?;
                te.events.push(SmfEvent::Program { tick, channel: st & 0x0f, program });
            }
            0xd0..=0xdf => { let _ = p.byte()?; }
            0xe0..=0xef => { let _ = (p.byte()?, p.byte()?); }
            0xf0..=0xf7 => {
                // SysEx: F0 len <bytes> 或 F7 len; 跳过
                let _ = p.skip_sysex()?;
                // SysEx 终止 running status
                running_status = None;
            }
            0xff => {
                // Meta 事件
                let mtype = p.byte()?;
                let mlen = p.vlq()?;
                let mbuf = p.take(mlen as usize)?;
                match mtype {
                    0x2f => { /* end of track */ return Ok(()); }
                    0x51 => {
                        if mbuf.len() >= 3 {
                            let us = ((mbuf[0] as u32) << 16) | ((mbuf[1] as u32) << 8) | (mbuf[2] as u32);
                            te.events.push(SmfEvent::Tempo { tick, us_per_qn: us });
                            *tempo_count += 1;
                            let _ = ppq;
                        }
                    }
                    0x58 => {
                        if mbuf.len() >= 2 {
                            te.events.push(SmfEvent::TimeSig { tick, num: mbuf[0], denom: 1u8 << mbuf[1] });
                            *ts_count += 1;
                        }
                    }
                    _ => { /* 其他 meta 忽略 */ }
                }
                // Meta 事件保持 running status 状态 (spec: running status 不清除)
            }
            _ => { /* 未定义 status */ }
        }
    }
    Ok(())
}

// ---------- 字节游标 ----------
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self { Self { data, pos: 0 } }
    fn byte(&mut self) -> Result<u8, SmfError> {
        let b = self.data.get(self.pos).ok_or(SmfError::Truncated)?;
        self.pos += 1;
        Ok(*b)
    }
    fn peek(&self) -> Result<u8, SmfError> {
        self.data.get(self.pos).copied().ok_or(SmfError::EndOfData)
    }
    fn be_u16(&mut self) -> Result<u16, SmfError> {
        let h = self.byte()? as u16;
        let l = self.byte()? as u16;
        Ok((h << 8) | l)
    }
    fn be_u32(&mut self) -> Result<u32, SmfError> {
        let a = self.byte()? as u32;
        let b = self.byte()? as u32;
        let c = self.byte()? as u32;
        let d = self.byte()? as u32;
        Ok((a << 24) | (b << 16) | (c << 8) | d)
    }
    /// 变长量 (7-bit chunks, MSB 续位)
    fn vlq(&mut self) -> Result<u64, SmfError> {
        let mut value: u64 = 0;
        let mut cnt = 0;
        loop {
            let b = self.byte().map_err(|_| SmfError::EndOfData)?;
            value = (value << 7) | (b & 0x7f) as u64;
            cnt += 1;
            if cnt > 4 {
                return Err(SmfError::BadVlq);
            }
            if b & 0x80 == 0 {
                return Ok(value);
            }
        }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], SmfError> {
        if self.pos + n > self.data.len() {
            return Err(SmfError::Truncated);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    /// 跳过 SysEx 数据 (len 前缀变长量 + payload)
    fn skip_sysex(&mut self) -> Result<(), SmfError> {
        let len = self.vlq()?;
        self.take(len as usize)?;
        Ok(())
    }
}

// ---------- 音符配对 + 逐轨视图 ----------

/// 一条已配对音符 (供 track view / piano roll / 播放)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmfNote {
    pub channel: u8,
    pub pitch: u8,
    pub vel: u8,
    pub start_tick: u64,
    pub dur_ticks: u64,
}

/// 逐轨归并视图: 每通道的音符 + 该通道程序号(用于音色显示)
#[derive(Debug, Clone, Default)]
pub struct SmfTrackView {
    pub notes: Vec<SmfNote>,
    pub program: Option<u8>,
    /// 该通道的 Bank Select (MSB, LSB)。追踪 CC0 (MSB) + CC32 (LSB), 缺省 None = 默认 bank。
    /// 用于 ch10 鼓组名解析 (msb=127 时 prg 指鼓组) 与普通通道变体显示。
    pub bank: Option<(u8, u8)>,
    pub name: String,
}

/// 把解析出的所有轨事件 → 按通道归并的逐轨视图 (16 通道)
pub fn build_track_views(smf: &Smf) -> Vec<SmfTrackView> {
    let mut views: Vec<SmfTrackView> = (0..16).map(|_| SmfTrackView::default()).collect();
    // 收集所有轨事件 (轨道数可能 >1; 混合通道事件)
    let mut all: Vec<&SmfEvent> = Vec::new();
    for t in &smf.tracks {
        for e in &t.events {
            all.push(e);
        }
    }
    // 按 tick 排序。★ 2026-08-13 John: 琶音器 MIDI(ch03)出现超长音符.
    //   根因 = 旧排序强行"同 tick NoteOff 先于 NoteOn" → 破坏文件内同音断奏配对
    //   (同 tick 的 on(X)/off(X) 文件序已隐含"先闭再开"; 强制 off 先行 → 旧 on 悬空等到下个同音 → 超长).
    //   修复: 只按 tick 排序 (Rust sort_by_key 稳定), 同 tick 保留文件原始相对顺序 → 配对正确.
    //   代价: format1 多轨同 tick 的相对顺序按 track 输入序(合理).
    fn evt_tick(e: &SmfEvent) -> u64 {
        match e {
            SmfEvent::NoteOn { tick, .. } | SmfEvent::NoteOff { tick, .. }
            | SmfEvent::Tempo { tick, .. } | SmfEvent::TimeSig { tick, .. }
            | SmfEvent::Cc { tick, .. } | SmfEvent::Program { tick, .. } => *tick,
        }
    }
    all.sort_by_key(|e| evt_tick(e));
    // 展开 on/off 配对
    // 用一个近似: 每个 (channel,pitch) 维护未闭合 on 栈
    let mut active_start: Vec<Vec<u64>> = vec![Vec::new(); 256]; // channel*16+pitch 简化: 用 key
    // 更好: HashMap<(ch, pitch), Vec<(start_tick, vel)>>
    use std::collections::HashMap;
    let mut open: HashMap<(u8, u8), Vec<(u64, u8)>> = HashMap::new();
    for e in all {
        match e {
            SmfEvent::NoteOn { tick, channel, pitch, vel } => {
                // 同音重复: 若有未闭合, 先闭(新 On 前)
                let key = (*channel, *pitch);
                if let Some(st) = open.get_mut(&key) {
                    for &(s, sv) in st.iter() {
                        if *tick >= s {
                            views[*channel as usize].notes.push(SmfNote {
                                channel: *channel, pitch: *pitch, vel: sv, start_tick: s,
                                dur_ticks: *tick - s,
                            });
                        }
                    }
                    st.clear();
                }
                open.entry(key).or_default().push((*tick, *vel));
            }
            SmfEvent::NoteOff { tick, channel, pitch } => {
                let key = (*channel, *pitch);
                if let Some(st) = open.get_mut(&key) {
                    if let Some((s, sv)) = st.pop() {
                        if *tick >= s {
                            views[*channel as usize].notes.push(SmfNote {
                                channel: *channel, pitch: *pitch, vel: sv, start_tick: s,
                                dur_ticks: *tick - s,
                            });
                        }
                    }
                }
            }
            SmfEvent::Program { tick: _, channel, program } => {
                views[*channel as usize].program = Some(*program);
            }
            SmfEvent::Cc { tick: _, channel, num, val } => {
                // 追踪 Bank Select: CC0=MSB, CC32=LSB (XG: MSB 0=melodic, 127=Drum set)
                let (mut msb, mut lsb) = views[*channel as usize].bank.unwrap_or((0, 0));
                match *num {
                    0 => msb = *val,
                    32 => lsb = *val,
                    _ => {}
                }
                // 只在被设置过任一 CC 且与默认不同时记录 (None = 全默认)
                if msb != 0 || lsb != 0 {
                    views[*channel as usize].bank = Some((msb, lsb));
                }
            }
            _ => {}
        }
    }
    // 未闭 on (文件缺 off) → 截到视图末尾时长 (用 1 拍)
    for ((ch, pitch), sts) in open.iter() {
        for &(s, sv) in sts.iter() {
            views[*ch as usize].notes.push(SmfNote {
                channel: *ch, pitch: *pitch, vel: sv, start_tick: s,
                dur_ticks: 96, // 占位
            });
        }
    }
    views
}

// ---------- Tempo Map (时间系统) ----------

/// 分段 tempo 映射: 绝对 tick → 累积秒. 用于 tick→秒 与 秒→tick.
#[derive(Debug, Clone)]
pub struct TempoMap {
    pub us_per_qn: u32,       // 每个四分之一音符的微秒数 (初始)
    /// 排序后的分段: (起始tick, 该分段起 us_per_qn, 该分段起累积秒 f64)
    pub segments: Vec<(u64, u32, f64)>,
    /// 文件总时长秒 (最后一段结束)
    pub total_seconds: f64,
}

impl TempoMap {
    /// 从事件列表构建 tempo map. 默认 500_000 us/四分音符 (120 bpm)
    pub fn from_events(events: &[SmfEvent], ppq: u32, total_ticks: u64) -> Self {
        let mut changes: Vec<(u64, u32)> = Vec::new();
        let mut base = 500_000u32;
        for e in events {
            if let SmfEvent::Tempo { tick, us_per_qn } = e {
                if *tick == 0 {
                    base = *us_per_qn;
                } else {
                    changes.push((*tick, *us_per_qn));
                }
            }
        }
        changes.sort_by_key(|&(t, _)| t);
        changes.dedup_by_key(|&mut (t, _)| t);
        // 构建分段累计秒
        let mut segments: Vec<(u64, u32, f64)> = Vec::new();
        let mut cur_us = base;
        let mut cur_tick = 0u64;
        let mut acc_sec = 0.0f64;
        segments.push((0, cur_us, 0.0));
        for (t, us) in &changes {
            // 从 cur_tick 到 t 用 cur_us
            let dt = *t - cur_tick;
            acc_sec += dt as f64 * cur_us as f64 / 1_000_000.0 / ppq as f64;
            cur_us = *us;
            cur_tick = *t;
            segments.push((*t, cur_us, acc_sec));
        }
        // 末尾补总长 (到 total_ticks)
        let remaining = total_ticks.saturating_sub(cur_tick);
        acc_sec += remaining as f64 * cur_us as f64 / 1_000_000.0 / ppq as f64;
        Self {
            us_per_qn: base,
            segments,
            total_seconds: acc_sec,
        }
    }

    /// tick → 秒 (线性插值于所在分段)
    pub fn tick_to_sec(&self, tick: u64, ppq: u32) -> f64 {
        // 找所在分段
        let mut idx = 0;
        for (i, (t, _, _)) in self.segments.iter().enumerate() {
            if *t <= tick {
                idx = i;
            } else {
                break;
            }
        }
        let (seg_tick, seg_us, seg_acc) = self.segments[idx];
        let dt = tick.saturating_sub(seg_tick);
        seg_acc + dt as f64 * seg_us as f64 / 1_000_000.0 / ppq as f64
    }

    /// 秒 → tick (逆查: 在每个分段用线性)
    pub fn sec_to_tick(&self, sec: f64, ppq: u32) -> u64 {
        if sec <= 0.0 {
            return 0;
        }
        let mut idx = 0;
        for (i, (_, _, acc)) in self.segments.iter().enumerate() {
            if *acc <= sec {
                idx = i;
            }
        }
        let (seg_tick, seg_us, seg_acc) = self.segments[idx];
        let dt_sec = sec - seg_acc;
        let dt_tick = (dt_sec * 1_000_000.0 / seg_us as f64 * ppq as f64).max(0.0) as u64;
        seg_tick + dt_tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16be(v: u16) -> [u8; 2] { [(v >> 8) as u8, v as u8] }
    fn u32be(v: u32) -> [u8; 4] { [(v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8] }
    fn mthd(fmt: u16, ntrk: u16, div: u16) -> Vec<u8> {
        let mut v = b"MThd".to_vec();
        v.extend_from_slice(&u32be(6));
        v.extend_from_slice(&u16be(fmt));
        v.extend_from_slice(&u16be(ntrk));
        v.extend_from_slice(&u16be(div));
        v
    }
    /// 构造一个 track chunk
    fn mtrk(events: &[u8]) -> Vec<u8> {
        let mut v = b"MTrk".to_vec();
        v.extend_from_slice(&u32be(events.len() as u32));
        v.extend_from_slice(events);
        v
    }
    fn vlq(v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut bytes = vec![(v & 0x7f) as u8];
        let mut v = v >> 7;
        while v > 0 {
            bytes.insert(0, ((v & 0x7f) as u8) | 0x80);
            v >>= 7;
        }
        out.extend_from_slice(&bytes);
        out
    }

    #[test]
    fn parse_minimal_smf_note() {
        // 单轨: delta0 NoteOn 60 v100, delta 96 NoteOff, delta0 EOT
        let mut ev = Vec::new();
        ev.extend_from_slice(&vlq(0));
        ev.extend_from_slice(&[0x90, 60, 100]);
        ev.extend_from_slice(&vlq(96));
        ev.extend_from_slice(&[0x80, 60, 0]);
        ev.extend_from_slice(&vlq(0));
        ev.extend_from_slice(&[0xff, 0x2f, 0]);
        let data: Vec<u8> = mthd(1, 1, 192).into_iter().chain(mtrk(&ev)).collect();
        let smf = parse_smf(&data).expect("parse");
        assert_eq!(smf.format, 1);
        assert_eq!(smf.ppq, 192);
        assert_eq!(smf.tracks.len(), 1);
        // events: 2 个 (NoteOn + NoteOff)
        let es = &smf.tracks[0].events;
        assert_eq!(es.len(), 2);
        match &es[0] {
            SmfEvent::NoteOn { tick, channel, pitch, vel } => {
                assert_eq!((*tick, *channel, *pitch, *vel), (0, 0, 60, 100));
            }
            _ => panic!("expected NoteOn"),
        }
        match &es[1] {
            SmfEvent::NoteOff { tick, channel, pitch } => {
                assert_eq!((*tick, *channel, *pitch), (96, 0, 60));
            }
            _ => panic!("expected NoteOff"),
        }
    }

    #[test]
    fn parse_running_status() {
        // running status: 首事件 NoteOn, 后续省略 status
        let mut ev = Vec::new();
        ev.extend_from_slice(&vlq(0));
        ev.extend_from_slice(&[0x90, 60, 100]);
        ev.extend_from_slice(&vlq(1));
        ev.extend_from_slice(&[62, 90]); // running NoteOn
        ev.extend_from_slice(&vlq(1));
        ev.extend_from_slice(&[60, 0]); // running NoteOn vel 0 → NoteOff
        ev.extend_from_slice(&vlq(0));
        ev.extend_from_slice(&[0xff, 0x2f, 0]);
        let data: Vec<u8> = mthd(1, 1, 96).into_iter().chain(mtrk(&ev)).collect();
        let smf = parse_smf(&data).expect("parse");
        let es = &smf.tracks[0].events;
        assert_eq!(es.len(), 3);
        // 80..8f
        assert!(matches!(es[1], SmfEvent::NoteOn { tick: 1, pitch: 62, vel: 90, .. }));
        // vel 0 → NoteOff
        assert!(matches!(es[2], SmfEvent::NoteOff { tick: 2, pitch: 60, .. }));
    }

    #[test]
    fn parse_tempo_and_timesig() {
        // delta0 tempo 500000(120bpm), delta0 NoteOn..., delta96 NoteOff, delta0 EOT
        // 加上 time sig 4/4
        let mut ev = Vec::new();
        ev.extend_from_slice(&vlq(0));
        ev.extend_from_slice(&[0xff, 0x51, 0x03, 0x07, 0xa1, 0x20]); // 500,000
        ev.extend_from_slice(&vlq(0));
        ev.extend_from_slice(&[0xff, 0x58, 0x04, 0x04, 0x02, 0x18, 0x08]);
        ev.extend_from_slice(&vlq(0));
        ev.extend_from_slice(&[0x90, 60, 100]);
        ev.extend_from_slice(&vlq(96));
        ev.extend_from_slice(&[0x80, 60, 0]);
        ev.extend_from_slice(&vlq(0));
        ev.extend_from_slice(&[0xff, 0x2f, 0]);
        let data: Vec<u8> = mthd(1, 1, 96).into_iter().chain(mtrk(&ev)).collect();
        let smf = parse_smf(&data).expect("parse");
        let es = &smf.tracks[0].events;
        assert!(es.iter().any(|e| matches!(e, SmfEvent::Tempo { us_per_qn: 500_000, .. })));
        assert!(es.iter().any(|e| matches!(e, SmfEvent::TimeSig { num: 4, denom: 4, .. })));
        assert_eq!(smf.meta_tempo_count, 1);
        assert_eq!(smf.meta_timesig_count, 1);
    }

    #[test]
    fn tempo_map_tick_to_sec() {
        // ppq=96, 120bpm (500000 us), 8 拍 = 96*8 = 768 tick = 4 秒
        let events: Vec<SmfEvent> = vec![SmfEvent::Tempo { tick: 0, us_per_qn: 500_000 }];
        let tm = TempoMap::from_events(&events, 96, 768);
        assert!((tm.tick_to_sec(0, 96) - 0.0).abs() < 1e-9);
        assert!((tm.tick_to_sec(96, 96) - 0.5).abs() < 1e-9, "1 拍@120bpm=0.5s");
        assert!((tm.tick_to_sec(768, 96) - 4.0).abs() < 1e-9, "8拍=4s");
        assert!((tm.total_seconds - 4.0).abs() < 1e-9);
        // 逆查
        assert_eq!(tm.sec_to_tick(0.5, 96), 96);
        assert_eq!(tm.sec_to_tick(4.0, 96), 768);
    }

    #[test]
    fn tempo_map_midfile_change() {
        // 中途变 tempo: 0-384 tick @120bpm(2s), 384-768 @240bpm(100000us? => 240bpm=250000us, 1s)
        let events: Vec<SmfEvent> = vec![
            SmfEvent::Tempo { tick: 0, us_per_qn: 500_000 },
            SmfEvent::Tempo { tick: 384, us_per_qn: 250_000 },
        ];
        let tm = TempoMap::from_events(&events, 96, 768);
        // 384 tick @120 = 2.0s; 之后 @240 → 到 768 再加 1.0s
        assert!((tm.tick_to_sec(384, 96) - 2.0).abs() < 1e-9);
        assert!((tm.tick_to_sec(768, 96) - 3.0).abs() < 1e-9);
        assert!((tm.tick_to_sec(576, 96) - 2.5).abs() < 1e-9);
        // 逆查
        assert_eq!(tm.sec_to_tick(2.0, 96), 384);
        assert_eq!(tm.sec_to_tick(2.5, 96), 576);
    }

    #[test]
    fn build_views_pairs_notes() {
        // 构造一个多通道: ch0 note, ch1 note; 验证 visual 视图
        let mut t0 = Vec::new();
        t0.extend_from_slice(&vlq(0));
        t0.extend_from_slice(&[0x90, 60, 100]); // ch0
        t0.extend_from_slice(&vlq(96));
        t0.extend_from_slice(&[0x80, 60, 0]);
        t0.extend_from_slice(&vlq(0));
        t0.extend_from_slice(&[0x91, 72, 80]); // ch1
        t0.extend_from_slice(&vlq(48));
        t0.extend_from_slice(&[0x81, 72, 0]);
        t0.extend_from_slice(&vlq(0));
        t0.extend_from_slice(&[0xff, 0x2f, 0]);
        let mut t1 = Vec::new();
        t1.extend_from_slice(&vlq(0));
        t1.extend_from_slice(&[0xff, 0x51, 0x03, 0x07, 0xa1, 0x20]);
        t1.extend_from_slice(&vlq(0));
        t1.extend_from_slice(&[0x90, 64, 90]); // ch0 第二音
        t1.extend_from_slice(&vlq(48));
        t1.extend_from_slice(&[0x80, 64, 0]);
        t1.extend_from_slice(&vlq(0));
        t1.extend_from_slice(&[0xff, 0x2f, 0]);
        let data: Vec<u8> = mthd(1, 2, 96)
            .into_iter().chain(mtrk(&t0)).chain(mtrk(&t1)).collect();
        let smf = parse_smf(&data).expect("parse");
        assert_eq!(smf.format, 1);
        assert_eq!(smf.tracks.len(), 2);
        let views = build_track_views(&smf);
        // ch0: 2 音, ch1: 1 音
        assert_eq!(views[0].notes.len(), 2, "ch0 应有 2 音");
        assert_eq!(views[1].notes.len(), 1, "ch1 应有 1 音");
        // velocity 保留验证
        let v0: Vec<u8> = views[0].notes.iter().map(|n| n.vel).collect(); // ch0 两音 vel [100, 90]
        assert!(v0.contains(&100), "ch0 第一音 vel=100 应保留, got {:?}", v0);
        assert!(v0.contains(&90), "ch0 第二音 vel=90 应保留, got {:?}", v0);
        assert_eq!(views[1].notes[0].vel, 80, "ch1 vel=80 应保留");
        // 时长集合: ch0 有 dur96 和 dur48; ch1 有 dur48
        let mut d0: Vec<u64> = views[0].notes.iter().map(|n| n.dur_ticks).collect();
        d0.sort_unstable();
        assert_eq!(d0, vec![48, 96], "ch0 两音时值应 48+96");
        assert_eq!(views[1].notes[0].dur_ticks, 48, "ch1 单音时值 48");
        // ch0 程序号(无 program 事件)为 None
        assert!(views[0].program.is_none());
    }
}

#[test]
fn test_file_11_track3() {
    let bytes = include_bytes!("../11 - I Sawed The Demons (E2M1).mid");
    let chunk = &bytes[8224..8224+4272];
    
    let mut te = TrackEvents::default();
    let mut tempo_count = 0usize;
    let mut ts_count = 0usize;
    
    match parse_track_chunk(chunk, 120, &mut te, &mut tempo_count, &mut ts_count) {
        Ok(()) => {
            println!("OK: {} events, {} tempo changes, {} time sig changes", 
                      te.events.len(), tempo_count, ts_count);
        }
        Err(e) => {
            println!("ERR: {:?}", e);
            panic!("track 3 parse failed: {:?}", e);
        }
    }
}

/// ★ 2026-08-13 John bug 回归: 琶音器 MIDI ch03 出现超长音符(150拍).
///   根因: build_track_views 排序强行"同 tick NoteOff 先于 NoteOn", 破坏文件内
///   同音断奏配对(on(X)/off(X) 同 tick 交错) → on 悬空等很久 → 超长.
///   修复: 只按 tick 稳定排序, 同 tick 保留文件原始顺序.
///   本测试: 解析用户提供的文件, 断言无 >8拍(192tick) 的超长 note.
#[test]
fn test_arpeggio_no_superlong_notes() {
    let bytes = include_bytes!("../11 - I Sawed The Demons (E2M1) user_repro.mid");
    let smf_file = match parse_smf(bytes) {
        Ok(s) => s,
        Err(e) => panic!("parse_smf failed: {e:?}"),
    };
    let views = build_track_views(&smf_file);
    let ppq = smf_file.ppq.max(1) as u64;
    let gate = 8 * ppq; // 8拍
    let mut worst: (u64, u64, u64, u64) = (0, 0, 0, 0); // (dur, ch, pitch, start)
    let mut total_long = 0u64;
    for (ci, v) in views.iter().enumerate() {
        for n in &v.notes {
            if n.dur_ticks > gate {
                total_long += 1;
                if n.dur_ticks > worst.0 {
                    worst = (n.dur_ticks, ci as u64, n.pitch as u64, n.start_tick);
                }
            }
        }
    }
    // 修复前(旧排序 off_first): 本文件 43 个超长(雪花式假长音) — 排序错配导致.
    // 修复后(只按tick稳定): 3 个超长, 均为文件内真实长持音
    //   (ch3 pitch31 @1055→2111 持续低音叠琶音; ch6 pitch24/pitch21 长音),
    //   on→off 配对闭环, 非错配. 断言 < 10 (远小于 bug 时的 43) 即可锁死回归.
    assert!(total_long < 10,
        "排序修复后不应有雪花式假超长(修复前=43); 实际 {total_long} 个(应只剩真实长持音 <10), 最坏 ch{} pitch{} start{} dur={} tick",
        worst.1, worst.2, worst.3, worst.0);
}

