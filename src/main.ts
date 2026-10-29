import { createApp } from "vue";
import { createPinia } from "pinia";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App.vue";
import { router } from "./router";
import { installHomeEndKeys } from "./utils";
import "./style.css";

installHomeEndKeys();

// 无标题栏窗口的拖拽：非交互的“背景区”任意按下即拖动窗口（背景拖拽）。
// 保护：按钮/菜单/输入等交互元素与正文文本区豁免（不影响点按与文字选中）。
const DRAG_SAFE_SELECTOR =
  "button, a, input, textarea, select, [role=button], .n-button, .n-menu-item, .n-menu-item-content, .n-input, .n-textarea, .n-select, .tab-item, .rail-item";
const TEXT_ZONE_SELECTOR =
  "p, pre, blockquote, td, th, h1, h2, h3, h4, h5, h6, .md, .md *, .row-title, .n-list-item";

function onDragMouseDown(e: MouseEvent): void {
  if (e.button !== 0) return;
  const el = e.target as HTMLElement | null;
  if (!el) return;
  if (el.closest(DRAG_SAFE_SELECTOR) && !el.closest("[data-tauri-drag-region]")) return;
  if (el.closest(TEXT_ZONE_SELECTOR) && !el.closest("[data-tauri-drag-region]")) return;
  e.preventDefault(); // 拖动时勿选中文字
  void getCurrentWindow().startDragging();
}

document.addEventListener("mousedown", onDragMouseDown);

createApp(App).use(createPinia()).use(router).mount("#app");
