<template>
  <div class="page reading-page" ref="pageEl">
    <div v-if="loading" class="center-box">
      <n-spin size="large" />
    </div>

    <n-empty v-else-if="error" description="无法加载章节内容">
      <div class="muted small empty-extra">{{ error }}</div>
      <n-space>
        <n-button @click="load">重试</n-button>
        <n-button @click="goBack">返回</n-button>
      </n-space>
    </n-empty>

    <template v-else-if="chapter">
      <div class="reading-main">
        <div class="main-col">
          <div class="page-header">
            <div>
              <h2 class="page-title">{{ chapter.chapter_title }}</h2>
              <div class="page-subtitle">
                来自《{{ chapter.book_title }}》· 第 {{ chapter.chapter_number }}/{{
                  chapter.chapter_count
                }}
                章 · AI 领读
              </div>
            </div>
            <div class="chapter-nav">
              <n-button
                size="small"
                :disabled="!chapter.prev_chapter_unit_id"
                @click="goChapter(chapter.prev_chapter_unit_id)"
              >
                ← 上一章
              </n-button>
              <n-button
                size="small"
                :disabled="!chapter.next_chapter_unit_id"
                @click="goChapter(chapter.next_chapter_unit_id)"
              >
                下一章 →
              </n-button>
            </div>
          </div>

          <n-card title="原文" class="section">
            <div ref="unitWrap" class="unit-wrap">
              <template v-for="item in segmentBlocks" :key="item.seg.unit.id">
                <h4
                  v-if="segmentBlocks.length > 1"
                  :id="`unit-${item.seg.unit.id}`"
                  class="seg-title"
                >
                  {{ item.seg.unit.title }}
                </h4>
                <template v-for="(block, bi) in item.blocks" :key="bi">
                  <img
                    v-if="block.img && chapter.images_dir"
                    :src="imgSrc(block.img)"
                    :alt="block.img.alt"
                    class="content-img"
                    @error="onImgError"
                  />
                  <div v-else-if="block.html" class="unit-content md" v-html="block.html"></div>
                </template>
              </template>
            </div>
          </n-card>

          <div class="footer-bar">
            <div class="muted small">读完整章后，逐节闯关巩固本章知识</div>
            <div class="quiz-list">
              <div v-for="seg in chapter.segments" :key="seg.unit.id" class="quiz-row">
                <span class="quiz-title">{{ seg.unit.title }}</span>
                <n-tag size="small" :type="statusType(seg.unit.status)" :bordered="false">
                  {{ statusLabel(seg.unit.status) }}
                </n-tag>
                <n-tag
                  v-if="seg.unit.mastery !== null"
                  size="small"
                  :bordered="false"
                  :type="masteryType(seg.unit.mastery)"
                >
                  掌握度 {{ Math.round((seg.unit.mastery ?? 0) * 100) }}%
                </n-tag>
                <n-button size="small" type="primary" @click="router.push(`/quiz/${seg.unit.id}`)">
                  去闯关做题
                </n-button>
              </div>
            </div>
          </div>
        </div>

        <div class="ai-col">
          <n-card title="AI 领读" class="section">
            <template #header-extra>
              <n-button
                v-if="(session || explainText) && !reading"
                text
                size="small"
                @click="startReading"
              >
                重新领读
              </n-button>
            </template>
            <div v-if="reading && !explainText" class="center-box small-pad">
              <n-spin size="medium" />
              <span class="muted">AI 老师正在为你领读本章…</span>
            </div>
            <template v-else>
              <n-alert v-if="readError" type="error" title="领读失败">
                <div>{{ readError }}</div>
                <n-button size="small" class="retry-btn" @click="startReading">重试</n-button>
              </n-alert>
              <template v-if="session || explainText">
                <div v-if="session && session.knowledge_points.length > 0" class="kp-row">
                  <n-tag
                    v-for="kp in session.knowledge_points"
                    :key="kp"
                    type="info"
                    size="small"
                    round
                    :bordered="false"
                  >
                    {{ kp }}
                  </n-tag>
                </div>
                <div class="md" v-html="explanationHtml"></div>
                <div v-if="reading" class="stream-hint muted small">AI 老师正在领读本章…</div>
              </template>
              <div v-else-if="!readError" class="center-box small-pad">
                <span class="muted">让 AI 老师带你通读整章，梳理脉络、划出重点</span>
                <n-button type="primary" @click="startReading">开始领读</n-button>
              </div>
            </template>
          </n-card>

          <n-card title="答疑" class="section">
            <div class="ask-row">
              <n-input
                v-model:value="question"
                type="textarea"
                :autosize="{ minRows: 1, maxRows: 4 }"
                placeholder="就本章内容向 AI 老师提问…"
                :disabled="asking"
              />
              <n-button type="primary" :loading="asking" :disabled="!question.trim()" @click="onAsk">
                提问
              </n-button>
            </div>
            <div v-if="history.length > 0" class="qa-list">
              <div v-for="(qa, i) in history" :key="i" class="qa-item">
                <div class="qa-q"><span class="qa-tag">问</span>{{ qa.q }}</div>
                <div class="qa-a">
                  <span class="qa-tag ai">答</span>
                  <div class="qa-a-body">
                    <div v-if="qa.html" class="md" v-html="qa.html"></div>
                    <div v-else-if="!qa.error" class="qa-typing">
                      <n-spin size="small" />
                      <span class="muted small">AI 老师正在思考…</span>
                    </div>
                    <div v-if="qa.error" class="qa-error small">提问失败：{{ qa.error }}</div>
                  </div>
                </div>
              </div>
            </div>
          </n-card>
        </div>
      </div>
    </template>

    <div
      v-if="selShow"
      ref="selBtnEl"
      class="sel-btn"
      :style="{ left: selPos.x + 'px', top: selPos.y + 'px' }"
    >
      <n-button size="small" type="primary" @click="openAskModal">问 AI</n-button>
    </div>

    <n-modal
      v-model:show="askShow"
      preset="card"
      title="问 AI"
      :style="{ width: '560px', maxWidth: '92vw' }"
      :mask-closable="!askStreaming"
      :close-on-esc="!askStreaming"
      @after-leave="resetAskModal"
    >
      <div class="quote-block">{{ askQuote }}</div>
      <div v-if="askRounds.length > 0" class="rounds">
        <div v-for="(r, i) in askRounds" :key="i" class="round">
          <div class="round-q">问：{{ r.q }}</div>
          <div v-if="r.html" class="md" v-html="r.html"></div>
          <div v-else-if="!r.error" class="qa-typing">
            <n-spin size="small" />
            <span class="muted small">AI 老师正在思考…</span>
          </div>
          <div v-if="r.error" class="qa-error small">提问失败：{{ r.error }}</div>
        </div>
      </div>
      <div class="ask-row modal-ask-row">
        <n-input
          v-model:value="askQuestion"
          :placeholder="askRounds.length > 0 ? '就这段内容继续追问…' : '想问 AI 老师什么？'"
          @keyup.enter="onAskSubmit"
        />
        <n-button
          type="primary"
          :loading="askStreaming"
          :disabled="!askQuestion.trim() || askStreaming"
          @click="onAskSubmit"
        >
          {{ askRounds.length > 0 ? "追问" : "提问" }}
        </n-button>
      </div>
    </n-modal>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  NAlert,
  NButton,
  NCard,
  NEmpty,
  NInput,
  NModal,
  NSpace,
  NSpin,
  NTag,
  useMessage,
} from "naive-ui";
import { api, type ChapterContent, type ChapterSegment, type ReadingSession, type UnitStatus } from "../api";
import { errText } from "../utils";

interface QA {
  q: string;
  rawText: string;
  html: string;
  error: string | null;
}

interface ContentBlock {
  img?: { alt: string; name: string };
  text?: string;
  html?: string;
}

interface SegmentBlocks {
  seg: ChapterSegment;
  blocks: ContentBlock[];
}

type TagType = "default" | "primary" | "info" | "success" | "warning" | "error";

const route = useRoute();
const router = useRouter();
const message = useMessage();

const unitId = computed(() => Number(route.params.unitId));
const invalidId = computed(() => !Number.isInteger(unitId.value) || unitId.value <= 0);
const focusUnitId = computed(() => Number(route.query.focus));

const chapter = ref<ChapterContent | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);

const session = ref<ReadingSession | null>(null);
const reading = ref(false);
const readError = ref<string | null>(null);
const explainText = ref("");

const question = ref("");
const asking = ref(false);
const history = ref<QA[]>([]);

const explanationHtml = computed(() =>
  renderMarkdown(explainText.value || session.value?.explanation || ""),
);

// 各小节的正文块（文本/图片）：EPUB 导入时插图被抽成 `![alt](文件名)` 标记段，这里原位还原
const segmentBlocks = computed<SegmentBlocks[]>(
  () =>
    chapter.value?.segments.map((seg) => ({
      seg,
      blocks: contentBlocks(seg.content),
    })) ?? [],
);

function contentBlocks(content: string): ContentBlock[] {
  const blocks: ContentBlock[] = [];
  let buf: string[] = [];
  const flush = (): void => {
    const text = buf.join("\n");
    if (text.trim()) blocks.push({ text, html: renderMarkdown(text) });
    buf = [];
  };
  for (const line of content.split("\n")) {
    const m = line.trim().match(/^!\[([^\]]*)\]\(([^)\s]+)\)$/);
    if (m) {
      flush();
      blocks.push({ img: { alt: m[1], name: m[2] } });
    } else {
      buf.push(line);
    }
  }
  flush();
  return blocks;
}

function imgSrc(img: { name: string }): string {
  const dir = chapter.value?.images_dir;
  return dir ? convertFileSrc(`${dir}/${img.name}`) : "";
}

function onImgError(e: Event): void {
  (e.target as HTMLElement).style.display = "none";
}

// 划词提问状态
const pageEl = ref<HTMLElement | null>(null);
const unitWrap = ref<HTMLElement | null>(null);
const selBtnEl = ref<HTMLElement | null>(null);
const selShow = ref(false);
const selPos = ref({ x: 0, y: 0 });
const pendingQuote = ref("");

const askShow = ref(false);
const askQuote = ref("");
const askQuestion = ref("");
const askRounds = ref<QA[]>([]);
const askStreaming = ref(false);

async function load(): Promise<void> {
  if (invalidId.value) {
    error.value = "无效的单元链接";
    return;
  }
  loading.value = true;
  error.value = null;
  try {
    chapter.value = await api.getChapterContent(unitId.value);
    await scrollAfterLoad();
    // 已有保存的讲解直接展示（不重新生成）；全新章仅在设置开启时自动领读
    if (chapter.value.saved_session) {
      session.value = chapter.value.saved_session;
    } else if (chapter.value.segments.every((s) => s.unit.status === "unread")) {
      const st = await api.getSettings().catch(() => null);
      if (st?.auto_start_reading) void startReading();
    }
  } catch (e) {
    error.value = errText(e);
    message.error(`加载章节失败：${error.value}`);
  } finally {
    loading.value = false;
  }
}

function resetState(): void {
  chapter.value = null;
  session.value = null;
  explainText.value = "";
  readError.value = null;
  question.value = "";
  history.value = [];
  selShow.value = false;
  askShow.value = false;
  askQuote.value = "";
  askQuestion.value = "";
  askRounds.value = [];
}

// 路由 unitId 变化（章间翻页、从详情页跳入）→ 重置并整章重载
watch(unitId, (id, prev) => {
  if (id === prev) return;
  resetState();
  void load();
});

onMounted(() => {
  void load();
  document.addEventListener("mouseup", onDocMouseup);
  document.addEventListener("mousedown", onDocMousedown);
  document.addEventListener("keydown", onKeydown);
  window.addEventListener("scroll", hideSelButton, true);
});

onBeforeUnmount(() => {
  document.removeEventListener("mouseup", onDocMouseup);
  document.removeEventListener("mousedown", onDocMousedown);
  document.removeEventListener("keydown", onKeydown);
  window.removeEventListener("scroll", hideSelButton, true);
});

async function scrollAfterLoad(): Promise<void> {
  await nextTick();
  if (Number.isInteger(focusUnitId.value) && focusUnitId.value > 0) {
    document.getElementById(`unit-${focusUnitId.value}`)?.scrollIntoView({ block: "start" });
  } else {
    pageEl.value?.scrollIntoView({ block: "start" });
  }
}

function goChapter(targetUnitId: number | null): void {
  if (!targetUnitId) return;
  void router.push(`/reading/${targetUnitId}`);
}

function onKeydown(e: KeyboardEvent): void {
  const target = e.target as HTMLElement | null;
  if (
    target &&
    (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
  ) {
    return;
  }
  if (askShow.value || !chapter.value) return;
  if (e.key === "ArrowLeft") goChapter(chapter.value.prev_chapter_unit_id);
  else if (e.key === "ArrowRight") goChapter(chapter.value.next_chapter_unit_id);
}

async function startReading(): Promise<void> {
  if (reading.value || invalidId.value) return;
  reading.value = true;
  readError.value = null;
  explainText.value = "";
  try {
    session.value = await api.startChapterReading(unitId.value, (s) => {
      explainText.value += s;
    });
    explainText.value = session.value.explanation;
    // 章领读会把章内未读小节登记为"在读"，本地同步状态
    chapter.value?.segments.forEach((s) => {
      if (s.unit.status === "unread") s.unit.status = "reading";
    });
  } catch (e) {
    readError.value = errText(e);
    message.error(`领读失败：${readError.value}`);
  } finally {
    reading.value = false;
  }
}

async function onAsk(): Promise<void> {
  const q = question.value.trim();
  if (!q || asking.value || invalidId.value) return;
  const entry = reactive<QA>({ q, rawText: "", html: "", error: null });
  history.value.push(entry);
  question.value = "";
  asking.value = true;
  try {
    const a = await api.askChapter(unitId.value, q, (s) => {
      entry.rawText += s;
      entry.html = renderMarkdown(entry.rawText);
    });
    entry.rawText = a;
    entry.html = renderMarkdown(a);
  } catch (e) {
    entry.error = errText(e);
    message.error(`提问失败：${entry.error}`);
  } finally {
    asking.value = false;
  }
}

function onDocMouseup(e: MouseEvent): void {
  if (selBtnEl.value?.contains(e.target as Node)) return;
  selShow.value = false;
  const wrap = unitWrap.value;
  if (!wrap) return;
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return;
  if (
    !sel.anchorNode ||
    !sel.focusNode ||
    !wrap.contains(sel.anchorNode) ||
    !wrap.contains(sel.focusNode)
  ) {
    return;
  }
  const text = sel.toString().trim();
  if (text.length < 1 || text.length > 500) return;
  pendingQuote.value = text;
  const rect = sel.getRangeAt(0).getBoundingClientRect();
  selPos.value = {
    x: Math.max(8, Math.min(rect.left, window.innerWidth - 104)),
    y: Math.min(rect.bottom + 8, window.innerHeight - 44),
  };
  selShow.value = true;
}

function onDocMousedown(e: MouseEvent): void {
  if (selBtnEl.value?.contains(e.target as Node)) return;
  selShow.value = false;
}

function hideSelButton(): void {
  selShow.value = false;
}

function openAskModal(): void {
  if (!pendingQuote.value) return;
  askQuote.value = pendingQuote.value;
  askQuestion.value = "解释这段话";
  askRounds.value = [];
  askShow.value = true;
  selShow.value = false;
  window.getSelection()?.removeAllRanges();
}

function resetAskModal(): void {
  askQuote.value = "";
  askQuestion.value = "";
  askRounds.value = [];
  selShow.value = false;
}

async function askRound(q: string): Promise<void> {
  if (askStreaming.value || invalidId.value) return;
  const round = reactive<QA>({ q, rawText: "", html: "", error: null });
  askRounds.value.push(round);
  askStreaming.value = true;
  try {
    const a = await api.askChapterSelection(unitId.value, askQuote.value, q, (s) => {
      round.rawText += s;
      round.html = renderMarkdown(round.rawText);
    });
    round.rawText = a;
    round.html = renderMarkdown(a);
  } catch (e) {
    round.error = errText(e);
    message.error(`提问失败：${round.error}`);
  } finally {
    askStreaming.value = false;
  }
}

function onAskSubmit(): void {
  const q = askQuestion.value.trim();
  if (!q || askStreaming.value) return;
  askQuestion.value = "";
  void askRound(q);
}

function goBack(): void {
  const c = chapter.value;
  if (c && c.segments.length > 0) router.push(`/books/${c.segments[0].unit.book_id}`);
  else router.push("/library");
}

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

function masteryType(mastery: number | null): TagType {
  return mastery !== null && mastery >= 0.8 ? "success" : "warning";
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// 轻量 Markdown 渲染：标题、有序/无序列表、引用、代码块、表格、段落 + 行内粗体/斜体/代码/删除线/链接
// 输入先整体转义，行内格式在转义后处理，因此对部分（未完整）文本重复渲染也是安全的
function renderInline(escaped: string): string {
  let out = escaped;
  out = out.replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>");
  out = out.replace(/\*([^*\n]+)\*/g, "<em>$1</em>");
  out = out.replace(/~~([^~\n]+)~~/g, "<del>$1</del>");
  out = out.replace(/`([^`\n]+)`/g, "<code>$1</code>");
  out = out.replace(
    /\[([^\]\n]+)\]\((https?:\/\/[^)\s]+)\)/g,
    '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>',
  );
  return out;
}

// 表格行切成单元格（去掉首尾空列）
function splitTableRow(line: string): string[] {
  let row = line;
  if (row.startsWith("|")) row = row.slice(1);
  if (row.endsWith("|")) row = row.slice(0, -1);
  return row.split("|").map((c) => c.trim());
}

// |---|---| 这类分隔行：只含 |、-、:、空格且至少一个 |
function isTableSeparator(line: string): boolean {
  return line.includes("|") && /^[\s|:-]+$/.test(line) && line.includes("-");
}

function renderMarkdown(src: string): string {
  const lines = escapeHtml(src).split(/\r?\n/);
  const out: string[] = [];
  let list: { type: "ul" | "ol"; items: string[] } | null = null;
  let para: string[] = [];
  let code: string[] | null = null;
  const flushList = (): void => {
    if (list) {
      const items = list.items.map((item) => `<li>${item}</li>`).join("");
      out.push(`<${list.type}>${items}</${list.type}>`);
      list = null;
    }
  };
  const flushPara = (): void => {
    if (para.length > 0) {
      out.push(`<p>${para.join("\n")}</p>`);
      para = [];
    }
  };
  const flushAll = (): void => {
    flushList();
    flushPara();
  };
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const line = raw.trim();
    if (line.startsWith("```")) {
      if (code) {
        out.push(`<pre><code>${code.join("\n")}</code></pre>`);
        code = null;
      } else {
        flushAll();
        code = [];
      }
      continue;
    }
    if (code) {
      code.push(raw);
      continue;
    }
    if (line === "") {
      flushAll();
      continue;
    }
    if (line.includes("|") && i + 1 < lines.length && isTableSeparator(lines[i + 1].trim())) {
      flushAll();
      const header = splitTableRow(line);
      i += 1;
      const rows: string[][] = [];
      while (i + 1 < lines.length) {
        const next = lines[i + 1].trim();
        if (next === "" || !next.startsWith("|")) break;
        rows.push(splitTableRow(next));
        i += 1;
      }
      const thead = `<thead><tr>${header.map((c) => `<th>${renderInline(c)}</th>`).join("")}</tr></thead>`;
      const tbody = rows
        .map((r) => `<tr>${r.map((c) => `<td>${renderInline(c)}</td>`).join("")}</tr>`)
        .join("");
      out.push(`<table>${thead}${tbody.length > 0 ? `<tbody>${tbody}</tbody>` : ""}</table>`);
      continue;
    }
    const heading = line.match(/^(#{1,4})\s+(.+)$/);
    if (heading) {
      flushAll();
      const level = Math.min(heading[1].length + 2, 5);
      out.push(`<h${level}>${renderInline(heading[2])}</h${level}>`);
      continue;
    }
    // 输入已整体转义，引用符在这里是 &gt;
    const quote = line.match(/^&gt;\s*(.+)$/);
    if (quote) {
      flushAll();
      out.push(`<blockquote>${renderInline(quote[1])}</blockquote>`);
      continue;
    }
    const bullet = line.match(/^[-*]\s+(.+)$/);
    if (bullet) {
      flushPara();
      if (list?.type !== "ul") {
        flushList();
        list = { type: "ul", items: [] };
      }
      list.items.push(renderInline(bullet[1]));
      continue;
    }
    const ordered = line.match(/^\d+\s*[.、)]\s*(.+)$/);
    if (ordered) {
      flushPara();
      if (list?.type !== "ol") {
        flushList();
        list = { type: "ol", items: [] };
      }
      list.items.push(renderInline(ordered[1]));
      continue;
    }
    flushList();
    para.push(renderInline(line));
  }
  if (code) out.push(`<pre><code>${code.join("\n")}</code></pre>`);
  flushAll();
  return out.join("");
}
</script>

<style scoped>
.page.reading-page {
  max-width: none;
}

.reading-main {
  display: flex;
  gap: 20px;
  align-items: flex-start;
}

.main-col {
  flex: 1;
  min-width: 0;
}

.ai-col {
  width: 400px;
  flex-shrink: 0;
  position: sticky;
  top: 0;
  max-height: calc(100vh - 40px);
  overflow-y: auto;
  padding-bottom: 4px;
}

.content-img {
  display: block;
  max-width: 100%;
  margin: 14px auto;
  border-radius: 6px;
}

@media (max-width: 900px) {
  .reading-main {
    flex-direction: column;
  }

  .ai-col {
    width: 100%;
    max-height: none;
    position: static;
  }
}

.section {
  margin-bottom: 20px;
}

.chapter-nav {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

.unit-wrap {
  position: relative;
}

.seg-title {
  margin: 28px 0 10px;
  font-size: 17px;
  font-weight: 600;
  scroll-margin-top: 16px;
}

.seg-title:first-child {
  margin-top: 0;
}

.unit-content {
  word-break: break-word;
  padding-right: 6px;
}

.kp-row {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 12px;
}

.retry-btn {
  margin-top: 8px;
}

.stream-hint {
  margin-top: 10px;
}

.ask-row {
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

.ask-row .n-input {
  flex: 1;
}

.qa-list {
  margin-top: 8px;
}

.qa-item {
  padding: 12px 0;
  border-bottom: 1px dashed #efeff5;
}

.qa-item:last-child {
  border-bottom: none;
}

.qa-q,
.qa-a {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}

.qa-q {
  font-weight: 500;
  margin-bottom: 8px;
}

.qa-a-body {
  flex: 1;
  min-width: 0;
}

.qa-typing {
  display: flex;
  align-items: center;
  gap: 8px;
}

.qa-error {
  color: #d03050;
  white-space: pre-wrap;
}

.qa-tag {
  flex-shrink: 0;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  font-size: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #eef0f3;
  color: rgba(0, 0, 0, 0.6);
}

.qa-tag.ai {
  background: #e0f2e9;
  color: #18a058;
}

.footer-bar {
  margin-top: 4px;
  padding: 16px 20px;
  background: #fafafc;
  border: 1px solid #efeff5;
  border-radius: 8px;
}

.quiz-list {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
}

.quiz-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 0;
}

.quiz-row + .quiz-row {
  border-top: 1px dashed #efeff5;
}

.quiz-title {
  flex: 1;
  min-width: 0;
  font-size: 14px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sel-btn {
  position: fixed;
  z-index: 1000;
}

.quote-block {
  background: #f7f8fa;
  border: 1px solid #efeff5;
  border-radius: 4px;
  padding: 10px 12px;
  max-height: 140px;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 13px;
  line-height: 1.7;
  color: rgba(0, 0, 0, 0.55);
  margin-bottom: 16px;
}

.rounds {
  max-height: 44vh;
  overflow-y: auto;
  margin-bottom: 12px;
}

.round {
  padding: 10px 0;
  border-bottom: 1px dashed #efeff5;
}

.round:last-child {
  border-bottom: none;
}

.round-q {
  font-weight: 500;
  margin-bottom: 6px;
}

.modal-ask-row {
  margin-top: 4px;
}

.md {
  font-size: 15px;
  line-height: 1.8;
  word-break: break-word;
}

.md :deep(h3) {
  font-size: 17px;
  margin: 18px 0 8px;
}

.md :deep(h4) {
  font-size: 16px;
  margin: 14px 0 6px;
}

.md :deep(h5) {
  font-size: 15px;
  margin: 12px 0 6px;
}

.md :deep(p) {
  margin: 8px 0;
  white-space: pre-wrap;
}

.md :deep(ul),
.md :deep(ol) {
  margin: 8px 0;
  padding-left: 22px;
}

.md :deep(li) {
  margin: 4px 0;
}

.md :deep(strong) {
  font-weight: 650;
}

.md :deep(code) {
  background: #f2f3f5;
  padding: 1px 6px;
  border-radius: 4px;
  font-size: 0.92em;
}

.md :deep(blockquote) {
  margin: 8px 0;
  padding: 2px 12px;
  border-left: 3px solid #dfe2e8;
  color: rgba(0, 0, 0, 0.62);
}

.md :deep(pre) {
  margin: 10px 0;
  padding: 10px 12px;
  background: #f2f3f5;
  border: 1px solid #e8eaf0;
  border-radius: 6px;
  overflow-x: auto;
  white-space: pre;
}

.md :deep(pre code) {
  background: none;
  padding: 0;
  font-size: 13px;
  line-height: 1.6;
}

.md :deep(table) {
  border-collapse: collapse;
  margin: 10px 0;
  width: 100%;
  font-size: 14px;
}

.md :deep(th),
.md :deep(td) {
  border: 1px solid #e4e7ed;
  padding: 6px 10px;
  text-align: left;
}

.md :deep(th) {
  background: #f7f8fa;
  font-weight: 600;
}

.md :deep(a) {
  color: #18a058;
  text-decoration: none;
}

.md :deep(a:hover) {
  text-decoration: underline;
}

.md :deep(:first-child) {
  margin-top: 0;
}

.md :deep(:last-child) {
  margin-bottom: 0;
}

.md.unit-content {
  font-size: 18px;
  line-height: 1.9;
}

.md.unit-content :deep(p) {
  margin: 12px 0;
}
</style>
