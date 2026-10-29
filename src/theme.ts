import type { GlobalThemeOverrides } from "naive-ui";

/**
 * Apple Human Interface Guidelines 设计令牌 → Naive UI 主题覆盖（v2）。
 *
 * 视觉原则（对标 macOS 26 Finder / Apple Music）：
 * - 侧栏白底并入完整色调，不再追求全窗"奶油渐变"
 * - 激活项：白色圆角块（Apple Music 同款），为层级最重要的导航信号
 * - 按钮：默认按钮 白底+厚边框+内结构，不过度"轻浮"；主按钮维持绿色
 * - 文字层级：rgba 透明黑表达，符合系统无障碍感知
 */
export const appleTheme: GlobalThemeOverrides = {
  common: {
    fontFamily: `-apple-system, BlinkMacSystemFont, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif`,
    fontSize: "15px",
    lineHeight: "1.55",

    // —— 强调色（保留项目既定绿色）
    primaryColor: "#007AFF",
    primaryColorHover: "#3391FF",
    primaryColorPressed: "#0062CC",
    primaryColorSuppl: "#3391FF",
    infoColor: "#007AFF",
    infoColorHover: "#3391FF",
    infoColorPressed: "#0062CC",
    warningColor: "#FF9500",
    warningColorHover: "#FFB340",
    warningColorPressed: "#D07E00",
    errorColor: "#FF3B30",
    errorColorHover: "#FF4A3F",
    errorColorPressed: "#B42545",

    // —— 文字（透明灰，Apple 的层级）
    textColorBase: "rgba(0, 0, 0, 0.87)",
    textColor1: "rgba(0, 0, 0, 0.87)",
    textColor2: "rgba(0, 0, 0, 0.60)",
    textColor3: "rgba(0, 0, 0, 0.42)",

    // —— 背景基底（v2: 全白体系，侧栏不再灰块）
    bodyColor: "#FFFFFF",
    cardColor: "#FFFFFF",
    tableColor: "#FFFFFF",
    popoverColor: "#FFFFFF",

    // —— 边框：Finder 用 1px 实线，但需要轻微存在感
    borderColor: "rgba(0, 0, 0, 0.10)",
    dividerColor: "rgba(0, 0, 0, 0.09)",

    // —— 圆角：苹果 6-8px，窗口级 12px
    borderRadius: "7px",
    borderRadiusSmall: "6px",
  },

  // —— 按钮：Apple 风格，朴素坚固
  Button: {
    heightLarge: "36px",
    heightMedium: "34px",
    heightSmall: "28px",
    heightTiny: "24px",
    paddingLarge: "0 18px",
    paddingMedium: "0 16px",
    paddingSmall: "0 12px",
    paddingTiny: "0 8px",
    fontSizeLarge: "15px",
    fontSizeMedium: "14px",
    fontSizeSmall: "13px",
    fontSizeTiny: "12px",
    borderRadiusLarge: "8px",
    borderRadiusMedium: "7px",
    borderRadiusSmall: "6px",
    borderRadiusTiny: "6px",

    // 默认/次要按钮：白底 + 较实的边框与 hover——Apple 按钮的"存在感"
    colorSecondary: "#FFFFFF",
    colorSecondaryHover: "rgba(0, 0, 0, 0.03)",
    colorSecondaryPressed: "rgba(0, 0, 0, 0.06)",
    colorSecondaryFocus: "#FFFFFF",
    borderSecondary: "1px solid rgba(0, 0, 0, 0.16)",
    borderHoverSecondary: "1px solid rgba(0, 0, 0, 0.22)",
    borderPressedSecondary: "1px solid rgba(0, 0, 0, 0.28)",
    textColor: "rgba(0, 0, 0, 0.87)",
    textColorHover: "rgba(0, 0, 0, 0.75)",
    textColorPressed: "rgba(0, 0, 0, 0.65)",
  },

  // —— 输入框：圆角 7px、内描边、聚焦色环
  Input: {
    heightMedium: "34px",
    heightLarge: "36px",
    heightSmall: "28px",
    fontSizeMedium: "14px",
    fontSizeLarge: "15px",
    fontSizeSmall: "13px",
    paddingMedium: "2px 12px",
    borderRadius: "7px",
    borderRadiusSmall: "6px",
    border: "1px solid rgba(0, 0, 0, 0.16)",
    borderHover: "1px solid rgba(0, 0, 0, 0.22)",
    borderFocus: "1px solid rgba(0, 122, 255, 0.5)",
    boxShadowFocus: "0 0 0 3px rgba(0, 122, 255, 0.16)",
    color: "#FFFFFF",
    colorFocusBackground: "#FFFFFF",
  },

  // —— 卡片：白底 + 厚一点的实边框 + 微 shadow，从奶油底里"站"出来
  Card: {
    borderRadius: "10px",
    paddingMedium: "18px 20px",
    paddingSizeLarge: "20px 24px",
    borderColor: "rgba(0, 0, 0, 0.09)",
    boxShadow: "0 1px 3px rgba(0, 0, 0, 0.04), 0 2px 12px rgba(0, 0, 0, 0.04)",
  },

  // —— 标签：胶囊
  Tag: {
    borderRadius: "12px",
    heightTiny: "20px",
    heightSmall: "22px",
    heightMedium: "26px",
    fontSizeTiny: "11px",
    fontSizeSmall: "12px",
    fontSizeMedium: "13px",
  },

  // —— 列表
  List: {
    fontSize: "15px",
  },

  // —— 对话框/模态
  Dialog: {
    borderRadius: "12px",
  },

  // —— 菜单：选中态用更实的激活色
  Menu: {
    itemHeight: "40px",
    dividerColor: "rgba(0, 0, 0, 0.09)",
    // 侧栏在白底下：激活项为深一点的「选中感」（Apple Music 浅色模式同款思路）
    itemColorHover: "rgba(0, 0, 0, 0.05)",
    itemColorActive: "rgba(0, 0, 0, 0.08)",
    itemColorActiveHover: "rgba(0, 0, 0, 0.10)",
    itemTextColor: "rgba(0, 0, 0, 0.75)",
    itemTextColorHover: "rgba(0, 0, 0, 0.87)",
    itemTextColorActive: "rgba(0, 0, 0, 0.87)",
  },

  // —— 消息/通知
  Message: {
    borderRadius: "10px",
    maxWidth: "440px",
  },

  // —— 开关
  Switch: {
    railHeightLarge: "28px",
    railHeightMedium: "24px",
    railHeightSmall: "20px",
  },
};
