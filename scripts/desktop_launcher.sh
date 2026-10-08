#!/bin/sh
# 随包入口：只检查运行条件，不安装软件、不修改系统配置。
set -eu
APP_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
CHECK_ONLY=false
if [ "${1:-}" = '--check-runtime' ]; then
    CHECK_ONLY=true
    shift
fi

fail() {
    message="RecallCard 暂时无法打开

$1

Debian / Ubuntu 请通过系统官方软件源安装完整运行库：
sudo apt-get install git libgtk-3-0 libwebkit2gtk-4.1-0

如果已安装但缺少 WebKit 辅助程序，请修复该系统包：
sudo apt-get install --reinstall libwebkit2gtk-4.1-0

只把 .so 文件解压到任意目录，不能替代完整的系统安装。无需关闭浏览器沙箱。
也可先使用同目录的 recallcard 命令行读取资料。详情见「开始使用-中文.md」。"
    printf '%s\n' "$message" >&2
    if [ "$CHECK_ONLY" = false ] && [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
        if command -v zenity >/dev/null 2>&1; then
            zenity --error --title='RecallCard 启动检查' --text="$message" --width=600 2>/dev/null || true
        elif command -v kdialog >/dev/null 2>&1; then
            kdialog --title 'RecallCard 启动检查' --error "$message" 2>/dev/null || true
        elif command -v xmessage >/dev/null 2>&1; then
            xmessage -center -title 'RecallCard 启动检查' "$message" 2>/dev/null || true
        fi
    fi
    exit 78
}

[ -x "$APP_DIR/recallcard-desktop" ] || fail '运行包不完整，或 recallcard-desktop 没有执行权限。请重新完整解压运行包。'
command -v ldd >/dev/null 2>&1 || fail '缺少系统运行库检查工具 ldd，无法确认此运行包可启动。'
check_libraries() {
    if output=$(LC_ALL=C ldd "$1" 2>&1); then :; else
        fail "系统无法载入程序 $1：
$output"
    fi
    case "$output" in
        *'not found'*) fail "缺少程序 $1 的动态运行库：
$output" ;;
    esac
}
check_libraries "$APP_DIR/recallcard-desktop"

# 已验证的 Debian / Ubuntu amd64 包使用这个固定目录。其他发行版不据此误判。
# 官方文件清单见 docs/desktop-quickstart-v0.7.md。
if [ -f /etc/debian_version ] && [ "$(uname -m)" = x86_64 ]; then
    for helper in WebKitNetworkProcess WebKitWebProcess; do
        helper_path="/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/$helper"
        [ -x "$helper_path" ] || fail "缺少 WebKitGTK 的系统辅助程序：$helper_path"
        check_libraries "$helper_path"
    done
fi
if [ "$CHECK_ONLY" = true ]; then
    printf '%s\n' 'RecallCard 运行库检查通过；此检查不代表桌面窗口、模型或外部 AI 已连接。'
    exit 0
fi
if [ -z "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
    fail '没有检测到桌面显示会话。请在已登录的图形桌面打开，或使用同目录的 recallcard 命令行。'
fi
exec "$APP_DIR/recallcard-desktop" "$@"
