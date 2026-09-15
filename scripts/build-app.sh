#!/usr/bin/env bash
# 一键构建并打包 CloudStorage.app（macOS），产出 Homebrew cask 可安装的 zip。
#
# 用法：
#   ./scripts/build-app.sh           # release 构建 + 打包
#   ./scripts/build-app.sh --rebuild # 强制重新构建
#   ./scripts/build-app.sh --no-build  # 仅用现有 target/release 二进制打包
#
# 产物：
#   dist/CloudStorage-{version}-macos-{arch}.zip
# 脚本会打印该 zip 的 sha256，供 Homebrew cask 使用。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# 非交互 shell 通常未加载 rustup 环境，确保能找到 cargo/rustc
if [[ -x "$HOME/.cargo/bin/cargo" ]]; then
 export PATH="$HOME/.cargo/bin:$PATH"
fi

BIN_NAME="CloudStorage"
CRATE="object-storage-desktop"
ARCH="$(uname -m)" # arm64 / x86_64
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"

if [[ -z "$VERSION" ]]; then
  echo "错误：无法从 Cargo.toml 解析版本号" >&2
  exit 1
fi

BUILD=1
case "${1:-}" in
  "" | "--rebuild") BUILD=1 ;;
  "--no-build") BUILD=0 ;;
  *) echo "未知参数：${1:-}（支持 --rebuild / --no-build）" >&2; exit 1 ;;
esac

DIST_DIR="dist"
APP_BUNDLE="$DIST_DIR/CloudStorage.app"
ZIP_FILE="$DIST_DIR/CloudStorage-${VERSION}-macos-${ARCH}.zip"

echo "==> 版本 : $VERSION"
echo "==> 架构 : $ARCH"
echo "==> 目标 : $ZIP_FILE"

# 1. 构建 release 二进制
if [[ "$BUILD" == "1" ]]; then
  echo "==> cargo build --release -p $CRATE"
  cargo build --release -p "$CRATE"
else
  echo "==> 跳过构建，使用现有 target/release/$BIN_NAME"
fi

BIN="target/release/$BIN_NAME"
if [[ ! -x "$BIN" ]]; then
  echo "错误：找不到 $BIN，请先构建" >&2
  exit 1
fi

# 2. 组装 .app 目录结构
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"

echo "==> 拷贝二进制"
cp "$BIN" "$APP_BUNDLE/Contents/MacOS/$BIN_NAME"
strip -x "$APP_BUNDLE/Contents/MacOS/$BIN_NAME"

# 3. 生成 AppIcon.icns（源图 crates/ui/assets/app-icon.png，512x512）
ICON_SRC="crates/ui/assets/app-icon.png"
ICONSET_DIR="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET_DIR"
echo "==> 生成 AppIcon.icns"
for spec in \
  "16 icon_16x16.png" \
  "32 icon_16x16@2x.png" \
  "32 icon_32x32.png" \
  "64 icon_32x32@2x.png" \
  "128 icon_128x128.png" \
  "256 icon_128x128@2x.png" \
  "256 icon_256x256.png" \
  "512 icon_256x256@2x.png" \
  "512 icon_512x512.png" \
  "1024 icon_512x512@2x.png"; do
  size="${spec%% *}"
  name="${spec##* }"
  sips -z "$size" "$size" "$ICON_SRC" --out "$ICONSET_DIR/$name" >/dev/null
done
iconutil -c icns "$ICONSET_DIR" -o "$APP_BUNDLE/Contents/Resources/AppIcon.icns"

# 4. Info.plist 与 PkgInfo（模板中 __VERSION__ 替换为实际版本）
echo "==> 写入 Info.plist / PkgInfo"
sed "s/__VERSION__/$VERSION/g" scripts/Info.plist.in > "$APP_BUNDLE/Contents/Info.plist"
printf 'APPL????' > "$APP_BUNDLE/Contents/PkgInfo"

# 5. ad-hoc 代码签名（本地可启动；正式分发需 Developer ID + 公证）
echo "==> 代码签名（ad-hoc）"
if ! codesign --force --deep --sign - "$APP_BUNDLE"; then
  echo "警告：ad-hoc 签名失败，跳过签名（本地产物仍可运行）" >&2
fi

# 6. 打包 zip（ditto 保留权限与符号链接）
echo "==> 打包 zip"
rm -f "$ZIP_FILE"
mkdir -p "$DIST_DIR"
ditto -c -k --sequesterRsrc --keepParent "$APP_BUNDLE" "$ZIP_FILE"

# 7. 校验
echo "==> 校验签名与目录结构"
if codesign --verify --deep --strict "$APP_BUNDLE"; then
  echo "  签名校验 OK"
else
  echo "  警告：签名校验未通过" >&2
fi
plutil -lint "$APP_BUNDLE/Contents/Info.plist" >/dev/null && echo "  Info.plist 校验 OK"
find "$APP_BUNDLE" -type f | sed 's/^/    /'

echo
echo "==> 产物: $ZIP_FILE"
echo "==> 大小: $(du -h "$ZIP_FILE" | cut -f1)"
echo "==> sha256: $(shasum -a 256 "$ZIP_FILE" | awk '{print $1}')"