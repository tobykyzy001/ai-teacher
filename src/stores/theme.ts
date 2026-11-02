import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import {
  applyThemeToDom,
  isDarkTheme,
  loadTheme,
  saveTheme,
  type ThemeKey,
} from "../theme-mode";

/**
 * 全局主题状态。
 *
 * 为什么用 store 而不是 provide/inject：设置页（改主题）和阅读页（Aa 快捷切换）
 * 是两个不相邻的入口，两者必须永远看到同一个值。store 天然全局单例，
 * 且与项目既有的 useLibraryStore 风格一致。
 *
 * 颜色真正生效靠 style.css 的 [data-theme] 块 —— 这个 store 只做三件事：
 * 写 <html data-theme>、持久化、给 naive-ui 提供"当前是否深色"。
 */
export const useThemeStore = defineStore("theme", () => {
  const systemDarkMedia = window.matchMedia("(prefers-color-scheme: dark)");
  const systemDark = ref(systemDarkMedia.matches);
  systemDarkMedia.addEventListener("change", (e) => {
    systemDark.value = e.matches;
  });

  const key = ref<ThemeKey>(loadTheme());

  // 初始同步一次，避免首帧闪回默认色
  applyThemeToDom(key.value);

  watch(key, (k) => {
    applyThemeToDom(k);
    saveTheme(k);
  });

  /** naive-ui 该用亮色还是暗色（组件配色走 props，不吃 CSS 变量） */
  const isDark = computed(() => isDarkTheme(key.value, systemDark.value));

  function setTheme(next: ThemeKey): void {
    key.value = next;
  }

  return { key, systemDark, isDark, setTheme };
});
