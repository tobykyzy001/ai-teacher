import { createApp } from "vue";
import { createPinia } from "pinia";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App.vue";
import { router } from "./router";
import { installHomeEndKeys } from "./utils";
import "./style.css";

installHomeEndKeys();

// 无标题栏窗口（macOS Overlay 标题栏）的窗口 chrome：把整个窗口的"背景区"都当作标题栏——
// 单键按下即拖动窗口，顶部标题栏带内双击 = 原生标题栏双击（macOS 缩放 zoom / Win+Linux 最大化）。
// 保护：按钮/菜单/输入等交互元素与正文文本区豁免（不影响点按与文字选中）。
const DRAG_SAFE_SELECTOR =
  "button, a, input, textarea, select, [role=button], .n-button, .n-menu-item, .n-menu-item-content, .n-input, .n-textarea, .n-select, .tab-item, .rail-item";
const TEXT_ZONE_SELECTOR =
  "p, pre, blockquote, td, th, h1, h2, h3, h4, h5, h6, .md, .md *, .row-title, .n-list-item";

// 顶部标题栏带（红绿灯与侧栏 logo 所在的那条空带，高度同侧栏收起态的顶部避让）：
// 只有落在这条带内的双击才缩放窗口——正文空白处双击不动作，避免误触。
const TITLEBAR_BAND_PX = 64;

// 双击判定：两次按下时间/位置都要接近。不能用 e.detail：首次按下就进入原生拖窗，
// WKWebView 的 click 计数会被原生拖窗打断，detail 常回不到 2，只能自己记时间戳。
const DOUBLE_CLICK_MS = 600;
const DOUBLE_CLICK_SLOP_PX = 8;

let lastDownAt = 0;
let lastDownX = 0;
let lastDownY = 0;

function isDragSurface(el: HTMLElement): boolean {
  if (el.closest(DRAG_SAFE_SELECTOR)) return false;
  if (el.closest(TEXT_ZONE_SELECTOR)) return false;
  return true;
}

// 双击标题栏 = 系统原生"缩放/最大化"切换（macOS zoom 只铺满可用屏幕，不是绿按钮的
// 独立 Space 全屏）；全屏 Space 里没有标题栏，双击不响应
async function toggleWindowZoom(): Promise<void> {
  try {
    const win = getCurrentWindow();
    if (await win.isFullscreen()) return;
    await win.toggleMaximize();
  } catch {
    /* 移动端等没有窗口标题栏缩放的平台：无视 */
  }
}

function onMouseDown(e: MouseEvent): void {
  if (e.button !== 0) return;
  const el = e.target as HTMLElement | null;
  if (!el || !isDragSurface(el)) {
    // 点到交互控件/正文：清掉双击计时，避免"点按钮 + 快速点空白"被误判成双击
    lastDownAt = 0;
    return;
  }

  const now = performance.now();
  const isDouble =
    now - lastDownAt <= DOUBLE_CLICK_MS &&
    Math.abs(e.clientX - lastDownX) <= DOUBLE_CLICK_SLOP_PX &&
    Math.abs(e.clientY - lastDownY) <= DOUBLE_CLICK_SLOP_PX;

  if (isDouble && e.clientY < TITLEBAR_BAND_PX) {
    // 原生标题栏双击 = 缩放/还原（macOS 走 NSWindow zoom，不进入独立 Space 的全屏）；
    // 已经在全屏 Space 时没有标题栏可双击，直接不响应（等价原生行为）
    lastDownAt = 0;
    e.preventDefault();
    void toggleWindowZoom();
    return;
  }

  lastDownAt = now;
  lastDownX = e.clientX;
  lastDownY = e.clientY;

  e.preventDefault(); // 拖动时勿选中文字
  void getCurrentWindow().startDragging();
}

document.addEventListener("mousedown", onMouseDown);

createApp(App).use(createPinia()).use(router).mount("#app");
