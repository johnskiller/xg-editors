# TASKS.md — 任务跟踪 / 当前进度 / 待办

> **易变文档**:这里记录正在做什么、下一步做什么、坑和进度。
> 每次 session 开工/收尾都更新这里。**不要**把这类内容写进 AGENTS.md。
> 完成任务后把条目移到「已完成」区或删除。

---

## 当前进行中

**main 已本地 merge (afcae82, 2026-08-15, 11 commits 未 push, 等 SC-55 VST 实测后走发版):**
- `feat/sysex-passthrough` (9 commits): SMF SysEx 透传(按 tick 全接口广播, channel=0xFF 哨兵) + Roland GS 识别 + SYSEX 区滚动/表头对齐/hex 详情/scrollbar
- `fix/program-change-playback` (2 commits): 曲中 Program Change 按 tick 发送(SC-55/LCD 跟随换音色, 根因之前只 tick0 注入) + LCD 刷新改浮窗可见性驱动(不再绑 Params 右栏)
- 测试 **111/111 绿**; 待 SC-55 VST 实测(sysEx 透传 + GS reset → Piano1 + 换音色链条已程序验证)
- 发版流程: bump APP_VERSION + sync ?v= → push main → CI 自动 build wasm + Pages

**上一波已 merged (v0.1.25):** Playable Piano Roll + Event List 已发布 Pages, 见「已完成」

## 待办池 (Backlog)

- [ ] **PolySyPd (bank000/pgm091) 缺 icon** —— 未定事项,等 John 回家真机 MU90 确认回显再定:
      `src/xg_icons.rs` ICONS 自动生成表 59 个不含 PolySyPd; resolve_icon 精确/前缀/fallback 三路都不中 → LCD 空白。
      方案待定: (a) FALLBACK 归到合成 pad 图标, (b) 专门画 16x16 图标。2026-08-12 John 拍板先记录、暂不动。
- [ ] 右栏 params「Rev: Hall | Cho: Chorus1 | Var: off」效果器行 —— 系统级效果(Rev/Cho 全局, Var 偏系统), 待并入 SystemFx 数据源(单源化后续)
- [ ] Cutoff/Reso(音色编辑参数)—— 已决定不进 part(留全局), 未来可考虑独立「音色编辑器」分区
- [ ] PlayView 复用 mute/solo 状态(当前只在 Channel View 加按钮; 播放/PlayView 输出已受 dispatch 过滤影响)
- [ ] (暂缓, John 2026-08-15 拍板) **SysEx 列表点击/按钮发送** —— 不加: 目前仅显示 RAW hex, 用户无法判断内容;
      等做了 SysEx 编辑 或 把 raw hex 解码成 event list 式可读信息 再做。播放时已全量透传, 手动发送需求弱。
- [ ] Event List 增强 (后续): PG/CC 行点击也可选听; playhead 播放行高亮; 行内容编辑(为编辑功能铺路)
- [ ] 「小问题」清单第二轮(用户/自己再收集)
- [ ] `docs/cloudflare-pages-deploy.md` —— 历史调研文档,确认要不要留(可删)
- [ ] `STATUS_CHECK.md` —— 一次性状态核对文档,已过时,可删
- [ ] (未定) 用户反馈窗口式 params 的参数还需再理清 —— 见设计 reference/part-single-source-design-2026-08-12.md

## 已完成 (历史)

- [x] **TopBar 美化 + 渲染修复 (2026-08-13, feat/topbar-beautify, merged main f4b129b → Pages 0.1.24)**: John 全部确认无误
  - TopBar: 深色 #1f2f45、fake-bold 亮白标题、Tempo/4-4 可读、亮金 count 固定宽、手绘 transport 24px
  - 全局深色主题 (menu/params/piano topbar); 底部状态栏保持浅色
  - 渲染修复: bar ruler 铺到 params 边缘、channel view 无蓝灰 padding、bar/beat 1-based 一位数、statusbar 文件名可读、ghost-note 清除
  - 测试 100/100
- [x] **Channel View per-channel Mute/Solo (2026-08-13, feat/channel-mute-solo, merged main d863645 → Pages v0.1.22-d863645)**:
  - 每行 gutter 加 M/S 按钮 (ChNN 名 与 电平表 之间, John 定案; M 红 / S 琥珀, custom widget `src/ms_button.rs`)
  - 播放输出层过滤 `dispatch_play_events`: 静音通道事件不发 MIDI; mute/solo 触发明细清音 (All Sound/Notes Off)
  - 语义: Mute 优先 Solo; 任一 solo → 非 solo 通道全静音; 不持久化; mute 后电平表归零 (demo+SMF 双路径)
  - 测试 97/97 绿 (+6); 浏览器像素验证 (M 红/电平归零/S 琥珀/多 solo) via scripts/verify_channel_mute_solo.py + verify_solo_buttons.py
  - 踩坑: egui `ui.put(custom widget)` 与长生命周期 painter 冲突 → painter 分段作用域获取
  - ✅ John 实测通过 → merge + push → Pages 部署验证 (APP_VERSION hash 注入成功, wasm 200)
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
