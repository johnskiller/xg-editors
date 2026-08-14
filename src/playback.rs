// PlayView 播放画面 (CentralView::PlayView) 的渲染 + 数据层.
// 从 lib.rs 拆出 (Step 1): 音序器播放引擎 + PlayEvent 数据定义.
// 依赖: XgApp 结构体字段 (共享状态, 单 struct 多文件 impl), 无需 channel 解耦.

use crate::XgApp;
use eframe::egui;
use crate::smf;
use crate::midi_wasm;

/// 全局状态日志缓冲的转发 (playback 事件驱动的日志走得少, 但保持与 lib 一致).
/// console.log 诊断 (仅 wasm 生效).

/// 播放时排队的 MIDI 事件: on/off 字节 + 触发 tick + 通道
#[derive(Debug, Clone, PartialEq)]
pub struct PlayEvent {
    pub tick: u64,     // 触发时刻 (绝对 tick)
    pub bytes: Vec<u8>, // 完整 MIDI 消息 (NoteOn/NoteOff)
    pub off: bool,      // true=NoteOff (用于 pending 去重)
    pub channel: u8,
}

impl PlayEvent {
    pub(crate) fn note(channel: u8, pitch: u8, velocity: u8, tick: u64, on: bool) -> Self {
        let status = 0x90 | (channel & 0x0f);
        let vel = if on { velocity & 0x7f } else { 0x00 };
        Self {
            tick,
            bytes: vec![status, pitch & 0x7f, vel],
            off: !on,
            channel,
        }
    }

    /// Control Change (Bank Select 等), 用于播放前通道配置
    pub(crate) fn cc(channel: u8, num: u8, val: u8) -> Self {
        Self {
            tick: 0,
            bytes: vec![0xB0 | (channel & 0x0f), num & 0x7f, val & 0x7f],
            off: false,
            channel: channel & 0x0f,
        }
    }

    /// Control Change 带显式 tick (mid-file CC: pan CC10 / vol CC7 等), 按原曲时间发送
    pub(crate) fn cc_tick(channel: u8, num: u8, val: u8, tick: u64) -> Self {
        Self {
            tick,
            bytes: vec![0xB0 | (channel & 0x0f), num & 0x7f, val & 0x7f],
            off: false,
            channel: channel & 0x0f,
        }
    }

    /// Program Change, 用于播放前通道配置
    pub(crate) fn prog(channel: u8, program: u8) -> Self {
        Self {
            tick: 0,
            bytes: vec![0xC0 | (channel & 0x0f), program & 0x7f],
            off: false,
            channel: channel & 0x0f,
        }
    }
}

/// 计算时刻 playhead_tick 正在响的 (channel, pitch) 集合 (用于清音).
/// play_events 已按 tick 升序; 同 (ch,pitch) 的 on/off 按时间先后出现.
/// 事件 tick > playhead 未到, 不算. 对同一 (ch,pitch) 若 on 后又有 off 则不再响.
pub(crate) fn notes_active_at(pt: u64, play_events: &[PlayEvent]) -> std::collections::BTreeSet<(u8, u8)> {
    use std::collections::BTreeSet;
    let mut active: std::collections::BTreeMap<(u8, u8), u64> = std::collections::BTreeMap::new();
    for e in play_events {
        if e.tick > pt {
            break;
        }
        let key = (e.channel, e.bytes[1]);
        if !e.off {
            active.insert(key, e.tick);
        } else if active.get(&key).is_some() {
            active.remove(&key);
        }
    }
    active.into_iter().map(|(k, _)| k).collect()
}


impl XgApp {
    /// Play 按下: 从头/当前位置开始播放 (构建事件表 + 起始时刻)。
    /// 当前 UI 用 play_resume (不清 playhead); 本方法保留供"从某处开始播放"场景。
    #[allow(dead_code)]
    pub fn play_playhead_start(&mut self) {
        self.build_play_events();
        self.last_play_frame_ms = 0.0; // 让下一帧以 now 为基准
        if let Some(tm) = &self.tempo_map {
            let ppq = self.smf.as_ref().map(|s| s.ppq).unwrap_or(self.ppq as u32);
            self.play_real_sec = tm.tick_to_sec(self.playhead_tick, ppq);
        } else {
            self.play_real_sec = 0.0;
        }
        self.playing = true;
    }

    /// Resume: 从暂停位置续播. 不清 playhead, 不重建事件表 (Pause 后 Play 不应从头).
    /// 用户的期望: Pause → Play 从停处继续 (2026-08-09).
    pub fn play_resume(&mut self) {
        if self.play_events.is_empty() {
            self.build_play_events();
        }
        self.last_play_frame_ms = 0.0; // 以 now 为基准, 避免 resume 时 dt 突变
        if let Some(tm) = &self.tempo_map {
            let ppq = self.smf.as_ref().map(|s| s.ppq).unwrap_or(self.ppq as u32);
            self.play_real_sec = tm.tick_to_sec(self.playhead_tick, ppq);
        } else {
            let bpm = self.tempo_bpm.max(1.0);
            self.play_real_sec = self.playhead_tick as f64 * 60.0 / (self.ppq.max(1) as f64 * bpm);
        }
        self.playing = true;
    }

    /// 追加状态日志 (底部 status 栏显示最近一条; 循环缓冲最多 50 条).
    pub fn log_status(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        if self.status_log.len() >= 50 {
            self.status_log.pop_front();
        }
        self.status_log.push_back(m);
    }

    /// 加载 SMF 字节流: 解析 → 视图 → tempo map → 总时长. 失败返回错误字符串.
    pub fn load_smf_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<String, String> {
        crate::console_log("LOAD", format!("loading {} ({} bytes)", name, bytes.len()));
        let smf_file = match smf::parse_smf(bytes) {
            Ok(s) => s,
            Err(e) => {
                crate::console_log("LOAD", format!("parse failed: {:?}", e));
                return Err(format!("SMF parse: {e}"));
            }
        };
        crate::console_log("LOAD", format!("parsed: {} tracks, ppq={}", smf_file.tracks.len(), smf_file.ppq));
        let views = smf::build_track_views(&smf_file);
        // 所有轨最长 tick (end of notes) → 时间轴范围
        let mut end_tick = 0u64;
        for v in &views {
            for n in &v.notes {
                end_tick = end_tick.max(n.start_tick + n.dur_ticks);
            }
        }
        // 收集全部事件做 tempo map
        let all: Vec<smf::SmfEvent> = smf_file
            .tracks
            .iter()
            .flat_map(|t| t.events.iter().cloned())
            .collect();
        let tm = smf::TempoMap::from_events(&all, smf_file.ppq, end_tick);
        let smf_ppq = smf_file.ppq as u64; // 在 move 前取 (smf_file 稍后存入 self.smf)
        self.smf_name = name.to_string();
        self.smf = Some(smf_file);
        self.smf_views = views;
        // 缓存内嵌权威音色表 (只建一次; program→音色名 用)
        if self.voice_bank.is_none() {
            self.voice_bank = crate::data::VoiceBank::embedded_mu90().ok();
        }
        // 预填 16 通道音色名 (program + bank → XG/MU90 权威音色名)
        // ch10 (index 9) 是 XG 强制鼓通道: 默认 Standard Kit, 但有 bank(msb=127)/PC 时响应实际鼓组;
        // 其它通道: 有 bank 按 find(msb,prg,lsb), 无 bank 按 xg_by_prg (msb=0 旋律区)
        self.live_bank = [(0u8, 0u8); 16];
        self.live_program = [0u8; 16];
        for (i, v) in self.smf_views.iter().enumerate() {
            // 同步 live_bank/live_program ← SMF 解析出的真实 bank/program
            if let Some((msb, lsb)) = v.bank {
                self.live_bank[i] = (msb, lsb);
            }
            if let Some(p) = v.program {
                self.live_program[i] = p;
            }
            // 单源化: 同时同步到 parts[i] (唯一数据源)
            if let Some(part) = self.parts.get_mut(i) {
                part.msb = v.bank.map(|(msb, _)| msb).unwrap_or(0);
                part.lsb = v.bank.map(|(_, lsb)| lsb).unwrap_or(0);
                part.prog = v.program.unwrap_or(0);
                // 音色名已在下面计算，赋值给 part.voice
            }
            let name = match (i + 1 == 10, v.bank) {
                // 鼓通道: bank.msb == 127 → 鼓组 (LCD 8字符短名); 否则 (含 None) 默认 Standard Kit
                (true, Some((msb, lsb))) if msb == 127 => v.program
                    .map(|p| crate::data::drum_display_name(p).to_string())
                    .unwrap_or_else(|| crate::data::drum_display_name(0).to_string()),
                (true, _) => crate::data::drum_display_name(0).to_string(),
                // 旋律通道: 有 bank → 按 (msb,prg,lsb); 无 → msb=0 旋律区
                (false, Some((msb, lsb))) => v.program
                    .and_then(|prg| self.voice_bank.as_ref().and_then(|b| b.find(msb, prg, lsb)))
                    .map(|vo| vo.name.clone())
                    .unwrap_or_else(|| "GrandPno".to_string()),
                (false, None) => v.program
                    .and_then(|prg| self.voice_bank.as_ref().and_then(|b| b.xg_by_prg(prg)))
                    .map(|vo| vo.name.clone())
                    .unwrap_or_else(|| "GrandPno".to_string()),
            };
            self.live_voice_names[i] = name.clone();
            // 单源化: 同步音色名到 parts[i]
            if let Some(part) = self.parts.get_mut(i) {
                part.voice = name;
            }
        }
        self.live_levels = [0.0; 16];
        self.live_volumes = [1.0; 16]; // 重新加载: 音量重置 (CC7 播放时覆盖)
        self.live_expressions = [1.0; 16];
        self.raw_vel_peaks = [0.0; 16];
        self.cc_live = [[0u8; 128]; 16];
        self.play_evt_count = 0;
        self.max_poly = 0;
        self.live_vel_peaks = [0.0; 16];
        self.live_master_vol = 1.0;
        self.active_notes.iter_mut().for_each(|m| m.clear());
        self.tempo_map = Some(tm.clone());
        // 用 SMF 真实 ppq + 初始 tempo (John 发现: 之前用默认 96/120, 导致 bar 数 86 vs Logic 69、
        // BPM 120 vs 95 全是错的)
        self.ppq = smf_ppq;
        self.tempo_bpm = 60_000_000.0 / tm.us_per_qn.max(1) as f64;
        self.smf_end_tick = end_tick;
        self.smf_total_sec = tm.total_seconds;
        // 自动取景: 显示整曲 (scroll=0); zoom 语义 = "1x = 全区正好充满 view"
        // (32768 tick 全曲: 1x → win_ticks=end_tick 正好显示全区)
        // (除非 URL 调试钩子显式给了 zoom/scroll, 则保持用户/截图指定的取景)
        if !self.url_override_view {
            self.track_view_scroll_ticks = 0;
            self.track_view_zoom = 1.0; // 1x = fit 全区
        }
        self.smf_is_dirty = true;
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&format!("LOADDBG end_tick={end_tick} zoom={} query={:?}",
            self.track_view_zoom,
            web_sys::window().and_then(|w| w.location().search().ok())).into());
        // 播放位置重置
        self.playhead_tick = 0;
        self.playing = false;
        self.event_cursor = 0;
        self.event_cursor_origin = 0;
        let summary = format!(
            "OK: {} track, {} channels, {:.1}s, endTick {}",
            self.smf.as_ref().map(|s| s.ntracks).unwrap_or(0),
            self.smf_views.iter().filter(|v| !v.notes.is_empty()).count(),
            tm.total_seconds,
            end_tick,
        );
        // wasm: 把状态写进 DOM (dump-dom/screenshot 可读, 不依赖 canvas GPU)
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                let _ = doc.set_title(&format!("XG Editor v{} [{}] {}", self.app_version, self.smf_name, summary));
            }
        }
        // 底部 status 日志记录加载摘要
        self.log_status(summary.clone());
        Ok(summary)
    }

    /// playhead → (bar, beat, tick) 显示. ppq 四分音符, 4/4 → 每 4 拍 = 1 小节
    /// ★ 2026-08-13 John: beat 从 1 开始 (1,2,3,4), 不显示 0.
    pub fn playhead_bar_beat(&self) -> (u64, u64, u64) {
        let beat = self.playhead_tick / self.ppq.max(1);
        let bar = beat / 4;
        let beat_in_bar = (beat % 4) + 1; // 1-based: 1,2,3,4
        let tick_in_beat = self.playhead_tick % self.ppq.max(1);
        (bar + 1, beat_in_bar, tick_in_beat)
    }

    // ---------- 音序器播放引擎 ----------

    /// 从当前轨道构建排序事件表 (NoteOn + NoteOff), 覆盖 0..total_ticks.
    /// Play/Stop 或轨音符变化时重建. 有 SMF 时用 SMF 音符; 否则用默认 16 轨 pattern.
    pub fn build_play_events(&mut self) {
        let mut evs: Vec<PlayEvent> = Vec::new();
        // 音符来源: SMF(逐通道视图) or 默认 pattern
        let total = self.total_ticks.max(1);
        let has_smf = self.smf.is_some();
        if has_smf {
            let endt = self.smf_end_tick.max(1);
            for view in &self.smf_views {
                for n in &view.notes {
                    if n.dur_ticks == 0 {
                        continue;
                    }
                    let start = n.start_tick % endt;
                    let end = start + n.dur_ticks;
                    evs.push(PlayEvent::note(n.channel, n.pitch, n.vel, start, true));
                    if end > endt {
                        evs.push(PlayEvent::note(n.channel, n.pitch, 0, endt, false));
                        evs.push(PlayEvent::note(n.channel, n.pitch, n.vel, 0, true));
                    } else {
                        evs.push(PlayEvent::note(n.channel, n.pitch, 0, end, false));
                    }
                }
            }
            self.total_ticks = endt; // 播放范围切到 SMF 时长
        } else {
            for t in &self.tracks {
                for n in &t.notes {
                    if n.dur_ticks == 0 {
                        continue;
                    }
                    let start = n.start_tick % total;
                    let end = start + n.dur_ticks;
                    evs.push(PlayEvent::note(n.channel, n.pitch, n.velocity, start, true));
                    if end > total {
                        evs.push(PlayEvent::note(n.channel, n.pitch, 0, total, false));
                        evs.push(PlayEvent::note(n.channel, n.pitch, n.velocity, 0, true));
                    } else {
                        evs.push(PlayEvent::note(n.channel, n.pitch, 0, end, false));
                    }
                }
            }
        }
        // ---- 播放通道配置注入: 保证每个有音符的通道先设好音色/bank (GM/XG 语义) ----
        // 关键: ch10(0-based 9) 是鼓专用 → 必须 Bank MSB 127 (Drum kit), doom.mid 只带 PC 没带 bank
        // 若不注入, ch10 会用当前 melody 音色发声或按 MU90 记忆的鼓组, 不可靠
        // 其它通道: SMF 里带了 Program 则沿用, 否则默认 PC=0 (GM 默认音色由面板决定)
        let mut chans_with_notes: std::collections::BTreeMap<u8, ()> = std::collections::BTreeMap::new();
        for e in &evs {
            if !e.off {
                chans_with_notes.insert(e.channel, ());
            }
        }
        // SMF 各通道的 Program (tick0 前有效) — 收集最近一次 Program 事件
        if has_smf {
            let mut prog: [Option<u8>; 16] = [None; 16];
            for track in self.smf.as_ref().unwrap().tracks.iter() {
                for ev in &track.events {
                    if let smf::SmfEvent::Program { channel, program, .. } = ev {
                        prog[*channel as usize] = Some(*program);
                    }
                }
            }
            for ch in chans_with_notes.keys() {
                let ch = *ch;
                if ch == 9 {
                    // 鼓通道: MSB=127 (Drum), LSB=0, PC = SMF 给的或默认 Standard(0)
                    let pc = prog[ch as usize].unwrap_or(0);
                    evs.push(PlayEvent::cc(ch, 0, 127));   // Bank Select MSB = 127 (Drum)
                    evs.push(PlayEvent::cc(ch, 32, 0));    // Bank Select LSB = 0
                    evs.push(PlayEvent::prog(ch, pc));
                } else if let Some(p) = prog[ch as usize] {
                    evs.push(PlayEvent::prog(ch, p));
                }
            }
            // mid-file CC: pan(10)/vol(7)/expr(11)/sustain(64)... 全部按原曲 tick 发送.
            // 过滤已被注入覆盖的 Bank Select MSB(0)/LSB(32) —— ch10 的 bank 注入不该被
            // 文件里可能存在的 bank 覆盖回 melodic. 其余 CC 全部保留(每个通道的 pan/vol 等).
            let smf_ref = self.smf.as_ref().unwrap();
            let endt = self.smf_end_tick.max(1); // 第二个 has_smf 块, endt 需重取
            for track in smf_ref.tracks.iter() {
                for ev in &track.events {
                    if let smf::SmfEvent::Cc { tick, channel, num, val } = ev {
                        if *num == 0 || *num == 32 {
                            continue; // bank select 由注入统一管(ch10 强制鼓 bank)
                        }
                        let t = (*tick).min(endt); // 不取模! 音符才需要 mod 回绕, CC 本就在 0..endt
                        evs.push(PlayEvent::cc_tick(*channel & 0x0f, *num, *val, t));
                    }
                }
            }
        }
        evs.sort_by_key(|e| e.tick);
        self.play_events = evs;
        self.event_cursor = 0;
        self.event_cursor_origin = 0;
    }

    /// 播放推进一帧: 按真实时间增量推进 playhead, 并把 (光标..playhead] 到期的
    /// 事件通过 Web MIDI send_to 发送 (wasm); native/无设备静默降级.
    pub(crate) fn tick_playback(&mut self, ctx: &egui::Context) {
        let now_ms = ctx.input(|i| i.time) * 1000.0;
        let last = if self.last_play_frame_ms > 0.0 { self.last_play_frame_ms } else { now_ms };
        self.last_play_frame_ms = now_ms;
        // 长停顿检测: 浏览器后台时 rAF 被节流 → dt 巨大. 音频播放器语义 = 后台暂停,
        // 切回从停点续播 (不追丢时间, 不 burst 灌事件). 实现: 清掉后台残留音, dt 按 0 处理.
        // (前台偶尔一帧卡 dt 可能几百 ms, 但 <500ms 不像后台; 用 >500ms 判"切回".)
        let dt_orig = now_ms - last;
        if dt_orig > 500.0 {
            // 后台切回: 清后台期间设备上未停的音 (播放器后台没发消息, 音符挂住)
            // 用户验证: 只 CC120/123 不够, 需对当前响的 note 逐发 NoteOff → kill_current_notes
            self.kill_current_notes();
            // 不推进 playhead, 从停点续播 (相当于后台暂停)
            self.last_play_frame_ms = now_ms; // 重置, 下一帧用真实帧
            return;
        }
        let dt_ms = dt_orig.max(0.0).min(100.0);
        if dt_ms <= 0.0 {
            return;
        }
        // 若 SMF 有 tempo map → 用真实秒驱动 tick (支持中途变 tempo)
        // 否则用固定 bpm → tick 换算
        let ppq_eff = self.smf.as_ref().map(|s| s.ppq as u64).unwrap_or(self.ppq) as f64;
        let next = if let Some(tm) = &self.tempo_map {
            self.play_real_sec += dt_ms / 1000.0;
            let t = tm.sec_to_tick(self.play_real_sec, ppq_eff as u32);
            if t > self.smf_end_tick {
                // 播完重头 (PPT 式循环)
                self.play_real_sec = 0.0;
                0
            } else {
                t
            }
        } else {
            let dt_ticks = (dt_ms * self.ppq as f64 * self.tempo_bpm / 60000.0).round() as u64;
            (self.playhead_tick + dt_ticks) % self.total_ticks.max(1)
        };
        let prev = self.playhead_tick;
        // 回绕/跳变检测
        if next < prev {
            self.event_cursor = 0;
            self.event_cursor_origin = 0;
        }
        self.playhead_tick = next;
        if self.play_events.is_empty() {
            self.build_play_events();
        }
        // 消费 (光标..playhead] 到期事件
        let origin = self.event_cursor_origin;
        let mut fired: Vec<PlayEvent> = Vec::new();
        let mut i = self.event_cursor;
        while i < self.play_events.len() {
            let ev = &self.play_events[i];
            if ev.tick >= origin && ev.tick <= next {
                fired.push(ev.clone());
                i += 1;
            } else if ev.tick > next {
                break;
            } else {
                i += 1;
            }
        }
        self.event_cursor = i;
        self.event_cursor_origin = next;
        // 消费事件 → 更新 active_notes / volumes / expressions
        for ev in &fired {
            self.apply_fired_event_to_meter(ev);
        }
        // FakeMu 式电平表平滑: 每帧做 attack/decay 逼近目标 (mimicStrength, octavia basic/index.mjs)
        self.tick_meter_smoothing();
        // 发送
        if !fired.is_empty() {
            self.dispatch_play_events(&fired, now_ms);
        }
    }

    /// 单帧电平表平滑: attack/decay 逼近目标 (FakeMu mimicStrength).
    /// 目标 = smooth_meter_target; diff>0 快(×0.8 剩余), diff<0 慢(×0.2 剩余).
    pub(crate) fn tick_meter_smoothing(&mut self) {
        for ch in 0..16 {
            let target = self.smooth_meter_target(ch);
            let diff = target - self.live_vel_peaks[ch];
            if diff >= 0.0 {
                self.live_vel_peaks[ch] += diff * 0.8;
            } else {
                self.live_vel_peaks[ch] += diff * 0.2;
            }
            self.live_levels[ch] = self.live_vel_peaks[ch];
        }
    }

    /// 通道 ch 的平滑目标电平 = raw_vel_peaks × CC7 × CC11 × master (FakeMu getStrengths 公式)
    /// Mute/Solo 静音的通道 → 目标 0 (John 2026-08-13: mute 后电平表归零)
    pub(crate) fn smooth_meter_target(&self, ch: usize) -> f32 {
        if self.channel_is_effectively_muted(ch) {
            return 0.0;
        }
        (self.raw_vel_peaks[ch] * self.live_volumes[ch]
            * self.live_expressions[ch] * self.live_master_vol)
            .clamp(0.0, 1.0)
    }

    /// 单个到期事件 → 电平表/音量状态:
    /// NoteOn → active_notes 插入 + 重算 raw_vel_peaks[ch];
    /// NoteOff → active_notes 移除 + 重算 raw_vel_peaks[ch];
    /// CC → cc_live[ch][num] 记录全量 (PlayView ccVis 数据源); CC7/11 → live_volumes/expressions;
    /// CC0/32 → live_bank (Bank Select); PC → live_program;
    /// 平滑(attack/decay)由 tick_playback 帧循环统一做 (FakeMu mimicStrength).
    pub(crate) fn apply_fired_event_to_meter(&mut self, ev: &PlayEvent) {
        if ev.channel >= 16 { return; }
        let ch = ev.channel as usize;
        let status = ev.bytes.first().copied().unwrap_or(0);
        match status & 0xF0 {
            0xB0 => {
                if let (Some(&num), Some(&val)) = (ev.bytes.get(1), ev.bytes.get(2)) {
                    self.cc_live[ch][num as usize] = val;
                    match num {
                        7 => self.live_volumes[ch] = (val as f32) / 127.0,      // volume
                        11 => self.live_expressions[ch] = (val as f32) / 127.0,  // expression
                        0 => { // Bank MSB
                            self.parts[ch].msb = val;
                        }
                        32 => { // Bank LSB
                            self.parts[ch].lsb = val;
                        }
                        _ => {}
                    }
                }
            }
            0xC0 => {
                if let Some(&prog) = ev.bytes.get(1) {
                    self.parts[ch].prog = prog;
                }
            }
            0x90 => {
                let pitch = ev.bytes.get(1).copied().unwrap_or(0);
                if ev.off {
                    // NoteOff: 移除该 pitch
                    self.active_notes[ch].remove(&pitch);
                } else {
                    // NoteOn (vel>0): 插入 (pitch, vel)
                    let vel = ev.bytes.get(2).copied().unwrap_or(0);
                    if vel > 0 {
                        self.active_notes[ch].insert(pitch, vel);
                    }
                }
                // 重算该通道按住音符中最大 velocity/127
                self.raw_vel_peaks[ch] = self.active_notes[ch]
                    .values().copied().max().unwrap_or(0) as f32 / 127.0;
                // 复音峰值保持 (顶部信息栏 maxPoly)
                let poly = self.active_notes.iter().map(|m| m.len()).sum::<usize>() as u64;
                if poly > self.max_poly {
                    self.max_poly = poly;
                }
            }
            _ => {}
        }
        // 事件计数 (顶部信息栏 events 字段)
        self.play_evt_count += 1;
    }

    /// 把到期事件发到 MIDI 设备. wasm: 单任务顺序发送 (缓存 output 后同步 send 保序);
    /// native stub 忽略.
    /// 按 part 路由: event.channel (0-15) → part → topology.route_for_part 选输出.
    /// 若该 part 无路由 (单接口 Port B 未接), 回退 broadcast 到 active_outputs.
    pub(crate) fn dispatch_play_events(&mut self, fired: &[PlayEvent], _now_ms: f64) {
        #[cfg(target_arch = "wasm32")]
        {
            let evs: Vec<PlayEvent> = fired.to_vec();
            // 预收集每个事件的输出名单 (去重后共用的口)
            let mut plan: Vec<(String, Vec<PlayEvent>)> = Vec::new();
            for e in &evs {
                // Channel View Mute/Solo 过滤 (播放输出层): 被静音的通道事件直接跳过, 不发到 MIDI
                let ch = (e.channel as usize) % 16;
                if self.channel_is_effectively_muted(ch) {
                    continue;
                }
                let part = (e.channel as u8) % 32;
                let target = self.midi_topology.route_for_part(part)
                    .map(|r| r.name.clone())
                    .or_else(|| self.active_outputs().first().cloned());
                if let Some(t) = target {
                    if let Some(slot) = plan.iter_mut().find(|(n, _)| *n == t) {
                        slot.1.push(e.clone());
                    } else {
                        plan.push((t, vec![e.clone()]));
                    }
                }
            }
            if !plan.is_empty() {
                // 单任务顺序发送: 对每个输出口, 该口名下的事件保序发送
                wasm_bindgen_futures::spawn_local(async move {
                    for (dev, dev_evs) in plan {
                        for e in &dev_evs {
                            let _ = midi_wasm::send_sync(&dev, &e.bytes).await;
                        }
                    }
                });
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = fired; // native: midi_wasm stub 静默
    }

    /// Playable Piano Roll (2026-08-13): 点按琴键/音符 → 即时发声预览.
    /// on=true 发 NoteOn 并登记挂音; on=false 发 NoteOff 并移除挂音.
    /// 挂音条目 (pitch → (vel, t0)): t0<0 表示"按住未放"(琴键, 只有 on=false 才 off);
    ///   t0>=0 表示"采样式短音"(note/琴键点击, 由 expire_preview_notes 超时自动 off).
    /// ★ ch 为 0-based MIDI 通道 (0..15; UI 1-based 值在调用处 -1, 与 PlayEvent::note 一致).
    /// 尊重 Mute/Solo (按 0-based idx 查 16 槽); native 或 Web MIDI 全静默降级.
    pub fn preview_note(&mut self, ch: u8, pitch: u8, vel: u8, on: bool, t0: f64) {
        let idx = (ch % 16) as usize;
        // Mute/Solo: 被静音的通道预览不发声 (与播放输出层一致)
        if self.channel_is_effectively_muted(idx) {
            return;
        }
        let p = pitch & 0x7f;
        if on {
            self.preview_notes[idx].insert(p, (vel & 0x7f, t0));
        } else {
            self.preview_notes[idx].remove(&p);
        }
        let ev = PlayEvent::note(ch % 16, p, vel, 0, on);
        #[cfg(target_arch = "wasm32")]
        {
            let part = (ch as u8) % 32;
            let target = self.midi_topology.route_for_part(part)
                .map(|r| r.name.clone())
                .or_else(|| self.active_outputs().first().cloned());
            if let Some(dev) = target {
                let bytes = ev.bytes.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    let _ = midi_wasm::send_sync(&dev, &bytes).await;
                });
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = ev; // native stub
    }

    /// 采样式短音自动关闭: 对 t0>=0 且 (now - t0) > 0.30s 的挂音发 NoteOff (不碰按住未放的 t0<0 琴键).
    /// update() 每帧调用 (30ms repaint), now = egui ctx.time (秒).
    pub fn expire_preview_notes(&mut self, now: f64) {
        // 先全量收集过期音符, 再逐个发 off (避免 self.preview_note(&mut) 与借用冲突)
        let mut expired_all: Vec<(u8, u8, u8)> = Vec::new();
        for (ch, map) in self.preview_notes.iter().enumerate() {
            for (&p, &(v, t0)) in map.iter() {
                if t0 >= 0.0 && (now - t0) > 0.30 {
                    expired_all.push((ch as u8, p, v));
                }
            }
        }
        for (ch, p, v) in expired_all {
            self.preview_note(ch, p, v, false, 0.0);
        }
    }

    /// 停止播放时清除所有挂音: 对所有 16 通道发 All Notes Off (CC123) + All Sound Off (CC120).
    /// 防止"按下 Stop 后仍有音符持续响"(用户报告) —— 播放停止不静音设备, 需要显式清音.
    pub fn send_all_sound_off(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            let outs = self.active_outputs();
            if !outs.is_empty() {
                let mut msgs: Vec<Vec<u8>> = Vec::with_capacity(32);
                for ch in 0..16u8 {
                    // CC120 = All Sound Off (立即静音, 不收 pedal 影响)
                    msgs.push(vec![0xB0 | ch, 120, 0]);
                    // CC123 = All Notes Off (正常释放)
                    msgs.push(vec![0xB0 | ch, 123, 0]);
                }
                wasm_bindgen_futures::spawn_local(async move {
                    for dev in &outs {
                        for m in &msgs {
                            let _ = midi_wasm::send_sync(dev, m).await;
                        }
                    }
                });
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {}
    }

    /// 对单个 MIDI 通道发 All Notes Off (CC123) + All Sound Off (CC120).
    /// Channel View mute/solo 触发时清除该通道挂音 (DAW 行为: mute 后立即不响, 无残留).
    pub fn sound_off_channel(&self, ch: u8) {
        #[cfg(target_arch = "wasm32")]
        {
            if ch >= 16 { return; }
            let outs = self.active_outputs();
            if !outs.is_empty() {
                let msgs: Vec<Vec<u8>> = vec![
                    vec![0xB0 | ch, 120, 0], // All Sound Off
                    vec![0xB0 | ch, 123, 0], // All Notes Off
                ];
                wasm_bindgen_futures::spawn_local(async move {
                    for dev in &outs {
                        for m in &msgs {
                            let _ = midi_wasm::send_sync(dev, m).await;
                        }
                    }
                });
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {}
    }

    /// Mute/Solo 状态变更后, 对所有「现在被静音」的通道补发清音 (All Notes/Sound Off).
    /// 否则已响的挂音不会因为 mute/solo 而停下 (DAW 行为必须: mute 后立即静).
    pub fn sync_sound_off_for_muted_channels(&self) {
        for ch in 0..16 {
            if self.channel_is_effectively_muted(ch) {
                self.sound_off_channel(ch as u8);
            }
        }
    }

    /// 停止/后台暂停时彻底清掉正在响的 note.
    /// 用户验证 (2026-08-09): 只发 CC120/123 不够, MU90 上仍有长音残留 →
    /// 需要针对当前正在响的 (channel, pitch) 逐发 NoteOff.
    /// 实现: 基于 play_events 计算此刻 (playhead_tick) 仍有发声的音符, 补发显式 NoteOff.
    /// 比盲发 16ch×128pitch 精准得多 (doom 播放中同时最多 <64 个 note).
    pub fn kill_current_notes(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            self.send_all_sound_off();
            let outs = self.active_outputs();
            if !outs.is_empty() {
                let pt = self.playhead_tick;
                let active = notes_active_at(pt, &self.play_events);
                let mut msgs: Vec<Vec<u8>> = Vec::new();
                for (ch, pitch) in active {
                    let status = 0x90 | (ch & 0x0f);
                    msgs.push(vec![status, pitch & 0x7f, 0]);
                }
                wasm_bindgen_futures::spawn_local(async move {
                    for dev in &outs {
                        for m in &msgs {
                            let _ = midi_wasm::send_sync(dev, m).await;
                        }
                    }
                });
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {}
    }
}
