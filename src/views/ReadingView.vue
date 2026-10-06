<template>
  <div class="page">
    <div v-if="loading" class="center-box">
      <n-spin size="large" />
    </div>

    <n-empty v-else-if="error" description="无法加载单元内容">
      <div class="muted small empty-extra">{{ error }}</div>
      <n-space>
        <n-button @click="load">重试</n-button>
        <n-button @click="goBack">返回</n-button>
      </n-space>
    </n-empty>

    <template v-else-if="content">
      <div class="page-header">
        <div>
          <h2 class="page-title">{{ content.unit.title }}</h2>
          <div class="page-subtitle">来自《{{ content.book_title }}》· AI 领读</div>
        </div>
      </div>

      <n-card title="原文" class="section">
        <div class="unit-content">{{ content.content }}</div>
      </n-card>

      <n-card title="AI 领读" class="section">
        <div v-if="reading" class="center-box small-pad">
          <n-spin size="medium" />
          <span class="muted">AI 老师正在为你领读…</span>
        </div>
        <n-alert v-else-if="readError" type="error" title="领读失败">
          <div>{{ readError }}</div>
          <n-button size="small" class="retry-btn" @click="startReading">重试</n-button>
        </n-alert>
        <template v-else-if="session">
          <div v-if="session.knowledge_points.length > 0" class="kp-row">
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
        </template>
        <div v-else class="center-box small-pad">
          <span class="muted">让 AI 老师带你通读本单元，划出重点</span>
          <n-button type="primary" @click="startReading">开始领读</n-button>
        </div>
      </n-card>

      <n-card title="答疑" class="section">
        <div class="ask-row">
          <n-input
            v-model:value="question"
            type="textarea"
            :autosize="{ minRows: 1, maxRows: 4 }"
            placeholder="就本单元内容向 AI 老师提问…"
            :disabled="asking"
          />
          <n-button type="primary" :loading="asking" :disabled="!question.trim()" @click="onAsk">
            提问
          </n-button>
        </div>
        <div v-if="asking" class="asking">
          <n-spin size="small" />
          <span class="muted small">AI 老师正在思考…</span>
        </div>
        <div v-if="history.length > 0" class="qa-list">
          <div v-for="(qa, i) in history" :key="i" class="qa-item">
            <div class="qa-q"><span class="qa-tag">问</span>{{ qa.q }}</div>
            <div class="qa-a"><span class="qa-tag ai">答</span><div class="md" v-html="qa.html"></div></div>
          </div>
        </div>
      </n-card>

      <div class="footer-bar">
        <span class="muted small">读完并理解后，通过做题巩固本单元知识</span>
        <n-button type="primary" size="large" @click="router.push(`/quiz/${unitId}`)">
          去闯关做题
        </n-button>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  NAlert,
  NButton,
  NCard,
  NEmpty,
  NInput,
  NSpace,
  NSpin,
  NTag,
  useMessage,
} from "naive-ui";
import { api, type ReadingSession, type UnitContent } from "../api";
import { errText } from "../utils";

interface QA {
  q: string;
  html: string;
}

const route = useRoute();
const router = useRouter();
const message = useMessage();

const unitId = computed(() => Number(route.params.unitId));
const invalidId = computed(() => !Number.isInteger(unitId.value) || unitId.value <= 0);

const content = ref<UnitContent | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);

const session = ref<ReadingSession | null>(null);
const reading = ref(false);
const readError = ref<string | null>(null);

const question = ref("");
const asking = ref(false);
const history = ref<QA[]>([]);

const explanationHtml = computed(() =>
  session.value ? renderMarkdown(session.value.explanation) : "",
);

async function load(): Promise<void> {
  if (invalidId.value) {
    error.value = "无效的单元链接";
    return;
  }
  loading.value = true;
  error.value = null;
  try {
    content.value = await api.getUnitContent(unitId.value);
    if (content.value.unit.status === "unread") {
      void startReading();
    }
  } catch (e) {
    error.value = errText(e);
    message.error(`加载单元失败：${error.value}`);
  } finally {
    loading.value = false;
  }
}

onMounted(load);

async function startReading(): Promise<void> {
  if (reading.value || invalidId.value) return;
  reading.value = true;
  readError.value = null;
  try {
    session.value = await api.startReading(unitId.value);
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
  asking.value = true;
  try {
    const a = await api.ask(unitId.value, q);
    history.value.push({ q, html: renderMarkdown(a) });
    question.value = "";
  } catch (e) {
    message.error(`提问失败：${errText(e)}`);
  } finally {
    asking.value = false;
  }
}

function goBack(): void {
  const c = content.value;
  if (c) router.push(`/books/${c.unit.book_id}`);
  else router.push("/library");
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// 轻量 Markdown 渲染：# / ## / ### 标题、- / * 列表、段落
function renderMarkdown(src: string): string {
  const lines = escapeHtml(src).split(/\r?\n/);
  const out: string[] = [];
  let list: string[] | null = null;
  let para: string[] = [];
  const flushList = (): void => {
    if (list) {
      out.push(`<ul>${list.map((item) => `<li>${item}</li>`).join("")}</ul>`);
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
  for (const raw of lines) {
    const line = raw.trim();
    if (line === "") {
      flushAll();
      continue;
    }
    const heading = line.match(/^(#{1,3})\s+(.+)$/);
    if (heading) {
      flushAll();
      const level = heading[1].length + 2;
      out.push(`<h${level}>${heading[2]}</h${level}>`);
      continue;
    }
    const bullet = line.match(/^[-*]\s+(.+)$/);
    if (bullet) {
      flushPara();
      list = list ?? [];
      list.push(bullet[1]);
      continue;
    }
    flushList();
    para.push(line);
  }
  flushAll();
  return out.join("");
}
</script>

<style scoped>
.section {
  margin-bottom: 20px;
}

.unit-content {
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 18px;
  line-height: 1.9;
  max-height: 62vh;
  overflow-y: auto;
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

.ask-row {
  display: flex;
  align-items: flex-start;
  gap: 12px;
}

.ask-row .n-input {
  flex: 1;
}

.asking {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 12px;
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

.qa-a .md {
  flex: 1;
  min-width: 0;
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
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
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

.md :deep(ul) {
  margin: 8px 0;
  padding-left: 22px;
}

.md :deep(li) {
  margin: 4px 0;
}

.md :deep(:first-child) {
  margin-top: 0;
}

.md :deep(:last-child) {
  margin-bottom: 0;
}
</style>
