# TASKS.md — 任务跟踪 / 当前进度 / 待办

> **易变文档**:这里记录正在做什么、下一步做什么、坑和进度。
> 每次 session 开工/收尾都更新这里。**不要**把这类内容写进 AGENTS.md。
> 完成任务后把条目移到「已完成」区或删除。

---

## 当前进行中

(空 — 等「小问题」清单整理出来后填入)

## 待办池 (Backlog)

- [ ] **「小问题」清单** —— 用户(jasking)正在整理,整理好之后逐条讨论、排实施顺序,再进 feature branch
- [ ] `docs/cloudflare-pages-deploy.md` —— 历史调研文档,确认要不要留(可删)
- [ ] `STATUS_CHECK.md` —— 一次性状态核对文档,已过时,可删

## 已完成 (历史)

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
