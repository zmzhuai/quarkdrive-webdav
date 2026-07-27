#!/usr/bin/env bash
#
# 发版：改版本号 → 跑测试 → 提交 → 打 tag → 推送。
#
#   scripts/release.sh 1.4.0
#
# 推送 tag 会触发两个 workflow：
#   - docker.yml   构建 amd64/arm64 镜像，推到 ghcr.io/<owner>/quarkdrive-webdav
#                  打上 :<tag> 和 :latest
#   - release.yml  构建六个平台的二进制，发到 GitHub Releases
#
# 推送之前会停下来问一次 —— tag 一旦推上去，CI 就会往外发布，撤回很麻烦。
# 非交互场景（比如自己的脚本里调用）加 --yes 跳过确认。

set -euo pipefail

cd "$(dirname "$0")/.."

VERSION=""
ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
    --yes|-y) ASSUME_YES=1 ;;
    -*) echo "未知参数：$arg" >&2; exit 2 ;;
    *)
      if [ -n "$VERSION" ]; then echo "只能指定一个版本号" >&2; exit 2; fi
      VERSION="$arg"
      ;;
  esac
done

if [ -z "$VERSION" ]; then
  echo "用法：scripts/release.sh <版本号> [--yes]" >&2
  echo "例如：scripts/release.sh 1.4.0        # 会打出 v1.4.0" >&2
  echo "当前 Cargo.toml 版本：$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)" >&2
  exit 2
fi

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "版本号要形如 1.4.0（不带 v 前缀，脚本会自己加）" >&2
  exit 2
fi
TAG="v$VERSION"

# --- 前置检查 ------------------------------------------------------------

if [ -n "$(git status --porcelain)" ]; then
  echo "工作树不干净，先提交或 stash：" >&2
  git status --short >&2
  exit 1
fi

if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  echo "tag $TAG 已存在" >&2
  exit 1
fi

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [ "$BRANCH" != "main" ]; then
  echo "注意：当前在 $BRANCH，不是 main。tag 打在哪个提交上就发布哪个提交。"
fi

# --- 改版本号 ------------------------------------------------------------

# .bumpversion.cfg 已经和 Cargo.toml 对不上了，而且列的是本仓库没有的文件，
# 所以这里直接改 Cargo.toml，不走 bumpversion。
CURRENT="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
if [ "$CURRENT" = "$VERSION" ]; then
  echo "Cargo.toml 已经是 $VERSION，跳过改写"
else
  echo "版本号 $CURRENT -> $VERSION"
  # 只改 [package] 段里的第一个 version，别碰依赖的版本号
  awk -v v="$VERSION" '
    !done && /^version[[:space:]]*=/ { sub(/"[^"]*"/, "\"" v "\""); done=1 }
    { print }
  ' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
fi

# --- 验证 ---------------------------------------------------------------

echo "== cargo test（顺带把 Cargo.lock 里的版本号同步过来）"
cargo test --quiet

if [ -n "$(git status --porcelain)" ]; then
  echo
  git --no-pager diff --stat
  echo
  git add -A
  git commit -q -m "Release $TAG"
  echo "已提交：Release $TAG"
else
  echo "没有需要提交的改动，直接给当前提交打 tag"
fi

git tag -a "$TAG" -m "Release $TAG"
echo "已打 tag：$TAG"

# --- 推送 ---------------------------------------------------------------

REMOTE_URL="$(git remote get-url origin 2>/dev/null || echo '<没有 origin>')"
echo
echo "接下来会推送到 $REMOTE_URL："
echo "  - 分支 $BRANCH"
echo "  - tag  $TAG   （这一步会触发 CI 构建并对外发布镜像和二进制）"
echo

if [ "$ASSUME_YES" -ne 1 ]; then
  read -r -p "确认推送？[y/N] " reply
  case "$reply" in
    y|Y|yes|YES) ;;
    *)
      echo "已取消。本地的提交和 tag 还在，撤销："
      echo "  git tag -d $TAG && git reset --hard HEAD~1"
      exit 0
      ;;
  esac
fi

git push origin "$BRANCH"
git push origin "$TAG"

echo
echo "推送完成。构建进度："
echo "  https://github.com/${REMOTE_URL#*github.com[:/]}/actions" | sed 's/\.git\/actions$/\/actions/'
