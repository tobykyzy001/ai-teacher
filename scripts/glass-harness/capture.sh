#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Glass harness 截图脚本（headless Microsoft Edge / Chromium）
#
# 用法：
#   ./capture.sh <out-dir> [base-url] [size] [slug-filter...]
#
#   ./capture.sh baseline                          # 全部默认（http://localhost:5310）
#   ./capture.sh baseline http://localhost:5310
#   ./capture.sh baseline http://localhost:5310 1280,860
#   ./capture.sh shots http://localhost:5310 1280,860 reading    # 只拍 slug 含 reading 的
#
# 产物命名：<out-dir>/<route-slug>-<theme>-<size>.png
#
# 关键坑（改动前务必读）：
#   1. --headless=old：本机 Edge 154 上 --headless=new 会卡死不退出。
#   2. 每个进程独立 --user-data-dir（不同主题要各自的 localStorage）。
#   3. Edge 截完图**不会自己退出**（实测常驻 60s+）。所以后台起进程、
#      轮询 PNG 出现 → 多等 0.5s 落盘 → kill，绝不能前台等它结束。
#   4. --virtual-time-budget 推进页面内定时器/动画，避免截到 loading 态。
#   6. **--force-prefers-reduced-motion 不能省**。应用有 `.page-fade` 路由过渡
#      （App.vue 的 transition name="page-fade"，out-in 约 90+120ms）。没有这个旗标时，
#      截图经常正好落在过渡中间，整页渲染成 ~25% 不透明度 —— 看起来像"材质全糊了"，
#      其实是动画没跑完。加上它之后 transition-duration 被压到 0.01ms，稳定可复现。
#      （这条是 Lead 踩出来的：一张发白的 reading 截图差点被当成材质 bug。）
#   5. **macOS 上布局视口有最小宽度 ~492px**：--window-size=390,844 出来的 PNG 是
#      390 宽，但 window.innerWidth=492，页面按 492 排版后被裁到 390 —— 看起来像
#      应用在手机上溢出（Tab 栏第 4 项、正文右侧被切），其实是截图假象。
#      手机尺寸因此走 mobile-frame.html 的 iframe：iframe 精确 390x844，
#      里面的 innerWidth 就是 390，截图刚好裁出那一块。桌面的 1280 宽不受影响。
# ---------------------------------------------------------------------------
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EDGE="/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"

OUT_DIR_ARG="${1:-$HERE/baseline}"
BASE_URL="${2:-http://localhost:5310}"
# 第三个参数：桌面基准尺寸（形如 1280,860）；手机尺寸由 SHOTS 表逐条指定
DESKTOP_SIZE="${3:-1280,860}"
shift 3 2>/dev/null || true
FILTERS=("$@")

mkdir -p "$OUT_DIR_ARG"
OUT_DIR="$(cd "$OUT_DIR_ARG" && pwd)"


if [ ! -x "$EDGE" ]; then
  echo "找不到 Microsoft Edge：$EDGE" >&2
  exit 1
fi

DESK_SLUG="${DESKTOP_SIZE/,/x}"

# 每行：route|theme|WxH|slug
#   route 是真实 hash 路由；theme 决定 localStorage['ai-teacher:theme']
#   宽度 >=900 走 harness.html 直出；<900 走 mobile-frame.html 的 iframe 套框
SHOTS=(
  "/today|light|${DESKTOP_SIZE}|today-light-${DESK_SLUG}"
  "/today|night|${DESKTOP_SIZE}|today-night-${DESK_SLUG}"
  "/library|light|${DESKTOP_SIZE}|library-light-${DESK_SLUG}"
  "/library|night|${DESKTOP_SIZE}|library-night-${DESK_SLUG}"
  "/settings|light|${DESKTOP_SIZE}|settings-light-${DESK_SLUG}"
  "/settings|night|${DESKTOP_SIZE}|settings-night-${DESK_SLUG}"
  "/settings|sepia|${DESKTOP_SIZE}|settings-sepia-${DESK_SLUG}"
  "/settings|slate|${DESKTOP_SIZE}|settings-slate-${DESK_SLUG}"
  "/reading/101|light|${DESKTOP_SIZE}|reading-light-${DESK_SLUG}"
  "/reading/101|night|${DESKTOP_SIZE}|reading-night-${DESK_SLUG}"
  "/reading/101|light|${DESKTOP_SIZE}|reading-light-${DESK_SLUG}-scrolled|scroll=1400"
  "/reading/101|night|${DESKTOP_SIZE}|reading-night-${DESK_SLUG}-scrolled|scroll=1400"
  "/wrong|light|${DESKTOP_SIZE}|wrong-light-${DESK_SLUG}"
  "/books/1|light|${DESKTOP_SIZE}|bookdetail-light-${DESK_SLUG}"
  "/today|light|390,844|today-light-390x844"
  "/today|night|390,844|today-night-390x844"
  "/library|light|390,844|library-light-390x844"
  "/reading/101|light|390,844|reading-light-390x844"
)

# 静态材质页（不走应用，不需要 base-url 的 5310/5320 差异，但仍从当前 server 取）
# 静态材质页（不走应用路由）：路径|slug|窗口尺寸|needs_mat
# 尺寸按各页实际内容高度给，避免截出一大片空白底（空白会让"脏边/奶白"无从判断）。
# needs_mat=1 表示这一页依赖 --mat-* 令牌：在 HEAD 那种没有材质系统的代码树上
# 它会渲染成一堆**看不见的空白格**（一点都不像"改造前"，反而像渲染失败），
# 所以下面会先探测服务端样式表，没有 --mat-* 就直接跳过并说明。
STATIC_SHOTS=(
  "swatches.html|swatches|1280,1080|0"
  "swatches.html?theme=night|swatches-night|1280,1080|0"
  "states.html|states|1280,900|1"
)

# 探测当前 base-url 的样式表里有没有材质系统 v5 的 --mat-* 令牌。
# 用 ?direct 拿未加工的 CSS 原文（普通的 /src/style.css 是 HMR 包装的 JS 模块）。
has_mat_tokens() {
  curl -sf "$BASE_URL/src/style.css?direct" 2>/dev/null | grep -q -- "--mat-control-fill"
}

passes_filter() {
  [ ${#FILTERS[@]} -eq 0 ] && return 0
  local slug="$1" f
  for f in "${FILTERS[@]}"; do
    case "$slug" in *"$f"*) return 0 ;; esac
  done
  return 1
}

# 真正跑一次 Edge 并等 PNG 落盘。所有截图都走这里，避免参数漂移。
capture_url() {
  local url="$1" size="$2" png="$3"
  local ud
  ud="$(mktemp -d "/tmp/glass-harness-ud.XXXXXX")"

  rm -f "$png"
  "$EDGE" \
    --headless=old \
    --no-sandbox \
    --disable-gpu \
    --hide-scrollbars \
    --force-device-scale-factor=1 \
    --force-prefers-reduced-motion \
    --virtual-time-budget=5000 \
    --force-prefers-reduced-motion \
    --user-data-dir="$ud" \
    --screenshot="$png" \
    --window-size="$size" \
    "$url" >/dev/null 2>&1 &
  local pid=$!

  local i
  for i in $(seq 1 160); do
    if [ -s "$png" ]; then sleep 0.5; break; fi
    if ! kill -0 "$pid" 2>/dev/null; then break; fi
    sleep 0.25
  done
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
  rm -rf "$ud"
}

report() {
  local slug="$1" png="$2" url="$3"
  if [ -s "$png" ]; then
    printf '  ✓ %-38s %s bytes\n' "$slug.png" "$(wc -c <"$png" | tr -d ' ')"
    return 0
  fi
  printf '  ✗ %-38s FAILED  %s\n' "$slug.png" "$url" >&2
  return 1
}

shot() {
  local route="$1" theme="$2" size="$3" slug="$4" extra="${5:-}"
  local png="$OUT_DIR/$slug.png"
  local width height url
  width="${size%%,*}"
  height="${size##*,}"

  if [ "$width" -lt 900 ]; then
    url="$BASE_URL/scripts/glass-harness/mobile-frame.html?w=${width}&h=${height}&theme=${theme}&route=${route}"
  else
    url="$BASE_URL/scripts/glass-harness/harness.html?theme=${theme}"
    [ -n "$extra" ] && url="$url&${extra}"
    url="$url#${route}"
  fi

  capture_url "$url" "$size" "$png"
  report "$slug" "$png" "$url"
}

# 静态页（swatches / states）：没有路由，尺寸由 STATIC_SHOTS 逐条给。
shot_static() {
  local path="$1" slug="$2" size="$3"
  local png="$OUT_DIR/$slug.png"
  local url="$BASE_URL/scripts/glass-harness/${path}"
  capture_url "$url" "$size" "$png"
  report "$slug" "$png" "$url"
}

echo "输出目录：$OUT_DIR"
echo "Base URL：$BASE_URL"
fails=0
for entry in "${SHOTS[@]}"; do
  IFS='|' read -r route theme size slug extra <<<"$entry"
  passes_filter "$slug" || continue
  shot "$route" "$theme" "$size" "$slug" "${extra:-}" || fails=$((fails + 1))
done

# 静态材质页：尺寸固定 1280 宽（内容三带两张表，够宽才排得下）
for entry in "${STATIC_SHOTS[@]}"; do
  IFS='|' read -r path slug ssize needs_mat <<<"$entry"
  passes_filter "$slug" || continue
  if [ "$needs_mat" = "1" ] && ! has_mat_tokens; then
    printf '  – %-38s SKIP（该服务端没有 --mat-* 令牌，此页只对"改造后"有意义）\n' "$slug.png"
    continue
  fi
  shot_static "$path" "$slug" "$ssize" || fails=$((fails + 1))
done

echo
if [ "$fails" -gt 0 ]; then
  echo "有 $fails 张截图失败。" >&2
  exit 1
fi
echo "完成。"
