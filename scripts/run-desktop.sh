#!/bin/sh
# 构建并启动桌面版。带 --demo 时用演示资料库；默认沿用上次打开的库。
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

cargo build --release --manifest-path desktop/src-tauri/Cargo.toml

BIN="$ROOT/desktop/src-tauri/target/release/recallcard-desktop"

# 远程桌面、容器或没有可用的 GPU 时，WebKitGTK 会因拿不到渲染设备而白屏。
# 这些变量让它退回软件渲染；正常桌面会话不需要。
: "${DISPLAY:=:0}"
export DISPLAY
export GDK_BACKEND="${GDK_BACKEND:-x11}"
export WEBKIT_DISABLE_COMPOSITING_MODE=1
export WEBKIT_DISABLE_DMABUF_RENDERER=1
export LIBGL_ALWAYS_SOFTWARE=1

if [ "${1:-}" = "--demo" ]; then
  VAULT="$ROOT/../演示资料库"
  [ -d "$VAULT" ] || { echo "找不到演示资料库：$VAULT" >&2; exit 1; }
  echo "演示资料库：$VAULT"
fi

exec "$BIN"
