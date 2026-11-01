<template>
  <n-config-provider
    :theme="isDark ? darkTheme : null"
    :theme-overrides="isDark ? appleThemeDark : appleThemeLight"
  >
    <n-message-provider>
      <n-layout has-sider style="height: 100vh">
      <!-- 收起目标 = 64px 图标栏（而不是 0 宽）：导航常驻可点，文字瞬时消失、只留图标。
           之前"字自己折起来"的成因是两段 .3s 过渡叠加：sider 宽度在收窄，菜单文字列又同时
           在淡出，于是文字被正在变窄的容器从右往左切掉。这里把「收起」方向的过渡时长压成 0
           （见 .app-sider.collapsed 那组规则），点击那一刻文字就没了，全程只有宽度和图标在动；
           「展开」方向不动，文字照旧淡入、"继续阅读"由变宽的侧栏自然揭示 -->
      <n-layout-sider
        v-if="!isMobile && !isTablet"
        :width="220"
        collapse-mode="width"
        :collapsed-width="64"
        :collapsed="siderHidden"
        class="app-sider"
        :class="{ collapsed: siderHidden }"
      >
      <!-- 顶部空带：macOS Overlay 红绿灯压在这一块，不放任何可点内容（标题"AI 伴读"已移除，
           应用名留在窗口标题/Dock 上）；高度只需容下红绿灯，列表因此整体上移、更紧凑 -->
        <div class="sider-top" aria-hidden="true"></div>
        <!-- 收放钮不放列表里：放进去时它的 hover/阴影框是整条 0–64，
             而导航选中胶囊是 8–56，收起后两者宽度对不齐。改为固定悬浮圆钮，
             骑在"菜单列 / 正文列"的交界缝上（见模板末尾的 .edge-fab） -->
        <!-- 折叠态的文字气泡由 n-menu 自己提供（dropdownEnabled = root && collapsed），
             不要再挂 title 属性，否则会和它同时弹。
             collapsed-width 给 64（= 图标栏宽度），配合收起态把菜单横向内边距清零：
             图标水平中心 = 菜单内边距 + (collapsedWidth/2 - iconSize/2) + iconSize/2 = 32，
             正好是 64px 导轨的中线，选中块也以它为中心 -->
        <n-menu
          :options="menuOptions"
          :value="activeKey"
          :collapsed="siderHidden"
          :collapsed-width="64"
          @update:value="onSelect"
        />
        <div
          v-if="showContinue"
          class="continue-wrap"
        >
          <n-button
            class="continue-btn"
            block
            secondary
            type="primary"
            @click="goContinueReading"
          >
            <n-icon class="continue-icon" :size="20">
              <ArrowRedoCircleOutline />
            </n-icon>
            <span class="continue-label">继续阅读</span>
          </n-button>
        </div>
      </n-layout-sider>
      <!-- 平板区间：自绘 64px 图标栏 -->
      <aside v-else-if="isTablet" class="tablet-rail">
        <div class="rail-logo">伴</div>
        <button
          v-for="item in menuOptions"
          :key="item.key"
          class="rail-item"
          :class="{ active: activeKey === item.key }"
          @click="onSelect(item.key as string)"
        >
          <span class="rail-icon">
            <component :is="item.icon" />
          </span>
        </button>
        <button
          v-if="showContinue"
          class="rail-item continue"
          @click="goContinueReading"
        >
          <span class="rail-icon">
            <n-icon :size="22"><ArrowRedoCircleOutline /></n-icon>
          </span>
        </button>
      </aside>
      <n-layout-content
        ref="mainRef"
        class="main"
        :class="{
          mobile: isMobile,
          reading: isReadingRoute,
        }"
      >
        <!-- 页面切换：极短淡入淡出（无位移）。只做透明度、不加位移动画——
             HIG Motion 要求"简短可中断"，而位移入场在大块内容上来回时反而显廉价。
             key 用 route.path 而非 fullPath：同页换 query（如做题进度）不重播动画。
             必须 mode="out-in"：并发模式下新旧两页会同时存在于文档流、垂直堆叠，
             页面高度瞬间翻倍、滚动条跳一下；out-in 先退后进，总时长约 90+120ms。 -->
        <transition name="page-fade" mode="out-in">
          <router-view v-slot="{ Component }">
            <component :is="Component" :key="route.path" />
          </router-view>
        </transition>
      </n-layout-content>
      <!-- 收放钮：固定悬浮的液态玻璃圆钮，两个端点各有明确归属——
           展开态「«」骑在"菜单列 / 正文列"的接缝上（left 204，圆心正压缝 x=220）；
           收起态「≡」则沉进图标列、与那 4 个缩小图标同一竖列居中（left 16，圆心 x=32，
           正好等于收起态菜单图标的中心），不再停在接缝上显游离。
           纵向不动（top 28），只做水平平移，与 naive sider 同为 .3s 标准缓动。
           窗口拖动/双击缩放统一由 main.ts 按"背景区"判定（非按钮/输入/正文），无需额外拖动层 -->
      <button
        v-if="!isMobile && !isTablet"
        class="edge-fab"
        type="button"
        :style="{ left: siderHidden ? '16px' : '204px' }"
        :title="siderHidden ? '展开菜单栏' : '收起菜单栏'"
        :aria-label="siderHidden ? '展开菜单栏' : '收起菜单栏'"
        :aria-expanded="!siderHidden"
        @click="setSiderHidden(!siderHidden)"
      >
        <n-icon v-if="siderHidden" :size="18"><MenuOutline /></n-icon>
        <svg
          v-else
          viewBox="0 0 24 24"
          width="19"
          height="19"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <polyline points="11,6 5,12 11,18" />
          <polyline points="17,6 11,12 17,18" />
        </svg>
      </button>
      <nav v-if="isMobile && !isReadingRoute" class="tab-bar">
        <button
          v-for="item in menuOptions"
          :key="item.key"
          class="tab-item"
          :class="{ active: activeKey === item.key }"
          @click="onSelect(item.key as string)"
        >
          <span class="tab-icon">
            <component :is="item.icon" />
          </span>
          <span class="tab-label">{{ item.label }}</span>
        </button>
        <button
          v-if="showContinue"
          class="tab-item continue"
          @click="goContinueReading"
        >
          <span class="tab-icon">
            <n-icon :size="22"><ArrowRedoCircleOutline /></n-icon>
          </span>
          <span class="tab-label">继续阅读</span>
        </button>
      </nav>
      </n-layout>
    </n-message-provider>
  </n-config-provider>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  BookmarkOutline,
  ArrowRedoCircleOutline,
  BookOutline,
  CalendarOutline,
  MenuOutline,
  SettingsOutline,
} from "@vicons/ionicons5";
import {
  darkTheme,
  NButton,
  NConfigProvider,
  NIcon,
  NLayout,
  NLayoutContent,
  NLayoutSider,
  NMenu,
  NMessageProvider,
} from "naive-ui";
import { h, type Component } from "vue";
import { appleThemeDark, appleThemeLight } from "./theme";

// 跟随系统明暗（HIG Dark Mode：所有颜色提供 light/dark 双值，随外观自动切换）
const darkMedia = window.matchMedia("(prefers-color-scheme: dark)");
const isDark = ref(darkMedia.matches);
darkMedia.addEventListener("change", (e) => {
  isDark.value = e.matches;
});

function icon(c: Component) {
  return () => h(NIcon, null, { default: () => h(c) });
}

const route = useRoute();
const router = useRouter();

// 端设备自适应断点（CSS px）：
// ≤820 手机/iPad mini 竖屏 → 侧边栏换底部 Tab 栏；821–1100 iPad 竖屏/窄窗口 → 图标侧栏
const MOBILE_MAX = 820;
const TABLET_MAX = 1100;

const windowWidth = ref(window.innerWidth);
const isMobile = computed(() => windowWidth.value <= MOBILE_MAX);
const isTablet = computed(
  () => windowWidth.value > MOBILE_MAX && windowWidth.value <= TABLET_MAX,
);

function onResize() {
  windowWidth.value = window.innerWidth;
}

onMounted(() => window.addEventListener("resize", onResize));
onUnmounted(() => window.removeEventListener("resize", onResize));

const menuOptions = [
  { label: "今日学习", key: "/today", icon: icon(CalendarOutline) },
  { label: "书架", key: "/library", icon: icon(BookOutline) },
  { label: "错题本", key: "/wrong", icon: icon(BookmarkOutline) },
  { label: "设置", key: "/settings", icon: icon(SettingsOutline) },
];

const isReadingRoute = computed(() => route.path.startsWith("/reading/"));

const activeKey = computed(() => {
  if (
    route.path.startsWith("/library") ||
    route.path.startsWith("/books") ||
    route.path.startsWith("/reading") ||
    route.path.startsWith("/quiz")
  ) {
    return "/library";
  }
  if (route.path.startsWith("/wrong")) return "/wrong";
  if (route.path.startsWith("/settings")) return "/settings";
  return "/today";
});

function onSelect(key: string) {
  router.push(key);
}

// 记住最近阅读位置（进过任意阅读页即更新），随时一键回去；localStorage 持久，重开应用也在
const LAST_READING_KEY = "ai-teacher:last-reading";

function loadLastReading(): string | null {
  try {
    return localStorage.getItem(LAST_READING_KEY);
  } catch {
    return null;
  }
}

const lastReading = ref<string | null>(loadLastReading());

watch(
  () => route.path,
  (p) => {
    if (!p.startsWith("/reading/")) return;
    lastReading.value = p;
    try {
      localStorage.setItem(LAST_READING_KEY, p);
    } catch {
      /* 本地存储不可用时仅本次运行内记忆 */
    }
  },
);

const showContinue = computed(
  () => lastReading.value !== null && !route.path.startsWith("/reading/"),
);

function goContinueReading(): void {
  if (lastReading.value) router.push(lastReading.value);
}

// 侧栏收起（仅宽屏布局生效；平板图标栏/手机 Tab 栏本就紧凑，不做收起）：状态本地记忆，重开应用保持
const SIDER_HIDDEN_KEY = "ai-teacher:sider-hidden";

function loadSiderHidden(): boolean {
  try {
    return localStorage.getItem(SIDER_HIDDEN_KEY) === "1";
  } catch {
    return false;
  }
}

const siderHidden = ref(loadSiderHidden());

function setSiderHidden(v: boolean): void {
  siderHidden.value = v;
  try {
    localStorage.setItem(SIDER_HIDDEN_KEY, v ? "1" : "0");
  } catch {
    /* 本地存储不可用时仅本次运行内记忆 */
  }
}

// 换页时把主内容区滚回顶部：滚动容器 .main 为所有页面共享，长页滚到底再进短页时
// 旧滚动位置会把新页内容顶出视口（表现为"点去做题白屏"）
const mainRef = ref<InstanceType<typeof NLayoutContent> | null>(null);
watch(
  () => route.fullPath,
  async () => {
    await nextTick();
    (mainRef.value?.$el as HTMLElement | undefined)?.scrollTo({ top: 0 });
  },
);
</script>

<style scoped>
/* ---------- 桌面侧栏（Apple 总支栏体系） ---------- */

.app-sider {
  background: var(--bg-window);
  border-right: 0.5px solid var(--separator);
}

/* 顶部空带：给 macOS Overlay 红绿灯腾位（拖窗由 main.ts 的背景区判定接手）。
   88px 是"红绿灯 + 悬浮收放钮 + 上下各约 17px 留白"算出来的：
     红绿灯下缘约 y=27 → 钮顶 44（留 17px）→ 钮底 76 → 菜单首项 94（留 18px）
   为什么必须这么高：收起态侧栏只有 64px 宽，而红绿灯横向约占 x 14–66，
   整列在顶部被红绿灯占满，位于 x=32 的收放钮只要纵向进入 y 13–27 就会压住
   关闭/最小化按钮。所以"既不压红绿灯、又不贴菜单"只能靠把菜单整体下推 */
.sider-top {
  height: 88px;
  flex-shrink: 0;
}

/* 收起态：侧栏收成 64px 图标栏，护栏线保留 */
.app-sider.collapsed {
  border-right-color: var(--separator);
}

.app-sider :deep(.n-layout-sider__content) {
  background: transparent;
}

/* 菜单：左右与底部保留 12px 内边距，**顶部清零**——第一项自带 margin-top:6px，
   若再叠上 padding-top:12px，收放钮到第一个导航项就会出现 20px 的 gap，
   与项间 6px 的节奏不统一（收放钮现在就是列表第一项，间距必须同节奏） */
.app-sider :deep(.n-menu) {
  background: transparent;
  padding: 0 var(--space-3) var(--space-3);
}

/* 收起态菜单横向内边距清零：图标水平中心 = 内边距 + (collapsedWidth/2 − iconSize/2)
   + iconSize/2。留 12px 的话图标会落到 x=44（导轨中线是 32），选中块的 ::before
   （inset 8px）也会被挤偏；清零后图标正落在 64px 导轨中线上。
   纵向 padding 不动（已在 .n-menu 统一为 top 0），收放钮自身的 margin 同步归零，
   它才能和导航图标一样汇到 x=32 那一列 */
.app-sider.collapsed :deep(.n-menu) {
  padding-left: 0;
  padding-right: 0;
}

/* ---------- 文字瞬时消失 / 延迟出现（侧栏里只留图标） ----------
   起因：naive 的侧栏宽度是 .3s 过渡，菜单文字列自带 .3s 的 opacity 过渡，两者同时进行，
   眼睛就会看到"文字被正在变窄的容器从右往左挤扁"这一过程。这里把两个方向分别定死：
   - 收起（.collapsed 命中）：transition:none，点击那一刻文字直接消失，不参与任何渐变；
   - 展开（.collapsed 移除）：带 .18s 延迟再淡入，等宽度基本铺开了文字才出现，
     不会在 64px 宽度里先被裁一刀。收起态那条 transition:none 是简写，会把
     transition-delay 一并重置为 0，所以延迟只作用于展开方向，互不干扰 */
.app-sider :deep(.n-menu-item-content-header) {
  transition:
    color var(--dur) ease,
    opacity var(--dur-fast) ease 0.18s;
}

.app-sider.collapsed :deep(.n-menu-item-content-header),
.app-sider.collapsed :deep(.n-menu-item-content__arrow) {
  transition: none;
}

/* 折叠态图标由 naive 按 collapsedWidth 水平居中；文字列虽已 opacity:0 但仍占着 1fr 宽度，
   压成 0 才不会把图标挤出导轨中线 */
.app-sider.collapsed :deep(.n-menu-item-content-header) {
  width: 0;
  overflow: hidden;
}

/* 同理收掉右侧 18px 的箭头内边距（折叠态箭头本就 opacity:0），
   否则 24px 图标 + 16px 箭头会撑破 64px 宽的项、把图标顶偏 */
.app-sider.collapsed :deep(.n-menu-item-content) {
  padding-right: 0;
}

.app-sider.collapsed :deep(.n-menu-item-content__arrow) {
  display: none;
}

/* 菜单项：macOS 26 侧栏语言——选中 = accent soft 圆角块（8px 同心） */
.app-sider :deep(.n-menu-item) {
  border-radius: var(--radius-control);
}

/* 收起时图标不放大：naive 默认 collapsedIconSize=24 / iconSize=20，折叠动画里
   图标会同时"长大 20%"——用户会察觉成"图标突然变大了"，属意外效果。
   这里把折叠态的字形尺寸钉回 20px，与展开态一致，收起过程只发生位移、不发生缩放。
   注意两点：
   1. 必须 !important——尺寸是 naive 写在图标 div 上的内联 style（fontSize），
      普通样式表规则压不住内联；
   2. 只改 font-size，不动图标盒（width/height 两态都是 24px = maxIconSize），
      所以图标的水平中心仍在 x=32，布局零位移。
   （原先这里写的 `--n-item-icon-size: 18px` 是死代码：naive 的 CSS 里没有这个
     变量，写了也从未生效过。） */
.app-sider.collapsed :deep(.n-menu-item-content__icon) {
  font-size: 20px !important;
}

.app-sider :deep(.n-menu-item-content) {
  border-radius: var(--radius-control);
}

.app-sider :deep(.n-menu-item-content--selected) {
  font-weight: 600;
}

.continue-wrap {
  padding: var(--space-3) var(--space-3) 0;
  border-top: 0.5px solid var(--separator);
  margin-top: var(--space-2);
}

/* 收起态：文字"继续阅读"瞬间消失，只剩图标（与图标栏其余项同语言）。
   图标与文字是同一个按钮里的两个节点：展开态只显示文字（保持改动前的样子），
   收起态只显示图标并让按钮塌成 36×36 的小方块、居中对齐导轨中线，
   都按 display 切换，不产生中间过渡态 */
.continue-btn .continue-icon {
  display: none;
}

.app-sider.collapsed .continue-wrap {
  padding: var(--space-3) 0 0;
  display: flex;
  justify-content: center;
}

.app-sider.collapsed .continue-btn.n-button {
  width: 36px !important;
  min-width: 36px;
  padding: 0 !important;
}

.app-sider.collapsed .continue-btn .continue-label {
  display: none;
}

.app-sider.collapsed .continue-btn .continue-icon {
  display: inline-flex;
  margin: 0;
}

/* ---------- 平板区间 64px 图标栏 ---------- */

.tablet-rail {
  width: 64px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 2px;
  padding: 26px 8px 12px;
  background: var(--bg-window);
  border-right: 0.5px solid var(--separator);
}

.rail-logo {
  padding: 0 0 14px;
  font-size: var(--text-subhead);
  font-weight: 600;
  text-align: center;
  color: var(--label);
}

.rail-item {
  display: flex;
  align-items: center;
  justify-content: center;
  height: var(--hit);
  border: none;
  border-radius: var(--radius-control);
  background: none;
  color: var(--label-2);
  cursor: pointer;
  transition: background-color var(--dur-fast) ease;
  width: 100%;
}

.rail-item:hover {
  background: var(--fill-4);
}

.rail-item.active {
  background: var(--accent-fill);
  color: var(--accent);
}

.rail-item.continue {
  color: var(--accent);
  margin-top: auto;
  border-top: 0.5px solid var(--separator);
  border-radius: 0;
}

.rail-icon {
  display: inline-flex;
  font-size: 21px;
  line-height: 1;
}

/* ---------- 主内容区 ---------- */

.main {
  padding: var(--space-5) var(--space-6);
  overflow: auto;
  background: var(--bg-grouped);
}

.main.mobile {
  padding: 16px 16px;
  padding-bottom: calc(86px + env(safe-area-inset-bottom, 0px));
}

/* 阅读页全屏：Tab 栏收起后不再预留其占位 */
.main.mobile.reading {
  padding-bottom: calc(24px + env(safe-area-inset-bottom, 0px));
}

/* ---------- 收放钮：骑缝悬浮玻璃圆钮 ---------- */

/* 复用阅读页 .side-arrow 的液态玻璃配方。只做水平平移，与 naive sider 的 .3s 同缓动：
   - 展开 left 204 → 圆心 x=220，正压菜单列/正文列的接缝，右半落在内容 24px 左内边距的空白带；
   - 收起 left 16  → 圆心 x=32，与收起态那 4 个导航图标同列居中（图标中心也是 32），
     钮因此成为图标列的"第 0 项"，而不是飘在接缝上。
   圆形 + 玻璃质感与导航项的圆角块是两套语义：这是"浮在内容之上的控件"，
   放进列表里反而会因 hover 框宽度（0–64）与选中胶囊（8–56）不一致而显得没对齐 */
.edge-fab {
  position: fixed;
  /* 纵向落在"红绿灯之下、菜单之上"的空白里，上下留白对称：
     红绿灯下缘约 y=27 → 钮顶 44（间隙 17）→ 钮底 76 → 菜单首项 94（间隙 18）。
     上一版误改成 top:8（纵跨 8–40）正好盖住红绿灯——收起态钮在 x=32 与红绿灯同列，
     那条区间必须完全避开 */
  top: 44px;
  /* 高于阅读页悬浮箭头(20)/点按区(10)，低于划词弹窗(1000)与 naive 模态 */
  z-index: 800;
  width: 32px;
  height: 32px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
  color: var(--label-2);
  background: var(--glass-sheen), var(--glass-bg);
  -webkit-backdrop-filter: blur(20px) saturate(170%);
  backdrop-filter: blur(20px) saturate(170%);
  border: none;
  box-shadow:
    var(--glass-rim),
    0 1px 6px rgba(0, 0, 0, 0.1);
  outline: none;
  cursor: pointer;
  user-select: none;
  transition:
    left 0.3s cubic-bezier(0.4, 0, 0.2, 1),
    background 0.15s,
    color 0.15s;
}

.edge-fab:hover,
.edge-fab:focus-visible {
  color: var(--accent);
}

/* 拖窗/双击缩放由 main.ts 按"背景区"判定（非按钮/输入/正文），无需额外拖动层 */

/* ---------- 页面切换过渡 ---------- */

/* 纯透明度，无位移。出于两点：
   1. 位移入场会让长页在切换时"整体窜一下"，观感比直接切换更廉价；
   2. out-in 模式下两页不共存，透明度动画不会引起布局跳动。
   淡出略快于淡入（90 vs 120ms）：离开要干脆，进入可以稍缓。 */
.page-fade-enter-active {
  transition: opacity 0.12s ease;
}

.page-fade-leave-active {
  transition: opacity 0.09s ease;
}

.page-fade-enter-from,
.page-fade-leave-to {
  opacity: 0;
}

/* 减弱动效偏好：切页直接到位 */
@media (prefers-reduced-motion: reduce) {
  .page-fade-enter-active,
  .page-fade-leave-active {
    transition: none !important;
  }
}

/* ---------- 手机端 Tab 栏 ---------- */

.tab-bar {
  position: fixed;
  left: 16px;
  right: 16px;
  bottom: calc(12px + env(safe-area-inset-bottom, 0px));
  z-index: 100;
  display: flex;
  align-items: stretch;
  border-radius: var(--radius-pill);
  overflow: hidden;
  background: var(--glass-sheen), var(--glass-bg);
  -webkit-backdrop-filter: var(--glass-blur);
  backdrop-filter: var(--glass-blur);
  border: none;
  box-shadow: var(--glass-rim), var(--glass-drop);
}

.tab-item {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 3px;
  padding: 9px 4px 7px;
  border: none;
  background: none;
  color: var(--label-2);
  font-size: var(--text-caption2);
  cursor: pointer;
  min-width: 0;
  /* Apple 规范：可点触控区 ≥44pt */
  min-height: var(--hit);
  transition: color var(--dur-fast) var(--spring);
}

.tab-item.active {
  color: var(--accent);
  /* iOS 26 选中项：玻璃透镜——中性 fill + 顶部高光，不画实色盒 */
  background: var(--fill-4);
  border-radius: var(--radius-pill);
  margin: 6px;
  padding: 3px 0 1px;
  box-shadow: var(--glass-highlight);
}

.tab-item.continue {
  color: var(--accent);
}

.tab-icon {
  display: inline-flex;
  /* SF Symbols Tab 栏图标的常见光学尺寸 */
  font-size: 24px;
  line-height: 1;
}

.tab-label {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 100%;
  font-weight: 500;
}

/* 阅读页：外层不再整页滚动，正文列与领读列各在自己内部滚（各一条滚动条）。
   ⚠ n-layout-content 内部会再套两层 .n-layout-scroll-container，所以 .reading-page 不是 .main 的直接子元素：
   - 不能用 .main:has(> .reading-page)（一直没命中，两列被内容撑高、滚动全落在整页上，只剩一条滚动条）
   - 也不能只给 .reading-page 设 display:flex; flex:1：父级是 block，flex 子项不会收缩，照样被内容撑高
   必须像下面这样把每一层都钉成定高/定网格，最后两列各自 overflow-y:auto 才有各自的滚动条 */
.main:has(.reading-page) {
  height: 100%;
  overflow: hidden;
}

/* n-layout-content 内部因此多出两层 .n-layout-scroll-container（外层 flex 行、内层滚动容器），
   逐层把高度钉死为 100% 并禁用它们自身的滚动，滚动交给正文列 / 领读列 */
.main:has(.reading-page) .n-layout-scroll-container {
  height: 100%;
  min-height: 0;
  overflow: hidden;
}

.main:has(.reading-page) > .n-layout-scroll-container {
  display: flex;
  flex-direction: column;
}

.main:has(.reading-page) > .n-layout-scroll-container > .n-layout-scroll-container {
  flex: 1;
  padding: 20px 24px;
  box-sizing: border-box;
  display: grid;
  grid-template-rows: minmax(0, 1fr);
}

.main:has(.reading-page) .reading-page {
  min-height: 0;
  height: 100%;
  padding-bottom: 0;
}

.main:has(.reading-page) .reading-main {
  display: flex;
  min-height: 0;
  align-items: stretch;
}

@media (max-width: 900px) {
  .main:has(.reading-page) {
    height: auto;
    overflow: auto;
  }

  .main:has(.reading-page) .reading-page {
    display: block;
    padding-bottom: 48px;
  }
}

/* 减弱动态偏好：收起/展开改为瞬时切换（sider 宽度过渡与收放钮平移一并关掉） */
@media (prefers-reduced-motion: reduce) {
  .main,
  .app-sider,
  .edge-fab {
    transition: none !important;
  }
}
</style>
