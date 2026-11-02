/**
 * 全局主题：4 套纸张主题 + 跟随系统。
 *
 * 设计要点（与 style.css 的 [data-theme] 块一一对应）：
 * - 主题只改 <html data-theme>，颜色全部由 CSS 变量驱动 —— 不重新渲染组件、
 *   不加载第二份样式表、也没有"JS 主题对象"需要各页面去读。
 * - 选择持久化在 localStorage（键 THEME_KEY），跨页面、重启应用都保持一致。
 * - naive-ui 的组件配色（按钮/输入/卡片…）无法只靠 CSS 变量覆盖，
 *   仍需要一份 themeOverrides；它按"亮/暗"两档提供，具体由 effectiveDark 决定。
 */

export type ThemeKey = "light" | "sepia" | "slate" | "night" | "system";

export interface ThemeOption {
  key: ThemeKey;
  label: string;
  /** 取色用的代表色（设置页色板） */
  swatch: string;
  /** 边框色，深色主题下让色板边界可见 */
  border: string;
}

export const THEME_OPTIONS: ThemeOption[] = [
  { key: "light", label: "纸白", swatch: "#FFFFFF", border: "rgba(60,60,67,0.29)" },
  { key: "sepia", label: "暖纸", swatch: "#F8F2E2", border: "rgba(120,90,55,0.3)" },
  { key: "slate", label: "深灰", swatch: "#48484A", border: "rgba(120,120,128,0.6)" },
  { key: "night", label: "夜黑", swatch: "#1C1C1E", border: "rgba(84,84,88,0.6)" },
  { key: "system", label: "跟随系统", swatch: "linear-gradient(135deg,#FFFFFF 50%,#1C1C1E 50%)", border: "rgba(60,60,67,0.29)" },
];

export const THEME_KEY = "ai-teacher:theme";

/** 只有这几种是深色底（system 需另算） */
const DARK_THEMES: ThemeKey[] = ["slate", "night"];

export function loadTheme(): ThemeKey {
  try {
    const v = localStorage.getItem(THEME_KEY);
    if (v && THEME_OPTIONS.some((t) => t.key === v)) return v as ThemeKey;
  } catch {
    /* 本地存储不可用时用默认值 */
  }
  return "light";
}

export function saveTheme(key: ThemeKey): void {
  try {
    localStorage.setItem(THEME_KEY, key);
  } catch {
    /* 本地存储不可用时仅本次运行生效 */
  }
}

/** 该主题最终是不是深色底（决定 naive-ui 用暗色主题还是亮色） */
export function isDarkTheme(key: ThemeKey, systemDark: boolean): boolean {
  if (key === "system") return systemDark;
  return DARK_THEMES.includes(key);
}

/**
 * 把主题写进 <html data-theme>。
 * light 时移除属性（回到 :root 默认值），其余写法与 style.css 的选择器一致。
 */
export function applyThemeToDom(key: ThemeKey): void {
  const root = document.documentElement;
  if (key === "light") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", key);
}
