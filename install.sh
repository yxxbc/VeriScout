#!/bin/sh
# VeriScout (ves) installer — macOS / Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/yxxbc/VeriScout/main/install.sh | sh
#
# Flags:
#   -v, --version <tag>    install a specific tag (default: the latest release)
#   -d, --dir <path>       install into this directory (default: ~/.local/bin,
#                          or /usr/local/bin when it is writable)
#   -r, --repo <o/r>       install from another GitHub repository
#   -h, --help             show this help
#
# Environment overrides: VES_VERSION, VES_INSTALL_DIR, VES_REPO, VES_BASE_URL,
# VES_TARGET (force a target triple).
set -eu

REPO="${VES_REPO:-yxxbc/VeriScout}"
VERSION="${VES_VERSION:-latest}"
INSTALL_DIR="${VES_INSTALL_DIR:-}"
BASE_URL="${VES_BASE_URL:-}"
TARGET_OVERRIDE="${VES_TARGET:-}"
BIN_NAME="ves"

usage() {
    cat <<'EOF'
VeriScout (ves) installer — macOS / Linux

  curl -fsSL https://raw.githubusercontent.com/yxxbc/VeriScout/main/install.sh | sh
  sh install.sh -v v0.1.0 -d /usr/local/bin

  -v, --version <tag>   install a specific tag (default: latest release)
  -d, --dir <path>      install directory (default: ~/.local/bin or /usr/local/bin)
  -r, --repo <owner/repo>
  -h, --help            this text

Environment: VES_VERSION, VES_INSTALL_DIR, VES_REPO, VES_BASE_URL, VES_TARGET
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        -v|--version) [ $# -ge 2 ] || { echo "--version needs a tag" >&2; exit 2; }; VERSION="$2"; shift 2 ;;
        -d|--dir)     [ $# -ge 2 ] || { echo "--dir needs a path" >&2; exit 2; }; INSTALL_DIR="$2"; shift 2 ;;
        -r|--repo)    [ $# -ge 2 ] || { echo "--repo needs owner/repo" >&2; exit 2; }; REPO="$2"; shift 2 ;;
        -h|--help)    usage; exit 0 ;;
        *)            echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

have() { command -v "$1" >/dev/null 2>&1; }

fetch_file() {
    if have curl; then curl -fsSL --retry 2 --connect-timeout 20 -o "$2" "$1"
    elif have wget; then wget -qO "$2" "$1"
    else echo "this script needs curl or wget" >&2; exit 1
    fi
}

# The GitHub API answers 403 to anonymous callers without a User-Agent header (and
# shares one small quota per IP), so anything API-shaped goes through here.
fetch_stdout_api() {
    if have curl; then
        curl -fsSL --retry 2 --connect-timeout 20 \
            -H 'User-Agent: ves-installer' -H 'Accept: application/vnd.github+json' "$1"
    elif have wget; then
        wget -qO- --header='User-Agent: ves-installer' --header='Accept: application/vnd.github+json' "$1"
    else echo "this script needs curl or wget" >&2; exit 1
    fi
}

# Latest release tag. `https://github.com/<repo>/releases/latest` redirects to
# `/releases/tag/<tag>` for everyone, so read that first: no auth, no quota. The API
# stays as a fallback for setups where the redirect cannot be read.
latest_tag() {
    if have curl; then
        effective=$(curl -fsSL -o /dev/null -w '%{url_effective}' \
            "https://github.com/${REPO}/releases/latest" 2>/dev/null || true)
        case "$effective" in
            */releases/tag/*) printf '%s\n' "${effective##*/releases/tag/}"; return 0 ;;
        esac
    fi
    fetch_stdout_api "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
}

# ---- platform -------------------------------------------------------------
uname_s=$(uname -s)
uname_m=$(uname -m)
case "$uname_s" in
    Darwin) os_part="apple-darwin" ;;
    Linux)  os_part="unknown-linux-musl" ;;
    *)      echo "unsupported OS: $uname_s — use install.ps1 on Windows, or build from source" >&2; exit 1 ;;
esac
case "$uname_m" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *) echo "unsupported CPU: $uname_m" >&2; exit 1 ;;
esac
TARGET="${TARGET_OVERRIDE:-${arch}-${os_part}}"

# ---- resolve the release --------------------------------------------------
if [ "$VERSION" = "latest" ]; then
    echo "→ 查询最新版本（${REPO}）"
    VERSION=$(latest_tag)
    if [ -z "$VERSION" ]; then
        echo "could not find a release for $REPO" >&2
        echo "hint: install a specific tag with: sh install.sh -v v0.1.1" >&2
        exit 1
    fi
fi

if [ -z "$BASE_URL" ]; then
    BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
fi
ASSET="ves-${VERSION}-${TARGET}.tar.gz"

WORK=$(mktemp -d 2>/dev/null || mktemp -d -t ves-install)
trap 'rm -rf "$WORK"' EXIT INT TERM

echo "→ 下载 ${ASSET}"
if ! fetch_file "${BASE_URL}/${ASSET}" "${WORK}/${ASSET}"; then
    echo "download failed: ${BASE_URL}/${ASSET}" >&2
    echo "check the platform you asked for, or pick a tag: sh install.sh -v v0.1.1" >&2
    exit 1
fi

# ---- checksum -------------------------------------------------------------
if fetch_file "${BASE_URL}/SHA256SUMS.txt" "${WORK}/SHA256SUMS.txt" 2>/dev/null; then
    expected=$(tr -d '\r' < "${WORK}/SHA256SUMS.txt" | awk -v a="$ASSET" '$2 == a || $2 == "*"a {print $1; exit}')
    if [ -z "$expected" ]; then
        echo "! ${ASSET} 不在 SHA256SUMS.txt 里，跳过校验" >&2
    else
        if have sha256sum; then
            actual=$(sha256sum "${WORK}/${ASSET}" | awk '{print $1}')
        elif have shasum; then
            actual=$(shasum -a 256 "${WORK}/${ASSET}" | awk '{print $1}')
        elif have openssl; then
            actual=$(openssl dgst -sha256 "${WORK}/${ASSET}" | awk '{print $NF}')
        else
            actual=""
        fi
        if [ -z "$actual" ]; then
            echo "! 没找到 sha256 工具（sha256sum / shasum / openssl），跳过校验" >&2
        elif [ "$actual" != "$expected" ]; then
            echo "checksum mismatch for ${ASSET}" >&2
            echo "  expected $expected" >&2
            echo "  actual   $actual" >&2
            exit 1
        else
            echo "✓ 校验通过 sha256 $(printf '%.12s' "$actual")…"
        fi
    fi
else
    echo "! 没有 SHA256SUMS.txt，跳过校验" >&2
fi

# ---- unpack + install -----------------------------------------------------
tar -xzf "${WORK}/${ASSET}" -C "$WORK"
SRC="${WORK}/ves-${VERSION}-${TARGET}/ves"
if [ ! -f "$SRC" ]; then
    SRC=$(find "$WORK" -type f -name "$BIN_NAME" | head -n 1)
fi
if [ -z "$SRC" ] || [ ! -f "$SRC" ]; then
    echo "压缩包里没有找到 $BIN_NAME 可执行文件" >&2
    exit 1
fi

case "$INSTALL_DIR" in
    "~"*) INSTALL_DIR="${HOME}/${INSTALL_DIR#\~}" ;;
esac
if [ -z "$INSTALL_DIR" ]; then
    if [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
        INSTALL_DIR="/usr/local/bin"
    else
        INSTALL_DIR="${HOME}/.local/bin"
    fi
fi
mkdir -p "$INSTALL_DIR"

if [ -e "${INSTALL_DIR}/${BIN_NAME}" ]; then
    echo "→ 覆盖已有安装（${INSTALL_DIR}/${BIN_NAME}）"
fi
if have install; then
    install -m 0755 "$SRC" "${INSTALL_DIR}/${BIN_NAME}"
else
    cp "$SRC" "${INSTALL_DIR}/${BIN_NAME}"
    chmod 0755 "${INSTALL_DIR}/${BIN_NAME}"
fi

installed=$("${INSTALL_DIR}/${BIN_NAME}" --version 2>/dev/null || echo "$BIN_NAME")
echo
echo "✓ 已安装 $installed → ${INSTALL_DIR}/${BIN_NAME}"

case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        echo
        echo "! ${INSTALL_DIR} 不在 PATH 里，把这行加到 ~/.zshrc / ~/.bashrc："
        echo "    export PATH=\"${INSTALL_DIR}:\$PATH\""
        ;;
esac

cat <<'EOF'

下一步：

  ves config init          # 生成配置，填入 api_key（或设 DEEPSEEK_API_KEY）
  ves "一次性纸杯到底能不能回收"
  ves --html "一次性纸杯到底能不能回收"   # 想要一份离线 HTML 报告再加这个
  ves --help               # 全部命令

默认只在终端把结论说清楚，不写任何文件；加了 --html 才会生成报告到 visualizations/。
EOF
