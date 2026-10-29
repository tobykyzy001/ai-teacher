import { createApp } from "vue";
import { createPinia } from "pinia";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App.vue";
import { router } from "./router";
import { installHomeEndKeys } from "./utils";
import "./style.css";

installHomeEndKeys();

// 无标题栏窗口的拖拽：顶部 28px 内的非交互元素按下即拖动窗口；
// 另保留 data-tauri-drag-region 标记元素的旧约定（侧栏 logo 区）
const DRAG_BAND_PX = 28;
const DRAG_SAFE_SELECTOR =
  "button, a, input, textarea, select, [role=button], .n-button, .n-menu-item, .n-menu-item-content, .tab-item, .rail-item";

function onDragMouseDown(e: MouseEvent): void {
  if (e.button !== 0) return;
  const el = e.target as HTMLElement | null;
  if (!el) return;
  const marked = el.closest("[data-tauri-drag-region]");
  const inBand = e.clientY <= DRAG_BAND_PX;
  if (!marked && !inBand) return;
  if (el.closest(DRAG_SAFE_SELECTOR)) return;
  e.preventDefault(); // 避免拖动时误选文字
  void getCurrentWindow().startDragging();
}

document.addEventListener("mousedown", onDragMouseDown);

createApp(App).use(createPinia()).use(router).mount("#app");
