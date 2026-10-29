import type { GlobalThemeOverrides } from "naive-ui";

/**
 * Apple HIG 设计令牌 → Naive UI 主题覆盖（v3，严格态）。
 * 所有数值与 src/style.css :root 的令牌一一对应；改这边必须同步那边。
 * 依据 developer.apple.com/design/human-interface-guidelines：
 * - macOS 界面默认 13pt；按钮/输入统一胶囊化（iOS 26 起的系统控件形态）
 * - accent 只承担"主要操作"——primary 实心胶囊；default 按钮不显形（plain 语言）
 * - 语义色 = iOS 系统色；亮色 #34C759/#FF9500/#FF3B30，暗色 #30D158/#FFD60A/#FF453A
 */
function appleTheme(dark: boolean): GlobalThemeOverrides {
  return {
    common: {
      fontFamily: `-apple-system, BlinkMacSystemFont, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif`,
      fontSize: "13px", // macOS 界面默认字号（Typography: macOS 13pt）
      lineHeight: "1.45",

      primaryColor: dark ? "#FFFFFF" : "#1D1D1F",
      primaryColorHover: dark ? "#E8E8ED" : "#3A3A3C",
      primaryColorPressed: dark ? "#D6D6DA" : "#000000",
      primaryColorSuppl: dark ? "#E8E8ED" : "#3A3A3C",
      infoColor: dark ? "#FFFFFF" : "#1D1D1F",
      infoColorHover: dark ? "#E8E8ED" : "#3A3A3C",
      infoColorPressed: dark ? "#D6D6DA" : "#000000",
      warningColor: dark ? "#FFD60A" : "#FF9500",
      warningColorHover: dark ? "#FFDF4F" : "#FFB340",
      warningColorPressed: dark ? "#D4B008" : "#D07E00",
      errorColor: dark ? "#FF453A" : "#FF3B30",
      errorColorHover: dark ? "#FF6961" : "#FF4A3F",
      errorColorPressed: dark ? "#D70015" : "#D70015",
      successColor: dark ? "#30D158" : "#34C759",

      textColorBase: dark ? "#FFFFFF" : "#000000",
      textColor1: dark ? "#FFFFFF" : "#000000",
      textColor2: dark ? "rgba(235, 235, 245, 0.6)" : "rgba(60, 60, 67, 0.6)",
      textColor3: dark ? "rgba(235, 235, 245, 0.3)" : "rgba(60, 60, 67, 0.3)",

      bodyColor: dark ? "#000000" : "#F2F2F7",
      cardColor: dark ? "#1C1C1E" : "#FFFFFF",
      tableColor: dark ? "#1C1C1E" : "#FFFFFF",
      popoverColor: dark ? "#1C1C1E" : "#FFFFFF",
      modalColor: dark ? "#1C1C1E" : "#FFFFFF",
      inputColor: dark ? "#1C1C1E" : "#FFFFFF",
      actionColor: dark ? "#1C1C1E" : "#FFFFFF",

      borderColor: dark ? "rgba(84, 84, 88, 0.6)" : "rgba(60, 60, 67, 0.29)",
      dividerColor: dark ? "rgba(84, 84, 88, 0.6)" : "rgba(60, 60, 67, 0.29)",

      borderRadius: "8px",
      borderRadiusSmall: "8px",
    },

    // —— 按钮：主 = accent 实心胶囊；次 = plain（不显形，accent 文字/默认文字）
    Button: {
      heightLarge: "50px", // iOS 主按钮标准高（≥44 触控）
      heightMedium: "36px",
      heightSmall: "30px",
      heightTiny: "24px",
      paddingLarge: "0 24px",
      paddingMedium: "0 16px",
      paddingSmall: "0 14px",
      paddingTiny: "0 8px",
      fontSizeLarge: "17px",
      fontSizeMedium: "15px",
      fontSizeSmall: "13px",
      fontSizeTiny: "11px",
      fontWeightMedium: "600",
      fontWeightLarge: "600",
      borderRadiusLarge: "999px",
      borderRadiusMedium: "999px",
      borderRadiusSmall: "999px",
      borderRadiusTiny: "999px",
      textColorPrimary: dark ? "#000000" : "#FFFFFF",
      textColorHoverPrimary: dark ? "#000000" : "#FFFFFF",
      textColorPressedPrimary: dark ? "#000000" : "#FFFFFF",
      textColorFocusPrimary: dark ? "#000000" : "#FFFFFF",
      textColorDisabledPrimary: dark
        ? "rgba(0, 0, 0, 0.45)"
        : "rgba(255, 255, 255, 0.6)",
      // default：plain 语言——无底色，默认文字色；hover 才给 fill-4 抬一手
      colorSecondary: "transparent",
      colorSecondaryHover: dark
        ? "rgba(116, 116, 128, 0.18)"
        : "rgba(116, 116, 128, 0.08)",
      colorSecondaryPressed: dark
        ? "rgba(118, 118, 128, 0.24)"
        : "rgba(118, 118, 128, 0.12)",
      colorSecondaryFocus: "transparent",
      borderSecondary: "1px solid transparent",
      borderHoverSecondary: "1px solid transparent",
      borderPressedSecondary: "1px solid transparent",
      textColor: dark ? "#FFFFFF" : "#000000",
      textColorHover: dark ? "#FFFFFF" : "#000000",
      textColorPressed: dark ? "#FFFFFF" : "#000000",
    },

    // —— 输入框：细分隔 + 聚焦 accent 环
    Input: {
      heightMedium: "36px",
      heightLarge: "44px",
      heightSmall: "28px",
      fontSizeMedium: "15px",
      fontSizeLarge: "17px",
      fontSizeSmall: "13px",
      paddingMedium: "2px 12px",
      borderRadius: "8px",
      borderRadiusSmall: "8px",
      border: dark
        ? "0.5px solid rgba(84, 84, 88, 0.6)"
        : "0.5px solid rgba(60, 60, 67, 0.29)",
      borderHover: dark
        ? "0.5px solid rgba(84, 84, 88, 0.8)"
        : "0.5px solid rgba(60, 60, 67, 0.4)",
      borderFocus: "1px solid rgba(175, 82, 222, 0.5)",
      boxShadowFocus: "0 0 0 3px rgba(175, 82, 222, 0.24)",
      color: dark ? "#1C1C1E" : "#FFFFFF",
      colorFocusBackground: dark ? "#1C1C1E" : "#FFFFFF",
    },

    // —— 卡片：内容层标准材质，零投影零玻璃
    Card: {
      borderRadius: "12px",
      paddingMedium: "16px",
      paddingSizeLarge: "20px 24px",
      borderColor: dark ? "rgba(84, 84, 88, 0.6)" : "rgba(60, 60, 67, 0.29)",
      borderWidth: "0.5px",
      boxShadow: "none",
    },

    // —— 标签：胶囊
    Tag: {
      borderRadius: "999px",
      heightTiny: "20px",
      heightSmall: "22px",
      heightMedium: "26px",
      fontSizeTiny: "11px",
      fontSizeSmall: "12px",
      fontSizeMedium: "13px",
    },

    List: { fontSize: "15px" },
    Dialog: { borderRadius: "14px" },

    // —— 侧栏菜单：选中 = accent soft 块（macOS 26 侧栏选中语言）
    Menu: {
      itemHeight: "40px",
      dividerColor: dark ? "rgba(84, 84, 88, 0.6)" : "rgba(60, 60, 67, 0.29)",
      itemColorHover: dark
        ? "rgba(255, 255, 255, 0.07)"
        : "rgba(0, 0, 0, 0.045)",
      itemColorActive: dark
        ? "rgba(120, 120, 128, 0.32)"
        : "rgba(120, 120, 128, 0.16)",
      itemColorActiveHover: dark
        ? "rgba(120, 120, 128, 0.38)"
        : "rgba(120, 120, 128, 0.22)",
      itemTextColor: dark ? "rgba(235, 235, 245, 0.85)" : "rgba(0, 0, 0, 0.85)",
      itemTextColorHover: dark ? "#FFFFFF" : "#000000",
      itemTextColorActive: dark ? "#FFFFFF" : "#000000",
    },

    Message: {
      borderRadius: "999px",
      maxWidth: "440px",
    },

    Switch: {
      railHeightLarge: "31px", // iOS switch 51×31
      railHeightMedium: "24px",
      railHeightSmall: "20px",
    },
  };
}

export const appleThemeLight = appleTheme(false);
export const appleThemeDark = appleTheme(true);
