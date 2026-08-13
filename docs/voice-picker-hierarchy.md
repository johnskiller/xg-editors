# 音色选择器分级改造设计 (Voice Picker 3-level)

**日期**: 2026-08-13
**状态**: 设计定稿待实施
**作者**: John 需求 + XG Editor

## 背景

当前 params 面板的"pick"音色下拉框是**单一扁平列表**(lib.rs:2221-2247):
- 数据源 `voice_bank.voices_for_device(device)` → 606 个音色全列出来
- 显示 `{msb:04}/{lsb:04}/{prg+1:04}  {name}`,按 msb→prg→lsb 排序
- 缺点:几百个音色一次列出,选择困难

## 目标

改为**三级菜单**: 分组(类别) → 具体乐器 → variation

```
第一层: 类别分组
  Piano
  Chromatic Percussion
  Organ
  Guitar
  ...
  Drum Kits (msb=127)
  SFX (msb=64 区)

第二层: 具体乐器 (prg)
  Piano
    ├─ GrandPno (prg 0)
    ├─ GrndPnoK (prg 0 lsb1) ...
    ├─ MelloGrP
    ...
  Organ
    ├─ DrawOrgn (prg 16)
    ...

第三层: variation (lsb)
```

## 数据来源与分组逻辑

### 现有数据 (mu90_voices.json)
每条 voice: `{ msb, prg, lsb, name }`,共 606 个:
- **MSB=0** (537): 旋律音色,prg 0-127 = GM 128 音色标准分区
- **MSB=64** (49): GM2 SFX 音效区 (CuttngNz/Thunder/Wind/Dog...)
- **MSB=126** (2): MelloGrP? 低频扩展
- **MSB=127** (18): 鼓组 (Standard Kit/Room Kit...)

### 类别映射 (GM 标准, MSB=0 下的 prg 分区)
| 类别 | prg 范围 | variations |
|---|---|---|
| Piano | 0-7 | 39 |
| Chromatic Percussion | 8-15 | 21 |
| Organ | 16-23 | 44 |
| Guitar | 24-31 | 45 |
| Bass | 32-39 | 51 |
| Strings | 40-47 | 12 |
| Ensemble | 48-55 | 50 |
| Brass | 56-63 | 44 |
| Reed | 64-71 | 14 |
| Pipe | 72-79 | 10 |
| Synth Lead | 80-87 | 49 |
| Synth Pad | 88-95 | 33 |
| Synth Effects | 96-103 | 62 |
| Ethnic | 104-111 | 23 |
| Percussive | 112-119 | 32 |
| Sound Effects | 120-127 | 8 |

### 非 MSB=0 分类
- **MSB=64**: 归类为 "Sound Effects" (SFX 区)
- **MSB=127**: 归类为 "Drum Kits" (鼓组)
- **MSB=126**: 稀有, 归 "Sound Effects" 或单独 "Special"

## 实现方案

### 1. data.rs 加类别模型
```rust
/// 音色类别 (第一层)
pub enum VoiceCategory {
    Piano, ChromaticPercussion, Organ, Guitar, Bass,
    Strings, Ensemble, Brass, Reed, Pipe,
    SynthLead, SynthPad, SynthEffects, Ethnic, Percussive, SoundEffects,
    DrumKits,
}
impl VoiceCategory {
    fn label(&self) -> &'static str { ... }
    /// msb/prg → category (GM 分区)
    fn from_msb_prg(msb: u8, prg: u8) -> VoiceCategory { ... }
}
```

### 2. 三级导航状态 (XgApp 字段)
```rust
pub voice_pick_open: bool,        // 选择器展开?
pub voice_pick_cat: Option<VoiceCategory>,   // 第一层选择
pub voice_pick_prg: Option<u8>,              // 第二层选择 (乐器)
```
(第三层 variation 直接列出, 选中即应用)

### 3. UI 渲染 (lib.rs 替换现 ComboBox)
用 eframe 的 **egui 菜单/或嵌套 ComboBox**。egui 原生不支持多级菜单下拉, 方案:
- **方案 A (推荐)**: 模态窗口/自定义折叠面板 — 点 "pick" 打开一个面板, 三列: 类别/乐器/variation, 三级可折叠
- **方案 B**: 三个串行 ComboBox — 类别 → 乐器 → variation, 逐级筛选 (简单但多步)

John 倾向? (egui 0.29 无原生 nested menubar, 需自定义)

## 待确认
1. 三级交互: 折叠面板 (A) vs 三级串行下拉 (B)?
2. MSB=64/126 是否并入 Sound Effects 区, MSB=127 独立 Drum Kits?
3. 类别的 GM 英文名 (Piano/CP/Organ...) 是否用中文?
