# TASKS.md — 任务跟踪 / 当前进度 / 待办

> **易变文档**:这里记录正在做什么、下一步做什么、坑和进度。
> 每次 session 开工/收尾都更新这里。**不要**把这类内容写进 AGENTS.md。
> 完成任务后把条目移到「已完成」区或删除。

---

## 当前进行中

(空 — 单源化重构已完成验收,当前在 feat/part-single-source 分支未合并/发布)

## 待办池 (Backlog)

- [ ] **PolySyPd (bank000/pgm091) 缺 icon** —— 未定事项,等 John 回家真机 MU90 确认回显再定:
      `src/xg_icons.rs` ICONS 自动生成表 59 个不含 PolySyPd; resolve_icon 精确/前缀/fallback 三路都不中 → LCD 空白。
      方案待定: (a) FALLBACK 归到合成 pad 图标, (b) 专门画 16x16 图标。2026-08-12 John 拍板先记录、暂不动。
- [ ] 右栏 params「Rev: Hall | Cho: Chorus1 | Var: off」效果器行 —— 系统级效果(Rev/Cho 全局, Var 偏系统), 待并入 SystemFx 数据源(单源化后续)
- [ ] Cutoff/Reso(音色编辑参数)—— 已决定不进 part(留全局), 未来可考虑独立「音色编辑器」分区
- [ ] 「小问题」清单第二轮(用户/自己再收集)
- [ ] `docs/cloudflare-pages-deploy.md` —— 历史调研文档,确认要不要留(可删)
- [ ] `STATUS_CHECK.md` —— 一次性状态核对文档,已过时,可删
- [ ] (未定) 用户反馈窗口式 params 的参数还需再理清 —— 见设计 reference/part-single-source-design-2026-08-12.md

## 已完成 (历史)

- [x] **Part 状态单源化重构 (2026-08-12, feat/part-single-source, 待验收合并)**:
  - 新增 `src/part.rs`: PartState(32 part × voice/bank/prog/8混音参数) + SystemFx(Rev/Cho/Var 类型)
  - XgApp 增 `parts[32]`; LCD/PlayView/ChannelView/params 面板统一从 parts 读
  - SMF 加载/CC0/CC32/PC 事件同步写入 parts[i]; params 前 8 条(VOL..KEY)per-part,Cutoff/Reso 留全局
  - 右栏 params 顶端新增「Part N · 音色 ▶bank▶pgm」显示行(John 要求)
  - 修复单源化引入的越界 panic(10 条 params vs 8 条 part params)
  - 测试 84/84 绿(+3 part.rs 单测,更新 4 个旧测试);本地 wasm 验证通过
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
