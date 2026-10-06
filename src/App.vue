<template>
  <n-message-provider>
    <n-layout has-sider style="height: 100vh">
      <n-layout-sider bordered :width="180">
        <div class="logo">AI 老师</div>
        <n-menu :options="menuOptions" :value="activeKey" @update:value="onSelect" />
      </n-layout-sider>
      <n-layout-content class="main">
        <router-view />
      </n-layout-content>
    </n-layout>
  </n-message-provider>
</template>

<script setup lang="ts">
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import { BookOutline, CalendarOutline, SettingsOutline } from "@vicons/ionicons5";
import { NIcon, NLayout, NLayoutContent, NLayoutSider, NMenu, NMessageProvider } from "naive-ui";
import { h, type Component } from "vue";

function icon(c: Component) {
  return () => h(NIcon, null, { default: () => h(c) });
}

const route = useRoute();
const router = useRouter();

const menuOptions = [
  { label: "今日学习", key: "/today", icon: icon(CalendarOutline) },
  { label: "书架", key: "/library", icon: icon(BookOutline) },
  { label: "设置", key: "/settings", icon: icon(SettingsOutline) },
];

const activeKey = computed(() => {
  if (route.path.startsWith("/library") || route.path.startsWith("/books")) return "/library";
  if (route.path.startsWith("/settings")) return "/settings";
  return "/today";
});

function onSelect(key: string) {
  router.push(key);
}
</script>

<style scoped>
.logo {
  padding: 18px 16px 10px;
  font-size: 18px;
  font-weight: 700;
}

.main {
  padding: 20px 24px;
  overflow: auto;
}
</style>
