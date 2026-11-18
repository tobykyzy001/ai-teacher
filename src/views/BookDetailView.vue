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
          <n-space class="book-tools">
            <n-button class="btn-tinted" @click="searchShow = true">书内搜索</n-button>
            <n-button class="btn-tinted" @click="goalShow = true">学习目标</n-button>
          </n-space>
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
                  <n-button
                    v-if="unit.status === 'unread' && unit.mastery === null"
                    size="small"
                    @click="openResplit(unit)"
                  >拆分</n-button>
                  <n-button v-if="detail?.resplit_unit_ids.includes(unit.id)" size="small" quaternary @click="undoResplit(unit)">撤销拆分</n-button>
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

    <n-modal v-model:show="searchShow" preset="card" title="书内搜索" :style="{ width: '680px', maxWidth: '92vw' }">
      <n-space vertical :size="12">
        <n-input v-model:value="searchQuery" clearable placeholder="输入正文或小节标题中的关键词" @keydown="onSearchKeydown" />
        <n-button type="primary" :loading="searching" @mousedown="commitComposingInput" @click="runSearch">搜索</n-button>
        <n-alert v-if="searchError" type="error">{{ searchError }}</n-alert>
        <n-empty v-else-if="searched && hits.length === 0" description="没有命中，请换一个关键词。" />
        <div v-if="hits.length" class="search-results">
          <n-button v-for="(hit, index) in hits" :key="`${hit.unit_id}-${hit.start}-${index}`" text class="search-hit" @click="goHit(hit)">
            <span class="search-hit-title">{{ hit.chapter_title }} · {{ hit.unit_title }}</span>
            <span>{{ hit.context }}</span>
          </n-button>
        </div>
      </n-space>
    </n-modal>

    <n-modal v-model:show="goalShow" preset="card" title="学习目标" :style="{ width: '460px', maxWidth: '92vw' }">
      <n-form label-placement="top" @submit.prevent>
        <n-form-item label="在几周内读完这本书？">
          <n-input-number v-model:value="goalWeeks" :min="1" :max="104" :precision="0" style="width: 100%" />
        </n-form-item>
        <p class="muted small">目标从今天起计算，只安排阅读进度；到期复习仍完整保留。每天的小节数按剩余天数自动均摊。</p>
        <n-space>
          <n-button type="primary" :loading="goalBusy" @click="saveGoal">保存目标</n-button>
          <n-button :loading="goalBusy" @click="clearGoal">清除目标</n-button>
        </n-space>
      </n-form>
      <n-alert v-if="detail?.learning_goal" type="info">
        已设 {{ detail.learning_goal.weeks }} 周目标，起始日 {{ detail.learning_goal.started_at }}，剩余 {{ detail.learning_goal.remaining_units }} 个小节。可在「今日学习」查看每日安排。
      </n-alert>
    </n-modal>

    <n-modal v-model:show="resplitShow" preset="card" title="调整小节粒度" :style="{ width: '520px', maxWidth: '92vw' }" :mask-closable="!resplitBusy">
      <p class="muted small">拆分「{{ resplitUnitRow?.title }}」，保留本章连续阅读。已有题目、复习、掌握度或章讲解的小节不能拆分。</p>
      <n-form-item label="每个小节目的字数（300–5000）">
        <n-input-number v-model:value="targetChars" :min="300" :max="5000" :step="100" :precision="0" style="width: 100%" />
      </n-form-item>
      <n-space>
        <n-button :loading="previewBusy" @click="loadPreview">预览</n-button>
        <n-button v-if="preview" type="primary" :loading="resplitBusy" @click="commitResplit">确认拆分</n-button>
        <n-button @click="resplitShow = false">取消</n-button>
      </n-space>
      <n-list v-if="preview" bordered class="split-preview">
        <n-list-item v-for="title in preview.preview" :key="title">{{ title }}</n-list-item>
      </n-list>
      <p v-if="preview" class="muted small">确认后将产生 {{ preview.unit_count }} 个小节。可通过撤销恢复原正文与标题；新子小节开始学习后不能撤销。</p>
    </n-modal>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { CheckmarkCircle, CheckmarkCircleOutline, ChevronForward } from "@vicons/ionicons5";
import {
  NButton,
  NEmpty,
  NIcon,
  NList,
  NListItem,
  NInput,
  NInputNumber,
  NModal,
  NForm,
  NFormItem,
  NAlert,
  NProgress,
  NSpace,
  NSpin,
  NTag,
  NTooltip,
  useMessage,
} from "naive-ui";
import { api, type BookDetail, type ChapterGroup, type ResplitPreview, type SearchHit, type Unit, type UnitStatus } from "../api";
import { commitComposingInput, errText, fmtDate, percent } from "../utils";
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
    goalWeeks.value = d.learning_goal?.weeks ?? 4;
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

// ---------- 全书检索 ----------
const searchShow = ref(false);
const searchQuery = ref("");
const searching = ref(false);
const searched = ref(false);
const searchError = ref("");
const hits = ref<SearchHit[]>([]);
let searchGeneration = 0;

function onSearchKeydown(e: KeyboardEvent): void {
  if (e.key === "Enter" && !e.isComposing && e.keyCode !== 229) void runSearch();
}

watch(searchQuery, () => { searched.value = false; searchError.value = ""; hits.value = []; });

async function runSearch(): Promise<void> {
  if (searching.value) return;
  const query = searchQuery.value.trim();
  if (!query) { message.warning("请先输入要查找的内容"); return; }
  searching.value = true;
  searchError.value = "";
  const generation = ++searchGeneration;
  try {
    const result = await api.searchBook(bookId.value, query);
    if (generation !== searchGeneration) return;
    hits.value = result;
    searched.value = true;
  } catch (e) {
    if (generation !== searchGeneration) return;
    searchError.value = errText(e);
    hits.value = [];
  } finally {
    if (generation === searchGeneration) searching.value = false;
  }
}

function goHit(hit: SearchHit): void {
  searchShow.value = false;
  void router.push({ path: `/reading/${hit.unit_id}`, query: { focus: hit.unit_id, q: hit.quote, start: hit.start, titleSearch: hit.title_search ? 1 : 0 } });
}

// ---------- 每书目标 ----------
const goalShow = ref(false);
const goalWeeks = ref<number>(4);
const goalBusy = ref(false);

async function saveGoal(): Promise<void> {
  const weeks = goalWeeks.value;
  if (!Number.isInteger(weeks) || weeks < 1 || weeks > 104) { message.warning("请输入 1–104 的整周数"); return; }
  goalBusy.value = true;
  try { await api.setLearningGoal(bookId.value, weeks); await load(); goalShow.value = false; message.success(`已设定 ${weeks} 周阅读目标，可在「今日学习」查看每日安排`); }
  catch (e) { message.error(`保存目标失败：${errText(e)}`); }
  finally { goalBusy.value = false; }
}

async function clearGoal(): Promise<void> {
  goalBusy.value = true;
  try { await api.deleteLearningGoal(bookId.value); await load(); goalShow.value = false; message.success("已清除此书的阅读目标；学习记录不变"); }
  catch (e) { message.error(`清除目标失败：${errText(e)}`); }
  finally { goalBusy.value = false; }
}

// ---------- 单小节拆分（后端再次校验历史记录） ----------
const resplitShow = ref(false);
const resplitUnitRow = ref<Unit | null>(null);
const targetChars = ref<number>(1000);
const preview = ref<ResplitPreview | null>(null);
const previewBusy = ref(false);
const resplitBusy = ref(false);
watch(targetChars, () => { preview.value = null; });

function openResplit(unit: Unit): void {
  resplitUnitRow.value = unit;
  preview.value = null;
  resplitShow.value = true;
}

async function loadPreview(): Promise<void> {
  if (!resplitUnitRow.value) return;
  previewBusy.value = true;
  try { preview.value = await api.previewResplit(resplitUnitRow.value.id, targetChars.value); }
  catch (e) { preview.value = null; message.error(`无法预览：${errText(e)}`); }
  finally { previewBusy.value = false; }
}

async function commitResplit(): Promise<void> {
  if (!resplitUnitRow.value || !preview.value) return;
  resplitBusy.value = true;
  try {
    await api.resplitUnit(resplitUnitRow.value.id, targetChars.value);
    resplitShow.value = false;
    message.success("小节已拆分，可立即撤销恢复原边界");
    await load();
  } catch (e) { message.error(`拆分失败：${errText(e)}`); }
  finally { resplitBusy.value = false; }
}

async function undoResplit(unit: Unit): Promise<void> {
  if (!window.confirm(`撤销「${unit.title}」所属的拆分并恢复原小节吗？仅未学习的新小节才可以撤销。`)) return;
  try { await api.undoResplit(unit.id); message.success("已恢复原小节"); await load(); }
  catch (e) { message.error(`撤销失败：${errText(e)}`); }
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

.book-tools {
  margin-top: var(--space-3);
}

.search-results {
  max-height: 60vh;
  overflow-y: auto;
  overscroll-behavior: contain;
}

.search-hit {
  display: block;
  padding: var(--space-3) 0;
  white-space: normal;
  text-align: left;
  line-height: 1.75;
  border-bottom: 1px solid var(--separator);
}

.search-hit:last-child {
  border-bottom: none;
}

.search-hit-title {
  display: block;
  font-weight: 600;
  margin-bottom: var(--space-1);
  color: var(--label);
}

.split-preview {
  margin-top: var(--space-4);
  max-height: 35vh;
  overflow-y: auto;
}

.goal-settings {
  display: inline-block;
  margin-top: var(--space-3);
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
