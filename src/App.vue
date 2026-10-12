<template>
  <n-message-provider>
    <n-layout has-sider style="height: 100vh">
      <n-layout-sider bordered :width="180">
        <div class="logo">AI 老师</div>
        <n-menu :options="menuOptions" :value="activeKey" @update:value="onSelect" />
        <div v-if="showContinue" class="continue-wrap">
          <n-button block secondary type="primary" @click="goContinueReading">
            继续阅读
          </n-button>
        </div>
      </n-layout-sider>
      <n-layout-content class="main">
        <router-view />
      </n-layout-content>
    </n-layout>
  </n-message-provider>
</template>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  BookmarkOutline,
  BookOutline,
  CalendarOutline,
  SettingsOutline,
} from "@vicons/ionicons5";
import {
  NButton,
  NIcon,
  NLayout,
  NLayoutContent,
  NLayoutSider,
  NMenu,
  NMessageProvider,
} from "naive-ui";
import { h, type Component } from "vue";

function icon(c: Component) {
  return () => h(NIcon, null, { default: () => h(c) });
}

const route = useRoute();
const router = useRouter();

const menuOptions = [
  { label: "今日学习", key: "/today", icon: icon(CalendarOutline) },
  { label: "书架", key: "/library", icon: icon(BookOutline) },
  { label: "错题本", key: "/wrong", icon: icon(BookmarkOutline) },
  { label: "设置", key: "/settings", icon: icon(SettingsOutline) },
];

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
</script>

<style scoped>
.logo {
  padding: 18px 16px 10px;
  font-size: 18px;
  font-weight: 700;
}

.continue-wrap {
  padding: 12px 12px 0;
  border-top: 1px solid #efeff5;
  margin-top: 8px;
}

.main {
  padding: 20px 24px;
  overflow: auto;
}
</style>
