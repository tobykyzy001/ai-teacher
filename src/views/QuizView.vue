<template>
  <div class="page">
    <div v-if="loading" class="center-box tall">
      <n-spin size="large" />
      <div class="loading-title">AI 正在出题…</div>
      <div class="muted small">首次出题需要模型通读单元并生成题目，可能要等十几到几十秒，请耐心等待</div>
    </div>

    <n-empty v-else-if="error" description="出题失败">
      <div class="muted small empty-extra">{{ error }}</div>
      <n-space>
        <n-button type="primary" @click="loadQuiz">重试</n-button>
        <n-button @click="router.push('/settings')">前往设置</n-button>
      </n-space>
    </n-empty>

    <n-empty v-else-if="questions.length === 0" description="AI 没有为本单元生成题目">
      <n-space>
        <n-button type="primary" @click="loadQuiz">重新出题</n-button>
        <n-button @click="router.push(`/reading/${unitId}`)">返回领读</n-button>
      </n-space>
    </n-empty>

    <template v-else-if="done">
      <div class="page-header">
        <div>
          <h2 class="page-title">测验完成</h2>
          <div v-if="bookTitle" class="page-subtitle">《{{ bookTitle }}》{{ unitTitle }}</div>
        </div>
      </div>
      <n-card>
        <div class="score">
          <span class="score-num">{{ correctCount }}</span>
          <span class="score-sep">/</span>
          <span class="score-total">{{ questions.length }}</span>
          <span class="score-label">题正确</span>
        </div>
        <n-progress
          type="line"
          :percentage="percent(correctCount, questions.length)"
          :height="10"
          :border-radius="5"
        />
        <div class="muted small summary-hint">
          正确率 {{ percent(correctCount, questions.length) }}%{{
            correctCount === questions.length ? "，全对，太棒了！" : "，错题会在复习中再次出现。"
          }}
        </div>
        <n-alert v-if="needReread" class="summary-alert" type="warning" title="建议回本章重读">
          本次错题过半（{{ questions.length - correctCount }}/{{ questions.length }}），建议先回本章把内容再读一遍；错题同时会按计划安排复习。
        </n-alert>
        <div class="summary-actions">
          <n-button
            v-if="needReread"
            type="primary"
            @click="router.push(`/reading/${unitId}`)"
          >
            回本章重读
          </n-button>
          <n-button :loading="regenerating" @click="onRegenerate">换一批新题</n-button>
          <n-button :loading="goingBook" @click="goBookDetail">返回书籍详情</n-button>
          <n-button :type="needReread ? 'default' : 'primary'" @click="router.push('/today')">
            回到今日
          </n-button>
        </div>
      </n-card>
    </template>

    <template v-else>
      <div class="page-header">
        <div>
          <h2 class="page-title">单元闯关</h2>
          <div v-if="bookTitle" class="page-subtitle">《{{ bookTitle }}》{{ unitTitle }}</div>
        </div>
        <div class="header-side">
          <n-popconfirm :disabled="regenerating" @positive-click="onRegenerate">
            <template #trigger>
              <n-button size="small" quaternary :loading="regenerating">换一批新题</n-button>
            </template>
            换一批后这批题的作答进度将丢弃（旧题仍保留在错题本/复习中），确定换吗？
          </n-popconfirm>
          <n-tag size="small" round>第 {{ idx + 1 }} / {{ questions.length }} 题</n-tag>
        </div>
      </div>

      <n-card v-if="current">
        <div class="q-stem">{{ current.stem }}</div>

        <n-radio-group
          v-if="current.qtype === 'mcq'"
          v-model:value="answer"
          class="q-options"
          :disabled="result !== null"
        >
          <n-radio v-for="(opt, i) in current.options" :key="i" :value="opt" :label="opt" />
        </n-radio-group>
        <n-input
          v-else
          v-model:value="answer"
          type="textarea"
          :rows="4"
          placeholder="请输入你的答案…"
          :disabled="result !== null"
        />

        <n-alert
          v-if="result"
          class="q-result"
          :type="result.correct ? 'success' : 'error'"
          :title="result.correct ? '回答正确' : '回答错误'"
        >
          <div class="q-line q-kp-result">
            <n-tag type="info" size="small" :bordered="false">考点：{{ current.knowledge_point }}</n-tag>
          </div>
          <div v-if="result.reference_answer" class="q-line">
            <b>参考答案：</b>{{ result.reference_answer }}
          </div>
          <div v-if="result.explanation" class="q-line q-explanation">{{ result.explanation }}</div>
        </n-alert>

        <div class="q-actions">
          <n-button
            v-if="result === null"
            type="primary"
            :loading="submitting"
            @mousedown="commitComposingInput"
            @click="onSubmit"
          >
            提交
          </n-button>
          <n-button v-else type="primary" @click="onNext">
            {{ isLast ? "查看结果" : "下一题" }}
          </n-button>
        </div>
      </n-card>
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
  NPopconfirm,
  NProgress,
  NRadio,
  NRadioGroup,
  NSpace,
  NSpin,
  NTag,
  useMessage,
} from "naive-ui";
import { api, type GradeResult, type QuizQuestion } from "../api";
import { commitComposingInput, errText, percent } from "../utils";

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

const questions = ref<QuizQuestion[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);

const idx = ref(0);
const answer = ref("");
const submitting = ref(false);
const result = ref<GradeResult | null>(null);
const correctCount = ref(0);
const done = ref(false);
const regenerating = ref(false);

const bookId = ref<number | null>(null);
const bookTitle = ref("");
const unitTitle = ref("");
const goingBook = ref(false);

const current = computed(() => questions.value[idx.value] ?? null);
const isLast = computed(() => idx.value === questions.value.length - 1);
// 错题过半（含一半）→ 结果页给出"回本章重读"入口；全对则无需重看
const needReread = computed(
  () =>
    questions.value.length > 0 &&
    (questions.value.length - correctCount.value) * 2 >= questions.value.length,
);

async function loadUnitInfo(): Promise<void> {
  if (invalidId.value) return;
  try {
    const c = await api.getUnitContent(unitId.value);
    bookId.value = c.unit.book_id;
    bookTitle.value = c.book_title;
    unitTitle.value = c.unit.title;
  } catch {
    // 拿不到书籍信息不阻塞做题，点“返回书籍详情”时会再试一次
  }
}

async function loadQuiz(): Promise<void> {
  if (invalidId.value) {
    error.value = "无效的单元链接";
    return;
  }
  loading.value = true;
  error.value = null;
  try {
    questions.value = await api.generateQuiz(unitId.value);
    idx.value = 0;
    answer.value = "";
    result.value = null;
    correctCount.value = 0;
    done.value = false;
  } catch (e) {
    error.value = errText(e);
    message.error(`出题失败：${error.value}`);
  } finally {
    loading.value = false;
    void notifyThinking();
  }
}

onMounted(() => {
  void loadUnitInfo();
  void loadQuiz();
});

async function onSubmit(): Promise<void> {
  const q = current.value;
  if (!q || submitting.value || result.value !== null) return;
  if (!answer.value.trim()) {
    message.warning("请先输入答案");
    return;
  }
  submitting.value = true;
  try {
    result.value = await api.submitAnswer(q.id, answer.value);
    if (result.value.correct) correctCount.value++;
  } catch (e) {
    message.error(`提交失败：${errText(e)}`);
  } finally {
    submitting.value = false;
    void notifyThinking();
  }
}

function onNext(): void {
  if (result.value === null) return;
  if (isLast.value) {
    done.value = true;
  } else {
    idx.value++;
    answer.value = "";
    result.value = null;
  }
}

// 换一批新题：旧题退役不删除（错题本/复习继续引用），本批作答进度清零重来
async function onRegenerate(): Promise<void> {
  if (regenerating.value || invalidId.value) return;
  regenerating.value = true;
  try {
    questions.value = await api.regenerateQuiz(unitId.value);
    idx.value = 0;
    answer.value = "";
    result.value = null;
    correctCount.value = 0;
    done.value = false;
  } catch (e) {
    message.error(`换一批失败：${errText(e)}`);
  } finally {
    regenerating.value = false;
    void notifyThinking();
  }
}

async function goBookDetail(): Promise<void> {
  if (goingBook.value) return;
  goingBook.value = true;
  try {
    let bid = bookId.value;
    if (bid == null) {
      const c = await api.getUnitContent(unitId.value);
      bid = c.unit.book_id;
      bookId.value = bid;
    }
    router.push(`/books/${bid}`);
  } catch (e) {
    message.error(`无法获取书籍信息：${errText(e)}`);
  } finally {
    goingBook.value = false;
  }
}
</script>

<style scoped>
.center-box.tall {
  padding: 100px 0;
}

.loading-title {
  font-size: var(--text-callout);
  font-weight: 600;
}

.q-kp-result {
  margin-bottom: 8px;
}

.q-stem {
  font-size: var(--text-headline);
  font-weight: 600;
  line-height: 1.7;
  margin-bottom: var(--space-5);
  white-space: pre-wrap;
}

.q-options {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.q-result {
  margin-top: var(--space-5);
}

.q-line {
  white-space: pre-wrap;
}

.q-explanation {
  margin-top: 6px;
}

.q-actions {
  margin-top: var(--space-5);
  display: flex;
  justify-content: flex-end;
  align-items: center;
}

.header-side {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.score {
  text-align: center;
  margin-bottom: var(--space-4);
}

.score-num {
  font-size: var(--text-title2);
  font-weight: 700;
  color: var(--label);
}

.score-sep,
.score-total {
  font-size: var(--text-title3);
  color: var(--label-3);
  margin: 0 var(--space-1);
}

.score-label {
  margin-left: var(--space-2);
  font-size: var(--text-subhead);
  color: var(--label-2);
}

.summary-hint {
  text-align: center;
  margin-top: var(--space-3);
}

.summary-alert {
  margin-top: var(--space-4);
}

.summary-actions {
  display: flex;
  justify-content: center;
  gap: var(--space-3);
  margin-top: var(--space-6);
}
</style>
