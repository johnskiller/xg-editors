# TopBar 美化设计 (2026-08-13)

**状态**: 设计已定稿 (John 2026-08-13 拍板 6 项)
**基线**: main @ `87e75ed` (v0.1.22, Pages 在线), 97/97 测试绿

## 1. 背景 / 目标

当前 TopBar 是**一个长 horizontal 堆叠 400+ 行**（panels.rs `top_bar`，1..405 行），
把标题、Tempo、传输控制、Open MIDI、MIDI 设备选择、拓扑、SysEx 捕获、PortB、
读 part、双向通信调试等**全塞进一行** —— 既乱又宽，窄窗口直接溢出。

John 诉求（2026-08-13）：
1. **尺寸略微加高**
2. **开头一个 menu icon**，把 Open MIDI / MIDI Setup 等收进菜单
3. **剩下的全面组件化**
4. **Transport control 补全** —— 现有只有 play/stop，需补全；**录音按钮先空着不实现功能**

## 2. 目标布局

```
┌──────────────────────────────────────────────────────────────────────────┐
│ [☰] XG Editor v0.1.23  │ Tempo [120] bpm 4/4 │  [⏵|⏸][⏹][⏺]  1:01.02 │ ...
└──────────────────────────────────────────────────────────────────────────┘
   ^ menu icon            ^ title+tempo        ^ transport (完整)   ^ 其他
```

TopBar 高度从默认 ~28px **略微加高到 ~40px**（留出 transport 按钮舒适高度）。

### 组件分区（从左到右）
1. **Menu icon**（`☰`，手绘三条横线 or 自定义控件）：下拉菜单
   - **文件**：Open MIDI...
   - **MIDI Setup**：设备选择（选中输出 A/B + 拓扑 + 连接状态）→ 展开
   - （占位，未来可加）保存 / 导出 / 设置
2. **标题 + 版本**：`XG Editor v0.1.23`（缩小字号/紧凑，避免喧宾夺主）
3. **Tempo / 拍号**：`Tempo [120] bpm 4/4`（DragValue + label 保持）
4. **Transport control（组件化）**：
   - **Play/Pause** 切换按钮（三角形 ▶ / 暂停 ⏸）
   - **Stop**（方块 ⏹）
   - **Record**（圆点 ⏺，**只画不响**；点击可切换视觉 armed 态但无功能）
   - 位置显示 `1:01.02`（bar:beat:tick，保留）
5. **右侧**：连接状态点 (`[OK] Connected` / `[--] Not connected`) —— 简化成小色点 + 文本
   （其余调试控件该收的收起）

## 3. 组件化方案（egui custom widget / 独立函数，匹配 MSButton 模式）

按 John 2026-08-13 的偏好（M/S 按钮要求 custom 控件），TopBar 组件做成**独立结构体+函数**，
放 `src/topbar.rs`（或就地拆小函数，看规模）：

### 3.1 `TransportButton` custom widget（`src/transport.rs`）
```rust
pub enum Transport {
    Play,    // ▶  播放
    Pause,   // ⏸  暂停 (播放中显示)
    Stop,    // ⏹  停止
    Record,  // ⏺  录音 (功能预留, 视觉 armed)
}
pub struct TransportButton { kind: Transport, active: bool, size: f32 }
impl egui::Widget for TransportButton { ... }
```
- **图标：字体字形优先**（egui 0.29 内置 emoji-icon-font 已实测覆盖 ▶⏸⏹⏺☰ —— 2026-08-13
  `check_ttf_cmap.py` 验证, 不再担心 tofu）。用字符渲染:
  - Play `▶`(U+25B6), Pause `⏸`(U+23F8), Stop `⏹`(U+23F9), Record `⏺`(U+23FA)
  - 若个别环境过滤掉 emoji-icon-font 再 fallback 到手绘矢量（painter 画三角/方块/圆）。
- **配色**：与主题一致（灰色常态，hover 提亮；Record 常态圆点灰，armed 时红）。
- 复用 `MSButton` 的 `ui.put(rect, widget)` 模式或 `ui.add_sized`。

### 3.2 `MenuIconButton` / 手绘 ☰（`src/topbar.rs`）
- 三横线手绘（`painter.line_segment` 三条）vs `egui::menu`。
- egui 0.29 有 `ui.menu_button("...", ...)`（已用于 voice picker），直接 `menu_button` 配自绘菜单项。
- 菜单项：Open MIDI / MIDI Setup / (未来) Settings。
  - **MIDI Setup 展开子菜单**：设备下拉（输出 A/B + mirror→B + 连接状态 + 拓扑）——从顶栏挪进来。

## 4. Transport 状态机（补全逻辑）

现有逻辑（panels.rs:22-53）：
- `playing` 时显示 `[Pause]`，点击 → Pause（清挂音，不清 playhead）
- 非 playing 显示 `[Play]`，点击 → `play_resume()`（从当前位置续播）
- `[Stop]` → playhead 归 0 + 清事件表 + 电平归零 + 清挂音

**补全后的 transport 语义（保持现有行为，不破坏）**：
- **Play/Pause 二合一**：`playing ? Pause : Play`
- **Stop**：独立（归 0 + 清音）
- **Record**：**新按钮，视觉 armed 切换**（`rec_armed: bool` 字段），点击切换亮/灭；
  无实际录音逻辑（John 明确"先空着功能不实现"）。
  - 考虑：Record 是否影响 playhead？暂不（纯占位）。

## 5. 收进 Menu 的控件（减少顶栏拥挤）

| 控件 | 现在位置 | 目标 |
|---|---|---|
| Open MIDI 按钮 | 顶栏 | → Menu ▸ File ▸ Open MIDI |
| MIDI 设备下拉 (A) | 顶栏 | → Menu ▸ MIDI Setup（设备选择 + mirror→B + 拓扑 + 连接状态） |
| PortB 下拉 | 顶栏 | → Menu ▸ MIDI Setup |
| mirror→B checkbox | 顶栏 | → Menu ▸ MIDI Setup |
| 连接状态 `[OK]/[--]` | 顶栏尾部 | 保留在顶栏（精简成小色点 + 文本，transport 相关，常用） |
| SysEx Capture / Analyze / Read Part / Read All / Bulk | 顶栏 | → Menu ▸ Tools（或 Debug 折叠）—— 这些是**调试/开发**控件，非常用 |
| Web MIDI 探测 / 拓扑文本 / Send Test / Bind Input | 顶栏 | → Menu ▸ Tools（或隐藏） |

**关键判断**：把这些都收进菜单后，顶栏只保留
`☰ | 标题+版本 | Tempo 拍号 | Transport | 位置显示 | 连接点` —— **清爽、组件化**。

⚠️ 调试控件（SysEx/Read/拓扑）**不能删**，只是挪进 Menu ▸ Tools 子菜单，功能保留。
开发期可能需要快速访问 —— 是否在顶栏保留一个可选的 "Debug" 展开？**待 John 拍板**。

## 6. 数据源 / 状态（不改逻辑，只重组）

- `self.playing` / `self.tempo_bpm` / `self.selected_midi(_b)` / `self.mirror_to_b` /
  `self.midi_connected` / `self.midi_devices` / `self.midi_topology` —— 全部已有，只挪 UI 位置。
- 新增：`rec_armed: bool`（Record 视觉状态，默认 false，不持久化）—— 纯 UI，无逻辑。
- 布局/状态：不新增持久化字段（除非 John 要）。

## 7. 测试 / 验证（程序化断言，John 铁律）

1. **编译 + 测试 97/97 绿**（重构不引入新逻辑，现有测试应全过）。
2. **Transport 状态机单测**（如果抽成纯函数）：`play_pause_toggle` / `stop_resets` /
   `record_is_noop`（点击 record 不改 playing/playhead——确认"空着不实现功能"）。
3. **浏览器像素验证**（playwright + PIL，复用既有脚本模式）：
   - TopBar 高度变高（截顶栏带高度范围）
   - Menu icon 点击 → 菜单弹出（出现菜单项文本像素）
   - Transport 按钮图标可见（Play 三角 / Record 红点）
   - Play 点击 → 图标变 Pause（状态切换可检测）
4. 需真实 egui 上下文；用 `?view=` 钩子固定视图。

## 8. 版本 / 发布

- 递增 v0.1.22 → **v0.1.23**，同步 `www/index.html` 两处。
- feature branch `feat/topbar-beautify` → cargo test 全绿 → 用户实测验收 → 才 merge main + push。

## 9. 边界 / 不做的事

- **不改 transport 语义**（Pause 不清 playhead / Stop 归零 / 清挂音 等现状保留）。
- **不做录音逻辑**（Record 纯 UI armed 显示）。
- **不删任何功能**（调试控件挪 Menu ▸ Tools，非删除）。
- 不改左侧栏/中央/底栏布局（纯 TopBar）。
- MIDI Setup 的**完整设置页**（改名/重扫/权限）暂不做，只拆 UI 位置。

## 10. 决策记录 (John 2026-08-13 已拍板)

1. **TopBar 高度**: **44px**。
2. **Record 按钮**: 只做 armed 视觉切换（点击红点亮灭），不接任何录音功能。
3. **调试控件**: 全收进 **Menu ▸ Tools** 子菜单（SysEx/Read/Bulk/Send Test/Bind/拓扑文本）。
4. **菜单结构**: **分层**（File / MIDI Setup / Tools）。
5. **连接状态**: 顶栏**保留**精简版（小色点 绿/灰 + "Connected"/"Not connected"）。
6. **Tempo/拍号/播放 count**: 全部**保留在顶栏 + 组件化 + 字体放大**；count 颜色由开发者选择。

