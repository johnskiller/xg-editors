# Channel View 改进 — vertical scroll + 深色底色 + bar 线收敛 (2026-08-13)

## 背景 (用户反馈, #dev thread)

Channel View(每行 = 1 个 MIDI channel)存在三个问题:

1. **缺 ScrollBar** —— 底部 Piano Roll 拉高覆盖部分 channel 后,无法滚动看全部 16 行。
2. **16 channel 行下部的空白区是白色底** —— 难看,应改深色(与行背景一致)。
3. **bar/beat 竖线超出 16 channel 范围** —— 竖线画到面板最底 `rect.bottom()`,导致下方白色空白区有大量淡淡的竖线。

## 根因

`render_channel_notes`(`src/panels.rs:465-676`)完全不用 `ScrollArea`:
- 内容区 `rect = ui.available_rect_before_wrap()` + `ui.allocate_rect(rect, hover())`—— 固定占满面板,无滚动。
- 行背景只在 `ch_rows`(16)行内画,行以下 = 面板默认白底。
- bar/beat 竖线(658-673)与 playhead(650-657)画到 `rect.bottom()`(整个面板底),超出 16 行内容。

## 方案 (复用 piano_roll 的成熟 ScrollArea 模式)

`src/piano_roll.rs:139-147` 已验证的模式:

```rust
let mut scroll_area = egui::ScrollArea::vertical()
    .auto_shrink([false, false])
    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
    .id_salt("channel_view_scroll");
scroll_area.show(ui, |ui| {
    let c0 = ui.min_rect().top();
    ui.allocate_space(egui::vec2(outer.width(), total_h)); // 内容高 = ch_rows*CHANNEL_ROW_H
    // 内部绝对绘制: 顶 c0, 底 c0+total_h
});
```

### 具体改动 (`render_channel_notes`)

1. **内容区包 ScrollArea**:
   - `rect` → 改为 `outer = ui.available_rect_before_wrap()` + 深底铺满(`p.rect_filled(outer, ..., 深色)`)—— 覆盖所有 padding。
   - 标尺(顶栏时间轴)保持在 ScrollArea **外**、固定不滚动(内容滚动时标尺仍对齐视口顶)。
   - 内容区(行+gutter+音符)进 ScrollArea 内,`c0 = ui.min_rect().top()`,`total_h = ch_rows * CHANNEL_ROW_H`。

2. **深色底**:
   - 行背景保留现有交错色(0x12,1e,2e / 0x1f,2f,45)。
   - 内容区整块底(`[outer.left, outer.right] × [c0, c0+total_h]`)画深色(0x0c,14,1e,与 piano_roll 一致),这样即使滚动到 16 行末尾下面也是深色,不露白。
   - 视口 padding/多余区也铺深色(`outer` 整块)。

3. **bar/beat 竖线 & playhead 收敛到内容区**:
   - 所有 `rect.bottom()` → `content_bottom = c0 + total_h`。
   - 竖线高度 `[grid_top/c0, content_bottom]`,不再伸到面板底。

4. **行背景/分隔**:
   - 行循环改用 `y0 = c0 + i*CHANNEL_ROW_H`(不再从 panel rect 顶算)。

## 视觉基准

- 16 行 × 28px = 448px 内容高。
- 深色基色与 piano_roll 一致 `(0x0c, 0x14, 0x1e)`,保证三视图视觉统一。
- ScrollBar 始终可见(AlwaysVisible),沿用 piano_roll 风格。

## 验证

- 不加 SMF: 默认 tracks 行数 → ScrollArea 自然包住。
- 加 SMF: 16 行 → 内容高 448px;视口不足 → 出滚动条;拖滚动条能看全部 16 行。
- bar 竖线 & playhead 不超过 16 行底。
- 底部空白区深色,无竖线。
- 程序化验证(playwright 截图): 滚动条存在、白底消失、竖线止于内容底。
