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

      <n-list v-if="detail.chapters.length > 0" bordered>
        <template v-for="ch in detail.chapters" :key="ch.first_unit_id">
          <n-list-item class="chapter-row" @click="toggleChapter(ch.first_unit_id)">
            <div class="chapter-line">
              <div class="chapter-info">
                <n-icon
                  class="chapter-chevron"
                  :class="{ open: expandedChapters.has(ch.first_unit_id) }"
                >
                  <ChevronForward />
                </n-icon>
                <span class="chapter-title">{{ ch.title }}</span>
                <span class="muted small chapter-progress">
                  {{ doneCount(ch) }}/{{ ch.units.length }} 小节已学
                </span>
              </div>
              <div class="chapter-actions">
                <n-tooltip>
                  <template #trigger>
                    <n-button
                      size="small"
                      secondary
                      circle
                      :type="chapterSkipped(ch) ? 'success' : 'default'"
                      :disabled="skipBusy"
                      @click.stop="toggleChapterSkip(ch)"
                    >
                      <n-icon size="16">
                        <CheckmarkCircle v-if="chapterSkipped(ch)" />
                        <CheckmarkCircleOutline v-else />
                      </n-icon>
                    </n-button>
                  </template>
                  {{ chapterSkipped(ch) ? "整章已标记无需掌握，点击取消" : "整章标记为无需掌握（不做题，进度照常+1）" }}
                </n-tooltip>
                <n-button size="small" @click.stop="goChapter(ch.first_unit_id)">领读 / 阅读</n-button>
              </div>
            </div>
          </n-list-item>
          <template v-if="expandedChapters.has(ch.first_unit_id)">
            <n-list-item v-for="unit in ch.units" :key="unit.id" class="unit-row-indent">
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
                  <n-tooltip>
                    <template #trigger>
                      <n-button
                        size="small"
                        secondary
                        circle
                        :type="unit.status === 'skipped' ? 'success' : 'default'"
                        :disabled="skipBusy"
                        @click="toggleSkip(unit)"
                      >
                        <n-icon size="16">
                          <CheckmarkCircle v-if="unit.status === 'skipped'" />
                          <CheckmarkCircleOutline v-else />
                        </n-icon>
                      </n-button>
                    </template>
                    {{ unit.status === "skipped" ? "已标记无需掌握，点击取消" : "标记为无需掌握（不做题，进度照常+1）" }}
                  </n-tooltip>
                  <n-button size="small" type="primary" @click="goQuiz(unit.id)">做题</n-button>
                </div>
              </div>
            </n-list-item>
          </template>
        </template>
      </n-list>
      <n-empty v-else description="本书还没有解析出任何单元" />
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { CheckmarkCircle, CheckmarkCircleOutline, ChevronForward } from "@vicons/ionicons5";
import {
  NButton,
  NEmpty,
  NIcon,
  NList,
  NListItem,
  NProgress,
  NSpace,
  NSpin,
  NTag,
  NTooltip,
  useMessage,
} from "naive-ui";
import { api, type BookDetail, type ChapterGroup, type Unit, type UnitStatus } from "../api";
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

const expandedChapters = ref<Set<number>>(new Set());

function toggleChapter(firstUnitId: number): void {
  if (expandedChapters.value.has(firstUnitId)) {
    expandedChapters.value.delete(firstUnitId);
  } else {
    expandedChapters.value.add(firstUnitId);
  }
}

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
  if (status === "skipped") return "info";
  return "default";
}

function statusLabel(status: UnitStatus): string {
  if (status === "reading") return "在读";
  if (status === "done") return "已学";
  if (status === "skipped") return "无需掌握";
  return "未读";
}

function masteryText(mastery: number | null): string {
  return `掌握度 ${Math.round((mastery ?? 0) * 100)}%`;
}

function masteryType(mastery: number | null): TagType {
  return mastery !== null && mastery >= 0.8 ? "success" : "warning";
}

function doneCount(ch: ChapterGroup): number {
  return ch.units.filter((u) => u.status === "done" || u.status === "skipped").length;
}

function chapterSkipped(ch: ChapterGroup): boolean {
  return ch.units.length > 0 && ch.units.every((u) => u.status === "skipped");
}

const skipBusy = ref(false);

// 与后端 set_unit_skipped_impl 语义一致：标记置 skipped、取消回 unread，两者都清掌握度；
// done_units 口径是 done+skipped，按前后状态折算增量。成功后只改本地数据，不重新拉取，页面位置不动。
function applySkipLocally(unit: Unit, skipped: boolean): void {
  const wasCounted = unit.status === "done" || unit.status === "skipped";
  unit.status = skipped ? "skipped" : "unread";
  unit.mastery = null;
  const delta = (skipped ? 1 : 0) - (wasCounted ? 1 : 0);
  if (detail.value && delta !== 0) {
    detail.value.book.done_units += delta;
  }
}

async function toggleSkip(unit: Unit): Promise<void> {
  if (skipBusy.value) return;
  const skipped = unit.status !== "skipped";
  skipBusy.value = true;
  try {
    await api.setUnitSkipped(unit.id, skipped);
    applySkipLocally(unit, skipped);
    library.upsert(detail.value!.book);
  } catch (e) {
    message.error(`标记失败：${errText(e)}`);
  } finally {
    skipBusy.value = false;
  }
}

async function toggleChapterSkip(ch: ChapterGroup): Promise<void> {
  if (skipBusy.value) return;
  const skipped = !chapterSkipped(ch);
  skipBusy.value = true;
  try {
    await api.setChapterSkipped(ch.first_unit_id, skipped);
    ch.units.forEach((u) => applySkipLocally(u, skipped));
    library.upsert(detail.value!.book);
  } catch (e) {
    message.error(`标记失败：${errText(e)}`);
  } finally {
    skipBusy.value = false;
  }
}

function goChapter(firstUnitId: number): void {
  router.push(`/reading/${firstUnitId}`);
}

function goQuiz(unitId: number): void {
  router.push(`/quiz/${unitId}`);
}
</script>

<style scoped>
.back-btn {
  margin-bottom: var(--space-3);
  font-size: var(--text-footnote);
}

.progress-wrap {
  margin-top: 14px;
  max-width: 420px;
}

.progress-text {
  margin-top: 6px;
}

.chapter-row {
  background: var(--fill-4);
  cursor: pointer;
}

.chapter-chevron {
  flex-shrink: 0;
  align-self: center;
  color: var(--label-3);
  transition: transform 0.2s var(--spring);
}

.chapter-chevron.open {
  transform: rotate(90deg);
}

.chapter-line {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}

.chapter-info {
  display: flex;
  align-items: baseline;
  gap: 10px;
  min-width: 0;
  flex: 1;
}

.chapter-title {
  font-size: var(--text-subhead);
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.chapter-progress {
  flex-shrink: 0;
}

.chapter-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-shrink: 0;
}

.unit-row-indent {
  padding-left: var(--space-8);
}

.unit-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
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
  background: var(--fill-3);
  color: var(--label-2);
  font-size: var(--text-caption1);
  display: flex;
  align-items: center;
  justify-content: center;
}

.unit-title {
  font-size: var(--text-subhead);
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unit-actions {
  display: flex;
  gap: var(--space-2);
  flex-shrink: 0;
}
</style>
