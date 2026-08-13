# Channel View per-channel Mute / Solo 设计 (2026-08-13)

**状态**: 设计定稿已确认 (John 2026-08-13)
**负责人**: XG Editor
**基线**: main @ `1aa4f74` (v0.1.21, Pages 在线), 91/91 测试绿

## 1. 背景 / 目标

Channel View(中央 Channel 页, 每行 = 1 MIDI channel)目前只有音色/电平/音符可视化,
无法单独静音某个通道来混音试听。需求: **每个 channel 行加 Mute / Solo 两个开关**:

- **Mute(ch)** — 该通道播放时静音 (音符不发送, 电平表归零)
- **Solo(ch)** — 只有被 solo 的通道发声; 其余通道全部静音
- 互斥规则: Mute 与 Solo 各自独立, 但**同时生效时 Mute 优先** (符合 DAW 惯例)
  - 有任一 channel solo → 非 solo 通道全部视为 muted
  - 无 solo → 各通道按自身 Mute 状态
- **不改变 SMF 数据/parts 音色** — 纯播放输出层的过滤, 可撤销 (点掉即恢复)

## 2. 数据模型

### 2.1 新字段 (XgApp)

```rust
/// 16 通道 Mute 状态 (1..16 → 下标 0..15; true=静音, 仅播放输出层)
pub channel_mutes: [bool; 16],
/// 16 通道 Solo 状态 (true=独奏; 任一 solo 激活时, 其他通道当 muted)
pub channel_solos: [bool; 16],
```

- 放 `XgApp` 字段 (与 live_volumes/raw_vel_peaks 等播放状态并列), 不塞进 `PartState`
  (那是"音色+混音参数"的硬件语义, mute/solo 是**编辑会话级播放开关**, 与会话绑, 不写硬件)。
- 默认全 `false` (无静音、无独奏)。`Default` via `[false; 16]`。

### 2.2 派生查询 (纯函数, 可单测)

```rust
/// 该通道在播放输出层是否应静音 (Mute 优先于 Solo, DAW 惯例)
pub fn channel_is_effectively_muted(&self, ch_idx: usize) -> bool {
    let any_solo = self.channel_solos.iter().any(|&s| s);
    // 任一 solo 激活 → 非 solo 通道必静音; 再叠加自身 mute
    if any_solo {
        !self.channel_solos[ch_idx] || self.channel_mutes[ch_idx]
    } else {
        self.channel_mutes[ch_idx]
    }
}
```

（此处 Mute 优先语义: solo 通道若自己也被 mute → 仍静音。）

## 3. 播放链集成 (核心: 在哪一层过滤)

需求只在"实际发声"生效, 不改音序器状态机 → **过滤点在 `dispatch_play_events`**
(playback.rs, 所有 NoteOn/NoteOff/CC/PC 事件发往 MIDI 设备的唯一出口)。

### 3.1 `dispatch_play_events` 增加过滤

在 `for e in &evs` 路由之前, 先按 channel 判静音, **被静音的通道直接跳过不发**:

```rust
for e in &evs {
    let ch = (e.channel as usize) % 16;      // 播放事件 channel 0-15
    if self.channel_is_effectively_muted(ch) {
        continue;                            // 静音通道: 不发
    }
    // ...原有按 part 路由/发送逻辑...
}
```

- `% 16`: 播放事件 channel 只有 0-15 (Part 1-16 ← PortA ch1-16, Part 17-32 ← PortB ch1-16
  由 `mirror_to_b`/拓扑在路由层做, 播放层始终 0-15 MIDI channel)。
- 过滤同时拦截 NoteOn+NoteOff: mute 通道不会有音符; unmute 恢复后正常。

### 3.2 ⚠️ 清音策略 — mute/solo 触发时的挂音

DAW 行为: **mute 一个正在响的通道, 必须立刻给该通道发 All Notes Off, 否则挂音持续**。
同理 **solo 激活 → 非 solo 通道立即清音**。

实现: mute/solo 状态变更时调用现有 `sound_off_for_channel(ch)`:

```rust
/// 对单个通道发 All Notes Off(CC123)+All Sound Off(CC120) (wasm 下走 midi_wasm)
pub fn sound_off_channel(&self, ch: u8) { ... } // 新函数, 复用 dispatch 的发送路径
```

- `dispatch_play_events` 里的静音过滤挡住**后续**音符; `sound_off_channel` 处理**已响**音符。
- 两者配合: 点 Mute → 立即清该 ch 挂音 + 后续事件不发 → 演奏正确。
- Point 切换时同理: 重新 solo/mute 的通道该清就清。

### 3.3 Level meter 归零 (可选优化)

- 静音通道的实时电平 `live_levels[ch]` 直接显示 0 (不再打点拉高), 视觉上"这条是死的"。
- 实现: `smooth_meter_target()` 里对 `channel_is_effectively_muted(ch)` 返回 0。

## 4. UI 设计 (Channel View 行头 gutter)

当前 gutter 宽 `158px`, 内容: 左 `ChNN`(12px 字) + 音色名(+30px) + 绿电平条(+98px, 28px 宽)。
**M/S 按钮放 ChNN(名) 和 电平表之间** (John 2026-08-13 定案), gutter 加宽到 `~188px`:

```
[Ch01  GrandPno..] [M][S] [|====|]      (notes 区)
```

- `ChNN` + 音色名 (左, 6..~92px)
- **M/S 按钮** (中, ~94..~128px): 两个 18px 见方按钮
- 绿电平条 (右, ~134..~162px)
- 分隔线 → `gutter_w ~188px`

### 4.1 按钮规格

- **每个 button** `~18px` 见方, 两按钮间 4px, 紧挨音色名之后。
- **Mute**: 常态灰 `(0x44,0x44,0x44)`, active 红 `(0xe0,0x35,0x35)`(DAW 惯例红 mute), 字 "M" 白 10px。
- **Solo**: 常态灰, active 黄/琥珀 `(0xff,0xb0,0x30)`(DAW 惯例 amber solo), 字 "S"。
- hover 提亮; 点击 toggle; 不显示文字标签 (空间有限, 用字母 M/S + 颜色语义)。
- 颜色/布局支柱: 程序化断言 + cargo test; 视觉最后由 John 实测验收。

### 4.2 交互

- 点击 M → `channel_mutes[ch] = !channel_mutes[ch]`; 若变为 true → `sound_off_channel(ch)`。
- 点击 S → `channel_solos[ch] = !channel_solos[ch]`; 若变为 true →
  对所有**非 solo** 通道 `sound_off_channel` (它们现在静音了)。
- 状态同时体现在 Channel View 行头 (红/amber 按钮) + **播放输出**。

### 4.3 行高/布局联动

- gutter 加宽到 `~188px`: `gutter_w` 常量改 + `notes_left = outer.left() + gutter_w` 自动跟随。
- M/S 按钮 y = 行中心, 在音色名与电平条之间 (John 定案)。
- 行高小(16px)时按钮 18px 略超, 接受(可调 `min(row_h, 18)` 视觉, 或按钮缩到 16px)。

## 5. 测试计划 (程序化断言, John 铁律)

1. `mute_blocks_dispatch` — mock fired events, 设 `channel_mutes[0]=true`, `dispatch_play_events`
   后该 ch 事件不进入发送 (用注入性 test hook / 检查发送路径计数)。
2. `solo_isolates_channels` — solo ch0, 只有 ch0 事件通过, 其余跳过。
3. `mute_priority_over_solo` — ch0 同时 solo+mute → 静音。
4. `solos_off_all_pass` — 全 solo=false → 所有事件按原样通过。
5. `meter_zero_when_muted` — `smooth_meter_target` 对 muted ch 返回 0。
6. `default_no_mute` — 新 app 全 false。

(dispatch 发送在 wasm cfg 下走真实 MIDI; native 是 stub。事件过滤逻辑抽成**非 wasm 可测**的
纯函数 `should_skip_channel(ch)`/`channel_is_effectively_muted(ch)`, 测试打在该纯函数 +
dispatch 顶层标注。)

## 6. 版本/发布

- 递增 v0.1.21 → **v0.1.22**, 同步 `www/index.html` APP_VERSION + `?v=` 两处。
- feature branch `feat/channel-mute-solo` → cargo test 全绿 → 用户实测验收 → 才 merge main + push
  (发布铁律: 未获用户点头不 merge/push)。

## 7. 边界 / 不做的事

- **不改 SMF/parts 音色、不改硬件 PartState** — mute/solo 是播放输出层的编辑会话状态。
- **不持久化** (会话级, 每次刷新重置; 与 persisting 的 UI 布局状态分离)。若 John 要持久化后续再加。
- **不做 PlayView 的 mute/solo 化** — 本次只 Channel View; PlayView 未来可复用同一状态。
- ch10 鼓也可 mute/solo (它只是普通 MIDI 通道, 无特殊豁免)。
- 顶部信息栏/其他面板不动。

## 8. 决策记录 (John 2026-08-13 已拍板)

1. 按钮位置: **ChNN(名) 与 电平表之间** (gutter 中部)。
2. 交互: **点击立即生效** (标准 DAW)。
3. 配色: **Mute 红 / Solo 琥珀**。
4. 持久化: **不持久化** (会话级, 刷新重置)。
5. 电平表: **mute 后归零**。
