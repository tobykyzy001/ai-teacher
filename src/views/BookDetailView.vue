<template>
  <div class="page">
    <n-button text class="back-btn" @click="router.push('/library')">← 返回书架</n-button>

    <div v-if="loading" class="center-box">
      <n-spin size="large" />
    </div>

    <n-empty v-else-if="error" description="无法加载书籍">
      <div class="muted small empty-extra">{{ error }}</div>
      <n-space>
        <n-button @click="load">重试</n-button>
        <n-button @click="router.push('/library')">返回书架</n-button>
      </n-space>
    </n-empty>

    <template v-else-if="detail">
      <div class="page-header">
        <div class="header-main">
          <h2 class="page-title">{{ detail.book.title }}</h2>
          <div class="page-subtitle">
            {{ detail.book.format.toUpperCase() }} · 导入于 {{ fmtDate(detail.book.added_at) }}
          </div>
          <div class="progress-wrap">
            <n-progress
              type="line"
              :percentage="percent(detail.book.done_units, detail.book.total_units)"
              :height="10"
              :border-radius="5"
            />
          </div>
          <div class="muted small progress-text">
            已学 {{ detail.book.done_units }}/{{ detail.book.total_units }} 个单元（{{
              percent(detail.book.done_units, detail.book.total_units)
            }}%）
          </div>
        </div>
      </div>

      <n-list v-if="detail.units.length > 0" bordered>
        <n-list-item v-for="unit in detail.units" :key="unit.id">
          <div class="unit-row">
            <div class="unit-info">
              <span class="unit-idx">{{ unit.idx + 1 }}</span>
              <span class="unit-title">{{ unit.title }}</span>
              <n-tag size="small" :type="statusType(unit.status)">
                {{ statusLabel(unit.status) }}
              </n-tag>
              <n-tag
                v-if="unit.mastery !== null"
                size="small"
                :bordered="false"
                :type="masteryType(unit.mastery)"
              >
                {{ masteryText(unit.mastery) }}
              </n-tag>
            </div>
            <div class="unit-actions">
              <n-button size="small" @click="goReading(unit.id)">领读</n-button>
              <n-button size="small" type="primary" @click="goQuiz(unit.id)">做题</n-button>
            </div>
          </div>
        </n-list-item>
      </n-list>
      <n-empty v-else description="本书还没有解析出任何单元" />
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  NButton,
  NEmpty,
  NList,
  NListItem,
  NProgress,
  NSpace,
  NSpin,
  NTag,
  useMessage,
} from "naive-ui";
import { api, type BookDetail, type UnitStatus } from "../api";
import { errText, fmtDate, percent } from "../utils";
import { useLibraryStore } from "../stores/library";

type TagType = "default" | "primary" | "info" | "success" | "warning" | "error";

const route = useRoute();
const router = useRouter();
const message = useMessage();
const library = useLibraryStore();

const bookId = computed(() => Number(route.params.bookId));
const invalidId = computed(() => !Number.isInteger(bookId.value) || bookId.value <= 0);

const detail = ref<BookDetail | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);

async function load(): Promise<void> {
  if (invalidId.value) {
    error.value = "无效的书籍链接";
    return;
  }
  loading.value = true;
  error.value = null;
  try {
    const d = await api.getBookDetail(bookId.value);
    detail.value = d;
    library.upsert(d.book);
  } catch (e) {
    error.value = errText(e);
    message.error(`加载书籍失败：${error.value}`);
  } finally {
    loading.value = false;
  }
}

onMounted(load);

function statusType(status: UnitStatus): TagType {
  if (status === "reading") return "warning";
  if (status === "done") return "success";
  return "default";
}

function statusLabel(status: UnitStatus): string {
  if (status === "reading") return "在读";
  if (status === "done") return "已学";
  return "未读";
}

function masteryText(mastery: number | null): string {
  return `掌握度 ${Math.round((mastery ?? 0) * 100)}%`;
}

function masteryType(mastery: number | null): TagType {
  return mastery !== null && mastery >= 0.8 ? "success" : "warning";
}

function goReading(unitId: number): void {
  router.push(`/reading/${unitId}`);
}

function goQuiz(unitId: number): void {
  router.push(`/quiz/${unitId}`);
}
</script>

<style scoped>
.back-btn {
  margin-bottom: 12px;
  font-size: 13px;
}

.progress-wrap {
  margin-top: 14px;
  max-width: 420px;
}

.progress-text {
  margin-top: 6px;
}

.unit-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.unit-info {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  flex: 1;
}

.unit-idx {
  flex-shrink: 0;
  width: 24px;
  height: 24px;
  border-radius: 50%;
  background: #f2f3f5;
  color: rgba(0, 0, 0, 0.55);
  font-size: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.unit-title {
  font-size: 15px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unit-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}
</style>
