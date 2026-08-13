# 可弹奏 Piano Roll + Event List 联动设计

> 日期: 2026-08-13  分支: `feat/playable-piano-eventlist` (未 merge, 等 John 验收)
> 目标: 两大新功能, 均为后续编辑功能做铺垫:
>   1. **Playable Piano Roll** — 点击黑白琴键 / 点击 note 发出对应音符 (MIDI note-on/off 到设备)
>   2. **Event List** — 列出当前 channel 所有 MIDI events, 与 piano roll 联动, 显示在 params 面板下部 (原 PARTS (0/32) 位置)

---

## 1. 背景与现状

### 1.1 数据模型 (src/smf.rs)
- `SmfEvent` (16-25): `NoteOn{..}/NoteOff{..}/Tempo{..}/TimeSig{..}/Cc{..}/Program{..}`, 带绝对 `tick`。
- `Smf.tracks: Vec<TrackEvents>`, 每 `TrackEvents.events: Vec<SmfEvent>` (加载顺序)。
- `build_track_views` → `Vec<SmfTrackView>` (按通道归并, 存 `notes: Vec<SmfNote>` + program/bank/name)。`smf_views[16]` 在 lib.rs:642。
- `SmfNote { channel, pitch, vel, start_tick, dur_ticks }` (333-339) — piano roll / 播放用。

### 1.2 MIDI 输出 (src/playback.rs)
- `PlayEvent` (15-20): `tick, bytes: Vec<u8>, off, channel`. `PlayEvent::note(ch, pitch, vel, tick, on)` 生成 `[0x90|ch, pitch, vel]` (23-32)。
- `dispatch_play_events` (535-572): wasm 下把 `fired` 按输出口分组, `midi_wasm::send_sync(dev, &e.bytes)` 保序发送; native 下静默降级 (571)。
- `send_all_sound_off` (576-599): 16 通道发 CC120+CC123。
- `PlayEvent::cc(ch, num, val)` (35-41): `[0xB0|ch, num, val]`。

### 1.3 Piano Roll 渲染 (src/piano_roll.rs)
- `render_piano_roll(&mut self, ui)` (70-294): 深底 → 标尺 (`draw_time_ruler`) → `ScrollArea::vertical().id_salt("piano_roll_scroll")` (148) → 内容区:
  - 左琴键 `key_rect` (184-187): `c_left..c_left+KEY_W`, 黑键 #1a1a20 / 白键 #e6e6e6, C 标注 (201-215)。
  - 时间轴 `time_rect` (219-222): `time_left..time_left+time_width`, bar/beat 竖线, 单 channel 音符 `pr_notes(ch)` (262-284), playhead (286-292)。
- **当前无任何鼠标交互** — 只有显示。ScrollArea 的 response 未被消费。
- `KEY_W`/`ROW_H`/`MIDI_LOW/HIGH` 常量在文件头部 (需确认); `pr_notes(ch)`(:42-63) 已 `pub(crate)`。

### 1.4 Event List 落点 (src/lib.rs params panel)
- PARTS dump 表在 2410-2429: `ScrollArea::vertical().id_salt("right_parts").max_height(240)` 显示 32-part `read_parts` (MSB/LSB/PC/Name)。
- 用户指定: **用 event list 取代该区域** ("利用 param 面板下部, 就是原来显示 PARTS (0/32) 的地方")。
- Params 面板 `SidePanel::right("params")` (2116), 宽 160..=420px。深色主题 #1f2f45。

### 1.5 联动通道
- Piano roll 当前通道 `cur_pr_channel` (lib.rs:700, 1..16), 顶部 Ch ComboBox (2523)。
- Event list 应跟随 `cur_pr_channel` → 选中行 → 可选: 联动 piano roll 滚动/playhead 定位。

---

## 2. 设计决策

### D1. Playable Piano Roll — 发声模型
- **新增 `preview_notes: [BTreeMap<u8, (u8, f64)>; 16]`** 状态 (通道 → pitch → (velocity, 起声时刻)): 记录"点按发声"的挂音。点击琴键/note → NoteOn; 松开/再次点击同 pitch → NoteOff。
- **复用 `dispatch_play_events` 思路**: 新增 `pub(crate) fn preview_note(&mut self, ch: u8, pitch: u8, vel: u8, on: bool)`:
  - wasm: 构造 `PlayEvent::note(ch, pitch, vel, 0, on)`, 走 `dispatch_play_events(&[ev], now)` 同款输出路由 (含 Mute/Solo 过滤)。
  - native: 静默。
- **vel**: 琴键固定 100; note 点击用音符自身 vel。
- **Mute/Solo**: 预览发声尊重 mute/solo (`channel_is_effectively_muted`), 与播放一致。

### D2. Piano Roll 命中检测
- 琴键区 `key_rect` (fixed width KEY_W, 左缘) + 时间轴区 `time_rect` (右侧)。
- **琴键点击**: `ui.interact(key_rect, id, Sense::click())`; 由 y 反算 pitch: `pitch = MIDI_HIGH - 1 - ((y - key_rect.top()) / ROW_H).floor()`。
- **note 点击**: 遍历可见 notes, 找 `note_rect.contains(pos)` 的 note → 触发该 note `preview_note(ch, pitch, vel, true)`, 再定时 off (或悬空, note 点击是"采样式"发声, 短声)。决策: **note 点击 = 触发一次短 note-on, 然后 300ms 后 note-off** (采样式, 不需要按着), 用 `preview_notes` + frame 检查。
- **冲突**: ScrollArea 已占用交互。做法: 在 ScrollArea 内部用 `ui.interact(note_rect, id, click)` 逐个 note 交互 (egui 支持), 琴键交互也在 ScrollArea 内。注意用独立 `Id` (`("pr_note", i)`)。

### D3. Event List 数据源 & 排序
- **数据源**: `self.smf` (`Smf.tracks` 原始事件) 按 `cur_pr_channel` 过滤 (SmfEvent 不都带 channel: Tempo/TimeSig 是全局, Cc/Program/NoteOn/Off 带 channel)。
- **排序**: 按 `tick` 升序 (同 tick 保持原始轨道相对序)。
- **过滤规则**: 只列当前 `cur_pr_channel` 的 `NoteOn/NoteOff/Program/Cc` events (Tempo/TimeSig 全局, 不在 channel list 显示 — 或者单独 Global 区? 决策: 先只列 channel 事件, Tempo/TimeSig 忽略)。
- **显示格式** (参考 DAW event list):
  ```
  tick    type    data
  00000   NoteOn  C4  v100
  00096   NoteOff C4
  00192   PC      Prog 5
  00288   CC7     Vol 100
  ```
- **note 显示名**: `midi_name(pitch)` (piano_roll 已有, C4 等)。

### D4. Event List ↔ Piano Roll 联动
- **选中 event list 行** → 设置 `event_list_sel: Option<usize>` + 高亮该 event in piano roll (对应 note 用高亮边框/色)。
- **点击 note (piano roll)** → 若该 note 在 event list 数据里, 同步选中对应行 (反向联动)。
- **定位**: 双击 event list 行 → `pr_scroll_ticks` 跳到该 event tick (可看上下文)。
- **playhead**: event list 当前播放位置高亮行 (若 tick 在播放范围) — 可选, 先做选中联动。

### D5. 布局落点 & 视图状态
- Event list 占据 params 面板下部原 PARTS 区 (2410-2429): `ScrollArea::vertical().id_salt("event_list")`, 高度 ~240, 深度主题。
- **PARTS dump 表保留吗?** 用户说"就是原来显示 PARTS (0/32) 的地方" → 语义是把那块区域给 event list。决策: **PARTS 表移到可折叠小节或移除顶部计数行, event list 占主区域**。稳妥: 加个 section 分隔符 + "EVENTS (ch N)" 标题, PARTS 表仍在但默认收进 `ScrollArea` 底部 (用 `CollapsingHeader`)。**先实现 event list 为主区, PARTS 用 CollapsingHeader 收起**。
- **视图状态**: `event_list_sel: Option<usize>` 在 XgApp 新增。

### D6. 状态字段汇总 (lib.rs XgApp 新增)
- `preview_notes: Vec<BTreeMap<u8, (u8, f64)>>` (16 通道 → pitch → (vel, t_ms)) — 挂音跟踪。
- `event_list_sel: Option<usize>` — 选中 event list 行索引 (指向过滤后 Vec)。
- `event_list_autoscroll: bool` — 播放时跟踪 playhead 滚动 (可选, 默认 true)。

---

## 3. 实现步骤

### Phase A: Playable Piano Roll (先做, 独立可验收)
1. `XgApp::preview_note(ch, pitch, vel, on)` + `preview_notes` 跟踪 + 输出路由 (paraphase dispatch).
2. Piano roll 琴键区交互: 点击下发 NoteOn, 松开/移出 NoteOff.
3. Piano roll note 区交互: 点击 → 采样发声 (NoteOn→300ms→NoteOff).
4. 测试: preview_note 构造 bytes 正确 / mute 过滤 / 命中 pitch 换算纯函数.

### Phase B: Event List (联动)
5. `event_list_for(ch)` → 过滤+排序 `Vec<&SmfEvent>` (或轻量拷贝).
6. Params 面板下部渲染 event list (ScrollArea), PARTS 表 CollapsingHeader 收起.
7. 选中行 ↔ piano note 高亮联动 + 双击定位 pr_scroll.
8. 测试: 过滤/排序/联动索引.

---

## 4. 验证计划 (程序化)
- `cargo test` 100+ 绿 (新增 playable/eventlist 单测).
- wasm 重建 + playwright 验证:
  - piano roll 点击琴键坐标 → console/meter 有 NoteOn 证据 (headless 无设备, 验证不 panic + 状态变化).
  - event list 渲染: params 区出现 EVENTS 标题与行, 像素/纯逻辑判定.
- 真机 MIDI 发声: John 本地 :8090 + Web MIDI 连 MU90 验证听感 (等 John 验收).

## 5. 关键风险 / 注意
- **ScrollArea 交互**: egui 里 ScrollArea 内部 UI 仍可用 `ui.interact` 自定义命中, 但拖动滚动与点击需要 `.sense` 区分; note 用 click (不拖), 琴键用 click.
- **预览发声 vs 播放**: 预览时若正在播放, 挂音用 preview_notes 独立管理, 不与 play_events 冲突; Stop 时一并清 (send_all_sound_off 已全清).
- **事件序/tick 0 冲突**: NoteOff@同tick 的显示, event list 要真实反映文件序 (不重排).
- **wasm 预览发声**: headless 无 MIDI 设备 → send_sync 静默失败? 需确认不影响 (send_sync 失败只是 log, 不 panic).
