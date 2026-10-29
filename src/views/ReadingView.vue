<template>
  <div class="page reading-page" ref="pageEl" :style="{ '--reading-bg': currentTheme.bg, '--reading-text': currentTheme.text, '--reading-heading': currentTheme.heading, '--reading-body-size': `${fontSize}px`, '--reading-line-height': lineHeight }">
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
        <div ref="mainColEl" class="main-col">
          <!-- 手机端：点屏幕左右边翻页（Apple Books 边击翻页；自身不可见） -->
          <div
            v-if="chapter.prev_chapter_unit_id"
            class="page-tap-zone left"
            role="button"
            aria-label="上一章"
            @click="goChapter(chapter.prev_chapter_unit_id)"
          />
          <div
            v-if="chapter.next_chapter_unit_id"
            class="page-tap-zone right"
            role="button"
            aria-label="下一章"
            @click="goChapter(chapter.next_chapter_unit_id)"
          />
          <!-- 翻章箭头：贴正文列左右外缘、钉在视口中线，左右位置由 colRect 实测写入 -->
          <div
            v-if="chapter.prev_chapter_unit_id"
            class="side-arrow left"
            :style="{ left: colRect.left + 'px' }"
            title="上一章（←）"
            role="button"
            aria-label="上一章"
            @click="goChapter(chapter.prev_chapter_unit_id)"
          >
            ‹
          </div>
          <div
            v-if="chapter.next_chapter_unit_id"
            class="side-arrow right"
            :style="{ right: colRect.right + 'px' }"
            title="下一章（→）"
            role="button"
            aria-label="下一章"
            @click="goChapter(chapter.next_chapter_unit_id)"
          >
            ›
          </div>

          <div class="read-col">
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

            <!-- 阅读设置：Apple 图书风（主题 + 字号）；手机默认收起，Aa 圆钮展开 -->
            <button
              class="back-fab"
              title="返回书库"
              aria-label="返回书库"
              @click="goBack"
            >
              ‹
            </button>
            <button
              class="aa-fab"
              title="阅读设置"
              aria-label="阅读设置"
              @click="settingsOpen = !settingsOpen"
            >
              Aa
            </button>
            <div v-show="settingsOpen" class="reading-settings">
              <div class="theme-group" role="group" aria-label="主题切换">
                <button
                  v-for="t in themes"
                  :key="t.key"
                  class="theme-btn"
                  :class="{ active: theme === t.key }"
                  :style="{ background: t.bg, borderColor: t.bg }"
                  :title="t.label"
                  @click="setTheme(t.key)"
                />
              </div>
              <div class="font-controls">
                <n-button size="small" quaternary @click="adjustFontSize(-1)">A−</n-button>
                <span class="font-size-label muted small">{{ fontSize }}px</span>
                <n-button size="small" quaternary @click="adjustFontSize(1)">A＋</n-button>
              </div>
            </div>

            <n-card title="原文" class="section reading-card" :style="{ background: currentTheme.cardBg }">
              <div ref="unitWrap" class="unit-wrap" :style="{ color: currentTheme.text, backgroundColor: currentTheme.cardBg }">
                <template v-for="item in segmentBlocks" :key="item.seg.unit.id">
                  <h4
                    v-if="segmentBlocks.length > 1"
                    :id="`unit-${item.seg.unit.id}`"
                    class="seg-title"
                    :style="{ color: currentTheme.heading }"
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
                    <div
                      v-else-if="block.html"
                      class="unit-content md"
                      :style="{ fontSize: `${fontSize}px`, lineHeight }"
                      v-html="block.html"
                    ></div>
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
                  <n-button class="btn-tinted" size="small" @click="router.push(`/quiz/${seg.unit.id}`)">
                    去闯关做题
                  </n-button>
                </div>
              </div>
            </div>
          </div>
        </div>

        <div v-if="!isNarrow" class="ai-col">
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
              <span class="muted">AI 伴读正在为你领读本章…</span>
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
                <div v-if="reading" class="stream-hint muted small">AI 伴读正在领读本章…</div>
              </template>
              <div v-else-if="!readError" class="center-box small-pad">
                <span class="muted">让 AI 伴读带你通读整章，梳理脉络、划出重点</span>
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
                placeholder="就本章内容向 AI 伴读提问…"
                :disabled="asking"
              />
              <n-button
                type="primary"
                :loading="asking"
                @mousedown="commitComposingInput"
                @click="onAsk"
              >
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
                      <span class="muted small">AI 伴读正在思考…</span>
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

    <!-- 手机端：正文纯净阅读——闯关/AI 收进底部学习助手抽屉（Glass action sheet） -->
    <button v-if="isNarrow" class="ai-fab" title="学习助手" @click="assistantOpen = true">
      AI
    </button>
    <n-drawer
      v-if="isNarrow"
      v-model:show="assistantOpen"
      placement="bottom"
      :height="drawerHeight"
    >
      <n-drawer-content title="学习助手" closable body-content-style="padding: 12px">
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
              <span class="muted">AI 伴读正在为你领读本章…</span>
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
                <div v-if="reading" class="stream-hint muted small">AI 伴读正在领读本章…</div>
              </template>
              <div v-else-if="!readError" class="center-box small-pad">
                <span class="muted">让 AI 伴读带你通读整章，梳理脉络、划出重点</span>
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
                placeholder="就本章内容向 AI 伴读提问…"
                :disabled="asking"
              />
              <n-button
                type="primary"
                :loading="asking"
                @mousedown="commitComposingInput"
                @click="onAsk"
              >
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
                      <span class="muted small">AI 伴读正在思考…</span>
                    </div>
                    <div v-if="qa.error" class="qa-error small">提问失败：{{ qa.error }}</div>
                  </div>
                </div>
              </div>
            </div>
          </n-card>
        </div>
      </n-drawer-content>
    </n-drawer>

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
      @after-enter="forceReaderRepaint"
      @after-leave="resetAskModal"
    >
      <n-alert v-if="quoteWarn" type="warning" :bordered="false" class="quote-warn">
        已选中 {{ askQuote.length }} 字的大段正文。提问时会把这整段原文一起发给 AI，可能产生更多费用。
      </n-alert>
      <div class="quote-block">{{ askQuote }}</div>
      <div v-if="askRounds.length > 0" class="rounds">
        <div v-for="(r, i) in askRounds" :key="i" class="round">
          <div class="round-q">问：{{ r.q }}</div>
          <div v-if="r.html" class="md" v-html="r.html"></div>
          <div v-else-if="!r.error" class="qa-typing">
            <n-spin size="small" />
            <span class="muted small">AI 伴读正在思考…</span>
          </div>
          <div v-if="r.error" class="qa-error small">提问失败：{{ r.error }}</div>
        </div>
      </div>
      <div class="ask-row modal-ask-row">
        <n-input
          v-model:value="askQuestion"
          :placeholder="askRounds.length > 0 ? '就这段内容继续追问…' : '想问 AI 伴读什么？'"
          @keydown.enter="onAskEnter"
        />
        <n-button
          type="primary"
          :loading="askStreaming"
          @mousedown="commitComposingInput"
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
  NDrawer,
  NDrawerContent,
  NEmpty,
  NInput,
  NModal,
  NSpace,
  NSpin,
  NTag,
  useMessage,
} from "naive-ui";
import { api, type ChapterContent, type ChapterSegment, type ReadingSession, type UnitStatus } from "../api";
import { commitComposingInput, errText } from "../utils";

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

// 模型不支持思考参数时，后端会降级并留一条一次性提示，AI 动作结束后取走展示
async function notifyThinking(): Promise<void> {
  const notice = await api.takeThinkingNotice().catch(() => null);
  if (notice) message.warning(notice);
}

const unitId = computed(() => Number(route.params.unitId));
const invalidId = computed(() => !Number.isInteger(unitId.value) || unitId.value <= 0);
const focusUnitId = computed(() => Number(route.query.focus));

const chapter = ref<ChapterContent | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);

// —— 阅读偏好（Apple 图书风主题 + 字号）————
const READING_PREFS_KEY = "ai-teacher:reading-prefs";

interface ThemeDef {
  key: string;
  label: string;
  bg: string;
  cardBg: string;
  text: string;
  heading: string;
}

const themes: ThemeDef[] = [
  { key: "paper", label: "纸白", bg: "#FFFFFF", cardBg: "#FFFFFF", text: "rgba(0,0,0,0.85)", heading: "rgba(0,0,0,0.87)" },
  { key: "sepia", label: "暖纸", bg: "#F5EFDC", cardBg: "#F8F3E4", text: "rgba(87,60,38,0.85)", heading: "rgba(70,48,24,0.87)" },
  { key: "slate", label: "深灰", bg: "#4A4A4C", cardBg: "#3A3A3B", text: "rgba(255,255,255,0.85)", heading: "rgba(255,255,255,0.9)" },
  { key: "night", label: "夜黑", bg: "#1C1C1E", cardBg: "#2C2C2E", text: "rgba(255,255,255,0.80)", heading: "rgba(255,255,255,0.9)" },
];

const theme = ref<string>("paper");
const fontSize = ref<number>(16);
// 阅读设置折叠：桌面常开；手机默认收起，Aa 圆钮展开（Apple Books aa 菜单）
const settingsOpen = ref<boolean>(window.innerWidth > 820);
const lineHeight = 1.75;

function loadPrefs(): { theme?: string; fontSize?: number } {
  try {
    return JSON.parse(localStorage.getItem(READING_PREFS_KEY) || "{}") ?? {};
  } catch {
    return {};
  }
}
applyPrefs(loadPrefs());

function applyPrefs(p: { theme?: string; fontSize?: number }): void {
  if (p.theme && themes.some((t) => t.key === p.theme)) theme.value = p.theme!;
  if (p.fontSize != null) fontSize.value = clampFont(p.fontSize);
}

function clampFont(n: number): number {
  return Math.min(24, Math.max(13, Math.round(n)));
}

function setTheme(key: string): void {
  theme.value = key;
}

function adjustFontSize(delta: number): void {
  fontSize.value = clampFont(fontSize.value + delta);
}

const currentTheme = computed(() => themes.find((t) => t.key === theme.value) ?? themes[0]);

watch([theme, fontSize], ([t, s]) => {
  try {
    localStorage.setItem(READING_PREFS_KEY, JSON.stringify({ theme: t, fontSize: s }));
  } catch {
    /* 本地存储不可用时仅本次运行内记忆 */
  }
});

const session = ref<ReadingSession | null>(null);
const reading = ref(false);
const readError = ref<string | null>(null);
const explainText = ref("");

const question = ref("");
// 手机端结构开关：正文纯净，闯关/AI 收抽屉（≥821px 时保持桌面双列结构）
const isNarrow = ref(window.innerWidth <= 820);
const assistantOpen = ref(false);
function onNarrowResize(): void {
  isNarrow.value = window.innerWidth <= 820;
}
const drawerHeight = computed(() => Math.round(window.innerHeight * 0.72));
const asking = ref(false);
const history = ref<QA[]>([]);

const explanationHtml = computed(() =>
  renderMarkdown(explainText.value || session.value?.explanation || ""),
);

// 翻章箭头贴正文列左右外缘：量正文列矩形，把到窗口左/右边缘的距离写进内联 left/right。
// mainColEl 在 v-if 之后才出现，必须在 ref 落地后再挂 ResizeObserver；窗口滚动条出现/消失时也重算
const mainColEl = ref<HTMLElement | null>(null);
const colRect = ref({ left: 0, right: 0 });
let colObserver: ResizeObserver | null = null;

function measureCol(): void {
  const el = mainColEl.value;
  if (!el) return;
  const r = el.getBoundingClientRect();
  colRect.value = { left: r.left, right: window.innerWidth - r.right };
}

watch(mainColEl, (el) => {
  colObserver?.disconnect();
  colObserver = null;
  if (!el) return;
  colObserver = new ResizeObserver(measureCol);
  colObserver.observe(el);
  colObserver.observe(document.body);
  measureCol();
});

onBeforeUnmount(() => {
  colObserver?.disconnect();
  colObserver = null;
});

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
// 划词时克隆保存的选区位置，问 AI 弹窗关闭后用它把选区恢复成"真选中"状态
const pendingRange = ref<Range | null>(null);

const askShow = ref(false);
const askQuote = ref("");
const askQuestion = ref("");
const askRounds = ref<QA[]>([]);
const askStreaming = ref(false);
// 选中原文会整段随提问发给 LLM；超过这个字数只在弹窗里提示，不阻止提问
const QUOTE_WARN_CHARS = 500;
const quoteWarn = computed(() => askQuote.value.length > QUOTE_WARN_CHARS);

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
  pendingQuote.value = "";
  pendingRange.value = null;
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
  pageEl.value?.addEventListener("wheel", onWheel, { passive: true });
  pageEl.value?.addEventListener("touchstart", onTouchStart, { passive: true });
  pageEl.value?.addEventListener("touchend", onTouchEnd, { passive: true });
  window.addEventListener("scroll", hideSelButton, true);
  window.addEventListener("resize", onNarrowResize, { passive: true });
});

onBeforeUnmount(() => {
  document.removeEventListener("mouseup", onDocMouseup);
  document.removeEventListener("mousedown", onDocMousedown);
  document.removeEventListener("keydown", onKeydown);
  pageEl.value?.removeEventListener("wheel", onWheel);
  pageEl.value?.removeEventListener("touchstart", onTouchStart);
  pageEl.value?.removeEventListener("touchend", onTouchEnd);
  window.removeEventListener("scroll", hideSelButton, true);
  window.removeEventListener("resize", onNarrowResize);
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

// 触控板/数位板双指横滑翻章：滚轮 deltaX 累计 200px 或触屏横向位移 70px 触发，
// 只认以横向为主的滑动（纵向滚动照常），1s 冷却防止一次滑动连翻多章
const SWIPE_WHEEL_THRESHOLD = 200;
const SWIPE_TOUCH_THRESHOLD = 70;
const SWIPE_COOLDOWN_MS = 1000;
let swipeAccX = 0;
let swipeTouchX: number | null = null;
let swipeTouchY: number | null = null;
let swipeLockedUntil = 0;

function onWheel(e: WheelEvent): void {
  if (askShow.value || !chapter.value) return;
  if (Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
  if (e.timeStamp < swipeLockedUntil) return;
  swipeAccX += e.deltaX;
  if (Math.abs(swipeAccX) < SWIPE_WHEEL_THRESHOLD) return;
  const target =
    swipeAccX > 0 ? chapter.value.next_chapter_unit_id : chapter.value.prev_chapter_unit_id;
  swipeAccX = 0;
  if (!target) return;
  swipeLockedUntil = e.timeStamp + SWIPE_COOLDOWN_MS;
  goChapter(target);
}

function onTouchStart(e: TouchEvent): void {
  if (e.touches.length !== 1) return;
  swipeTouchX = e.touches[0].clientX;
  swipeTouchY = e.touches[0].clientY;
}

function onTouchEnd(e: TouchEvent): void {
  if (swipeTouchX === null || swipeTouchY === null || !chapter.value || askShow.value) {
    swipeTouchX = null;
    swipeTouchY = null;
    return;
  }
  const t = e.changedTouches[0];
  const dx = t.clientX - swipeTouchX;
  const dy = t.clientY - swipeTouchY;
  swipeTouchX = null;
  swipeTouchY = null;
  if (Math.abs(dx) < SWIPE_TOUCH_THRESHOLD || Math.abs(dx) <= Math.abs(dy)) return;
  if (Date.now() < swipeLockedUntil) return;
  const target = dx < 0 ? chapter.value.next_chapter_unit_id : chapter.value.prev_chapter_unit_id;
  if (!target) return;
  swipeLockedUntil = Date.now() + SWIPE_COOLDOWN_MS;
  goChapter(target);
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
    void notifyThinking();
  }
}

async function onAsk(): Promise<void> {
  const q = question.value.trim();
  if (!q) {
    message.warning("请先输入问题");
    return;
  }
  if (asking.value || invalidId.value) return;
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
    void notifyThinking();
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
  if (!text) return;
  pendingQuote.value = text;
  pendingRange.value = sel.getRangeAt(0).cloneRange();
  const rect = sel.getRangeAt(0).getBoundingClientRect();
  selPos.value = {
    x: Math.max(8, Math.min(rect.left, window.innerWidth - 104)),
    y: Math.max(8, Math.min(rect.bottom + 8, window.innerHeight - 44)),
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

// WKWebView 偶发把选区变动丢在合成层缓存里不刷新（正文上残留空白高亮残影，或恢复选区不重绘），
// 强制重建正文列的合成层。必须在选区变动落帧之后调用。
function forceReaderRepaint(): void {
  const wrap = unitWrap.value;
  if (!wrap) return;
  wrap.style.transform = "translateZ(0)";
  void wrap.offsetHeight;
  wrap.style.transform = "";
}

function openAskModal(): void {
  if (!pendingQuote.value) return;
  askQuote.value = pendingQuote.value;
  askQuestion.value = "解释这段话";
  askRounds.value = [];
  askShow.value = true;
  selShow.value = false;
  window.getSelection()?.removeAllRanges();
  // 清除选区和弹窗层叠变化会挤在同一个合成事务里，残影此刻就可能留下：
  // 先在下一帧重绘一次，弹窗完全展开后（@after-enter）再重绘一次兜底
  requestAnimationFrame(forceReaderRepaint);
}

function resetAskModal(): void {
  askQuote.value = "";
  askQuestion.value = "";
  askRounds.value = [];
  // 关掉弹窗不等于放弃选区：把划词时保存的 Range 恢复成真实选中，"问 AI"按钮也摆回原位，
  // 想针对同一段再问不用重新划词；只有用户自己点正文空白处（onDocMousedown + 浏览器天然
  // 坍缩选区）才真正清掉。
  const range = pendingRange.value;
  if (!range) return;
  requestAnimationFrame(() => {
    const sel = window.getSelection();
    if (sel) {
      sel.removeAllRanges();
      try {
        sel.addRange(range);
        selShow.value = !sel.isCollapsed;
      } catch {
        // 正文已翻章/重建，旧 Range 失效，放弃恢复
      }
    }
    // WKWebView 偶发把选区变动丢在合成层缓存里不刷新（残留空白高亮或恢复不重绘）
    forceReaderRepaint();
  });
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
    void notifyThinking();
  }
}

function onAskSubmit(): void {
  const q = askQuestion.value.trim();
  if (!q) {
    message.warning("请先输入问题");
    return;
  }
  if (askStreaming.value) return;
  askQuestion.value = "";
  void askRound(q);
}

// 回车提交只认"真正的"回车：输入法候选词确认的回车（isComposing / keyCode 229）不算
function onAskEnter(e: KeyboardEvent): void {
  if (e.isComposing || e.keyCode === 229) return;
  onAskSubmit();
}

function goBack(): void {
  const c = chapter.value;
  if (c && c.segments.length > 0) router.push(`/books/${c.segments[0].unit.book_id}`);
  else router.push("/library");
}

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

/* 正文列只负责滚动，不设内边距——滚动条贴在正文栏右缘 */
.main-col {
  padding: 0;
  /* 两列滚动互不牵连：各自是滚动容器，不让滚到尽头时把滚动手势冒泡给外层 */
  overscroll-behavior: contain;
}

/* 正文内容居中并给左右各留 48px 空白放箭头；滚动条仍在外层 .main-col 的边缘
   限宽随屏宽放阶梯：普通屏 980（原节奏）→ 宽屏放量，杜绝两侧大沙漠 */
.read-col {
  max-width: 980px;
  margin: 0 auto;
  padding: 0 48px 48px;
}

@media (min-width: 1500px) {
  .read-col {
    max-width: 1200px;
  }
}

@media (min-width: 2000px) {
  .read-col {
    max-width: 1440px;
  }
}

/* 翻章箭头：悬浮小圆钮，钉在视口中线（无论正文多长都不随滚动跑），悬停变绿 */
.side-arrow {
  position: fixed;
  top: 50%;
  transform: translateY(-50%);
  z-index: 20;
  width: 32px;
  height: 32px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 22px;
  font-weight: 300;
  line-height: 1;
  /* 液态玻璃圆钮：半透白 + 模糊 + 高光内边，悬停变实并变绿；背景透明时 iOS 原生控件会画成方块，必须给底色 */
  color: #6b7280;
  background: rgba(255, 255, 255, 0.55);
  -webkit-backdrop-filter: blur(20px) saturate(170%);
  backdrop-filter: blur(20px) saturate(170%);
  border: 0.5px solid rgba(255, 255, 255, 0.55);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.7),
    0 1px 6px rgba(0, 0, 0, 0.1);
  outline: none;
  opacity: 1;
  cursor: pointer;
  user-select: none;
  transition:
    background 0.15s,
    color 0.15s;
}

/* 左右位置由脚本按正文列实际矩形写入内联 left，保证始终贴正文列外缘 */

.side-arrow:hover,
.side-arrow:focus-visible {
  color: var(--color-accent);
  background: #fff;
}

/* 阅读页自身是定高的网格单行（外层 .main 与其内部 .n-layout-scroll-container 的高度链见 App.vue）；
   三列容器填满这一行，两列各自内部滚动 */
.page.reading-page {
  display: grid;
  grid-template-rows: minmax(0, 1fr);
  min-height: 0;
  overflow: hidden;
}

.reading-main {
  display: flex;
  flex: 1;
  min-height: 0;
  height: 100%;
  gap: 20px;
  align-items: stretch;
  overflow: hidden;
}

/* 两列都被限制在阅读区高度内（height:100% + 父级定高），内容超出才在自己的滚动条里滚 */

.reading-settings {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
  padding: 10px 12px;
  margin-bottom: 16px;
  background: rgba(255, 255, 255, 0.85);
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  border: 1px solid var(--border-subtle);
  border-radius: 12px;
}

.theme-group {
  display: flex;
  align-items: center;
  gap: 8px;
}

.theme-btn {
  width: 24px;
  height: 24px;
  border-radius: 50%;
  border: 1.5px solid transparent;
  cursor: pointer;
  padding: 0;
  transition: transform 0.12s ease, box-shadow 0.12s ease;
}

.theme-btn:hover {
  transform: scale(1.12);
}

.theme-btn.active {
  box-shadow: 0 0 0 2px var(--text-secondary), 0 0 0 3.5px var(--text-tertiary);
}

.font-controls {
  display: flex;
  align-items: center;
  gap: 4px;
}

.font-size-label {
  min-width: 40px;
  text-align: center;
  color: var(--text-secondary);
}

.main-col {
  flex: 1;
  min-width: 0;
  height: 100%;
  overflow-y: auto;
  /* 常显滚动条：滚动条出现/消失时正文宽度不跳 */
  scrollbar-gutter: stable;
}

.ai-col {
  width: 400px;
  flex-shrink: 0;
  height: 100%;
  overflow-y: auto;
  overscroll-behavior: contain;
  scrollbar-gutter: stable;
  padding-bottom: 4px;
}

/* 窄屏三列放不下：退回单列整页滚动，箭头改为相对页面定位（随内容滚动） */

.reading-card {
  transition: background-color 0.2s ease;
}

/* 暗色主题下 n-card 标题仍用默认黑字——改成跟随阅读主题的标题色 */
.reading-card :deep(.n-card-header__main) {
  color: var(--reading-heading);
}

.reading-card :deep(.n-card__content) {
  background-color: inherit;
}

.unit-wrap * {
  color: inherit;
}

/* 让正文受主题色支配（不再固定死 map 默认） */
.unit-content,
.unit-content.md,
.unit-wrap,
.unit-wrap p,
.unit-wrap li,
.unit-wrap h1,
.unit-wrap h2,
.unit-wrap h3 {
  color: var(--reading-text) !important;
  background: transparent;
}

.seg-title {
  color: var(--reading-heading) !important;
}

.content-img {
  display: block;
  max-width: 100%;
  margin: 14px auto;
  border-radius: 6px;
}

@media (max-width: 1100px) {
  .ai-col {
    width: 320px;
  }
}

@media (max-width: 900px) {
  .reading-main {
    display: block;
  }

  .main-col {
    overflow: visible;
  }

  .read-col {
    padding: 0 16px 24px;
  }

  .ai-col {
    width: 100%;
    overflow: visible;
  }

  .side-arrow {
    position: absolute;
    display: none; /* 手机端隐藏悬浮翻页钮，改点按屏幕左右边翻页 */
  }

  .page-tap-zone {
    display: block;
  }

  .page-tap-zone.left {
    left: 0;
  }

  .page-tap-zone.right {
    right: 0;
  }
}

.page-tap-zone {
  display: none;
  position: fixed;
  top: 0;
  bottom: 0;
  width: 62px;
  z-index: 10;
  cursor: pointer;
}

/* Aa 圆钮 / 返回钮：手机端阅读设置与退出入口（Apple Books 的阅读 chrome） */
.back-fab,
.aa-fab {
  display: none;
  position: absolute;
  top: 0;
  right: 0;
  z-index: 15;
  width: 34px;
  height: 34px;
  border-radius: 50%;
  border: 0.5px solid rgba(255, 255, 255, 0.55);
  background: rgba(255, 255, 255, 0.6);
  -webkit-backdrop-filter: blur(16px) saturate(170%);
  backdrop-filter: blur(16px) saturate(170%);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.7),
    0 2px 8px rgba(0, 0, 0, 0.08);
  color: var(--text-secondary);
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  transition: transform 0.12s ease-out;
}

.back-fab:active,
.aa-fab:active {
  transform: scale(0.94);
}

.back-fab {
  display: none;
  position: absolute;
  top: 0;
  left: 0;
  z-index: 15;
  width: 34px;
  height: 34px;
  border-radius: 50%;
  border: 0.5px solid rgba(255, 255, 255, 0.55);
  background: rgba(255, 255, 255, 0.6);
  -webkit-backdrop-filter: blur(16px) saturate(170%);
  backdrop-filter: blur(16px) saturate(170%);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.7),
    0 2px 8px rgba(0, 0, 0, 0.08);
  color: var(--text-secondary);
  font-size: 24px;
  font-weight: 300;
  line-height: 1;
  cursor: pointer;
  transition: transform 0.12s ease-out;
}

/* 手机阅读页沉浸：只留一行居中小标题，章导航隐去（翻页走两侧箭头/滑动），原文拆卡片框 */
@media (max-width: 820px) {
  .aa-fab,
  .back-fab {
    display: block;
  }

  .read-col {
    position: relative;
    /* 手机端正文解绑：不限宽不居中，全部可用宽度都给内容 */
    max-width: none;
    margin: 0;
    padding: 0 16px 24px;
  }

  .reading-card {
    background: transparent !important;
    border: none !important;
    box-shadow: none !important;
  }

  .reading-card :deep(.n-card-header) {
    display: none;
  }

  .reading-card :deep(.n-card__content) {
    padding: 0 !important;
  }

  .read-col .page-header {
    flex-direction: row;
    justify-content: center;
    margin-bottom: 4px;
  }

  .read-col .page-header > div:first-child {
    text-align: center;
  }

  .read-col .page-title {
    font-size: 13px;
    font-weight: 500;
  }

  .read-col .page-subtitle {
    display: none;
  }

  .read-col .chapter-nav {
    display: none;
  }

  .reading-settings {
    margin-bottom: 8px;
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
  color: var(--color-accent);
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

.quote-warn {
  margin-bottom: 12px;
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
  color: var(--color-accent);
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
  font-size: var(--reading-body-size, 18px);
  line-height: var(--reading-line-height, 1.9);
}

.md.unit-content :deep(p) {
  margin: 12px 0;
}
</style>
