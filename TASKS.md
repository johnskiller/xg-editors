# TASKS.md — 任务跟踪 / 当前进度 / 待办

> **易变文档**:这里记录正在做什么、下一步做什么、坑和进度。
> 每次 session 开工/收尾都更新这里。**不要**把这类内容写进 AGENTS.md。
> 完成任务后把条目移到「已完成」区或删除。

---

## 当前进行中

(空 — 小问题清单第一轮已完成并发布,等下一批)

## 待办池 (Backlog)

- [ ] 「小问题」清单第二轮(用户/自己再收集)
- [ ] `docs/cloudflare-pages-deploy.md` —— 历史调研文档,确认要不要留(可删)
- [ ] `STATUS_CHECK.md` —— 一次性状态核对文档,已过时,可删

## 已完成 (历史)

- [x] **第一轮小问题修复 (2026-08-12, feat/lcd-live-sync)**:
  - ① LCD bank/pgm 与 part 联动: `update_lcd_params` SMF 加载后从 live_bank/live_program 取当前 part 通道真实值(音色/bank/pgm 三者同步),不再用滑块编辑值
  - ② Part10 鼓 icon: `resolve_icon` 增加鼓组短名(StandKit/Room/Jazz/Brush 等)→ Standard 鼓位图 fallback
  - ③ 默认音色名: 空通道音色 fallback 从 "ChNN" → "GrandPno" (XG 初始化默认 bank000/pgm001); 涉及 playback.rs(live_voice_names 预填 2 处) + lib.rs(PlayView 矩阵 voice_name_for_channel 兜底)
  - 测试: +1 (drum_voice_gets_icon) + 扩展 LCD 联动断言 → 81/81 绿
  - ✅ 用户确认: 无 SMF ChannelView = Ch01|GrandPno, 加载 MIDI 后空通道 = GrandPno, 均正确
- [x] v0.1.0 基线整理:orphan 干净历史、扁平结构、80/80 测试绿、GitHub Actions 自动部署 Pages
- [x] PlayView 星云美化版并入 v0.1.0 基线(源自原 v92~v95 迭代)
- [x] SMF 解析器 running-status bug 修复(Aftertouch/Pitch Bend 字节数,debug/file11-track3-parse 已合并)

## 开发流程(约定)

```
feature branch → 本地 cargo test 全绿 → merge 到 main → push main
→ GitHub Actions 自动构建 wasm + 部署 GitHub Pages
```

## 备注 / 有用的引用

- 完整旧历史/设计定稿在 `~/xg-editor-archive/reference/`(本仓库无 reference/ 目录)
- GitHub: https://github.com/johnskiller/xg-editors · Pages: https://johnskiller.github.io/xg-editors/
- 本机部署: `./build_wasm.sh 8090` → http://127.0.0.1:8090/
