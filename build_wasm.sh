#!/usr/bin/env bash
# XG Editor 冒烟验证构建脚本 — wasm 目标
# 用法: ./build_wasm.sh [serve_port]
set -euo pipefail
cd "$(dirname "$0")"

PORT="${1:-8080}"

echo "== 1. wasm 编译 =="
cargo build --release --target wasm32-unknown-unknown

echo "== 2. wasm-bindgen 生成 glue =="
wasm-bindgen --target web --out-dir www/pkg --no-typescript \
  target/wasm32-unknown-unknown/release/xg-editor.wasm

# 下划线别名兜底(旧 index 可能引下划线)
cp www/pkg/xg-editor.js www/pkg/xg_editor.js 2>/dev/null || true
cp www/pkg/xg-editor_bg.wasm www/pkg/xg_editor_bg.wasm 2>/dev/null || true

echo "== 3. 启动 http serve (Ctrl-C 停止) =="
echo "   http://127.0.0.1:${PORT}/"
python3 -m http.server "${PORT}" --directory www
