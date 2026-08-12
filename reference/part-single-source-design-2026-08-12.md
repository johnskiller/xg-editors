# Part 状态单源化重构设计 (2026-08-12)

> 目标: 消除 音色名/live_bank/live_program/cur_voice/cur_bank/cur_prog/params 多方分家的数据模型,
> 统一为「32 个 part 各自一份完整音色+混音参数」的唯一数据源。
> 触发: 加载 MIDI 后 LCD 音色变但 bank/pgm 恒 0 的根因(playback.rs live_bank/program 重置但 name 同步)。
> 用户(jasking)定调: (a) 音色+VOL/EXP/BRT/PAN/REV/CHO/VAR/KEY 每 part 独立;
> (b) Cutoff/Reso 是音色编辑参数,不随 part; (c) Rev/Cho/Var 类型是全局效果。

## 1. 真机模型 (MU90)

- **32 multi part** + 2 A/D part(A1/A2 音频输入,非 MIDI 音色 part)
- Part 1-16 ← Port A ch 1-16; Part 17-32 ← Port B ch 1-16
- SysEx 寻址: `F0 43 3N 4C 08 nn 00 [off] ... F7`, nn=0..31 → Part 1..32 (0-based), 与 port 无关
- 每个 part 的状态 = 音色(voice/msb/lsb/prog) + 混音参数(setup 参数)
- 音色参数(Voice Edit: Cutoff/Reso 等)是当前音色本身的属性,不是 part 的
- Rev/Cho/Var 效果器类型 = 系统级全局(Effect Bank), part 只有 send 量

## 2. 目标数据结构 (唯一数据源)

```rust
/// 每个 part 的完整状态 (1..=32)
pub struct PartState {
    pub voice: String,          // 音色名 (LCM/UI 显示, 如 "GrandPno")
    pub msb: u8,                // Bank Select MSB (CC0)
    pub lsb: u8,                // Bank Select LSB (CC32) — LCD 显示 bank 用 LSB
    pub prog: u8,               // Program 0-based (PC), LCD 显示 prog+1
    pub params: [f32; 8],       // VOL EXP BRT PAN REV CHO VAR KEY (% 或 0..1, 与 LCD 条对齐)
    // 注: 8 条 = 驱动 LCD 底部的那个顺序, 与现有 param_lcd_idx 映射一致
}

/// 全局系统效果 (每个 bank 一套, 当前先单套全局)
pub struct SystemFx {
    pub rev_type: String,       // "Hall" / "Plate" ...
    pub cho_type: String,       // "Chorus1" ...
    pub var_type: String,       // "off" ...
}

pub struct XgApp {
    // === 唯一数据源 ===
    pub parts: [PartState; 32], // 32-part 音色+混音参数
    pub cur_part: u32,          // 当前选中 part (1..=32), LCD/右栏显示它
    pub sys_fx: SystemFx,       // Rev: Hall | Cho: Chorus1 | Var: off

    // === 派生态 (非新建数据, 由 parts 实时派生/缓存) ===
    // live_levels / live_volumes / live_expressions / active_notes / cc_live
    //   仍是播放运行时状态 (电平/实时 CC 是"瞬时", 不是 part 状态)
    // PlayView 矩阵 / ChannelView 行头 / LCD 都读 parts, 不再读 live_voice_names 等

    // === 删除的旧字段 (迁移后) ===
    // live_voice_names: [String;16]   → parts[0..16].voice
    // live_bank: [(u8,u8);16]         → parts[0..16].(msb,lsb)
    // live_program: [u8;16]           → parts[0..16].prog
    // cur_voice / cur_bank / cur_prog → cur_part 那一个 PartState
    // params: Vec<(String,f32,f32,f32)> → parts[cur_part-1].params
}
```

## 3. 读取/写入路径 (所有视图从唯一源读)

| 视图 | 读取 | 写入 |
|---|---|---|
| **LCD 显示** | `parts[cur_part-1]` (voice/bank/prog/params) | — (只读, 由 editor 写前置) |
| **右栏 params 滑块** | `parts[cur_part-1].params[i]` | 改 `parts[cur_part-1].params[i]` → LCD 条同步 |
| **右栏 voice picker** | `parts[cur_part-1]` 当前音色 | 选新音色 → 写 msb/lsb/prog/voice |
| **PlayView 16ch 矩阵** | `parts[0..16]` | — |
| **ChannelView 行头** | `parts[0..16]` | — |
| **真机读回 part** (Read Part N) | — | 写 `parts[n]` (voice+msb+lsb+prog) |

### 加载 SMF
```
遍历 16 通道 (smf_views):
    parts[i].voice  = find(msb,prog,lsb) 音色名 (鼓通道 drum_display_name)
    parts[i].msb/lsb = v.bank (有则写, 无则 xg 默认 msb0/lsb0)
    parts[i].prog   = v.program (有则写, 无则 0)
    parts[i].params = 默认 (音量等后续按 CC7/CC11 播放实时更新)
其余 parts[16..32] 保持默认 (GrandPno msb0 lsb0 prog0)
```

### 播放中 CC/PC 事件 → 写 parts (不再写 live_* 数组)
```
CC0   → parts[ch].msb
CC32  → parts[ch].lsb
PC    → parts[ch].prog, parts[ch].voice = 查询音色名
CC7   → (瞬时音量 → live_volumes 播放状态, 但 LCD VOL 条读 parts[ch].params[0])
```

## 4. 与系统效果 / 音色编辑的关系 (用户定调)

- **per-part (8 条)**: VOL EXP BRT PAN REV CHO VAR KEY → `PartState.params[8]`
- **音色编辑 (不进 part)**: Cutoff / Reso → 属于当前音色上下文,单源重构保留为面板上的音色编辑区 (随 cur_part 切换但语义是"编辑当前音色的音色参数", 后续可进 VoiceEditState)
- **全局效果**: Rev: Hall | Cho: Chorus1 | Var: off → `SystemFx` (全局单套), LCD/面板显示从 sys_fx 读

## 5. 迁移步骤 (feature branch: feat/part-single-source)

1. 定义 `PartState` / `SystemFx` (新文件 src/part.rs 或并入 lib)
2. XgApp 增加 `parts: [PartState;32]` / `sys_fx`, 初始化默认 (全 GrandPno)
3. 迁移写入:
   - playback.rs 加载 SMF 预填 → 改写 parts[0..16]
   - 播放 CC/PC 事件 → 改写 parts
   - 真机 Read Part → 改写 parts[n]
   - 右栏 quickpick / 滑块 → 改写 parts[cur_part-1]
4. 迁移读取:
   - update_lcd_params → 读 parts[cur_part-1]
   - PlayView 矩阵 / ChannelView 行头 → 读 parts[0..16]
   - cur_voice/cur_bank/cur_prog 删除, 用 parts[cur_part-1] 取代
5. 删除旧字段 live_voice_names/live_bank/live_program/cur_*/params
6. 持久化 PersistedState: 从 parts[cur_part-1] 存取 (或改为存取整个 cur part)
7. 测试: 全部迁移;新增 "加载 SMF → 切 part → LCD/右栏/PlayView 全读同一 part 状态" 的断言

## 6. 风险与注意

- **参数语义**: params 值域 (0..127) vs LCD 条 (% 0..100) — 现有 8 条用 % 显示, 需统一 (设计: PartState.params 存 0..1 或原始 MIDI 值? 定: 存原始控制目标值 (VOL 0..127 等), 渲染/发送时才换算)
- **CC7/CC11 播放实时**: 播放时电平/表情是瞬时 (live_volumes), 但 LCD VOL 条显示的是 part 的静态 VOL 值 (CC7 也可能写 part). 需明确定义: 播放中 CC7 是否持久写 parts? (建议: 持久写, 真机也这样 — CC7 改变 part 音量设置)
- **32 part vs 16 通道**: SMF 只有 16 通道 → 只填 parts[0..16]; parts[16..31] 是 B 口 (当前微镜像模式不填)
- **cur_part 切换** = 只改 cur_part 字段, LCD/右栏自动跟随 (所有读取点都从 parts[cur_part-1] 取 = 天然联动, 这就是单源化的收益)
- **不要破坏**: 播放 (PlayEvent 表 / active_notes / cc_live) 是运行时瞬时状态, 与 part 静态状态分离, 迁移时不合并这两组

## 7. 验收清单 (John 实测)

- [ ] 加载 MIDI(多通道)后, 切 Part 下拉框:LCD 音色 + bank/pgm + 右栏 8 条参数**一起切换**
- [ ] 右栏改 VOL/EXP 等滑块 → LCD 底部对应条变化, 且**只影响当前 part**(切走再切回, 值保留)
- [ ] 右栏 voice picker 换音色 → LCD 音色/bank/pgm 换, 参数条保留
- [ ] PlayView / ChannelView 16 通道行头与 LCD 同源 (同一通道同一音色)
- [ ] Rev: Hall | Cho: Chorus1 | Var: off 显示从全局 sys_fx 读 (独立于 part)
- [ ] Cutoff/Reso 不随 part 切换 (音色编辑区)
- [ ] 真机 Read Part N → 该 part 的 LCD/右栏/矩阵全部同步
- [ ] 80+ 测试全绿; wasm 构建; 本地 :8090 实测; 再发布
