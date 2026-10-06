<template>
  <div class="page">
    <div class="page-header">
      <div>
        <h2 class="page-title">错题本</h2>
        <div class="page-subtitle">集中重练答错的题目，直到全部解决</div>
      </div>
      <n-button quaternary size="small" @click="reload">刷新</n-button>
    </div>

    <n-empty
      v-if="library.loaded && library.books.length === 0"
      description="书架还是空的，先去导入一本书吧。"
    >
      <n-button type="primary" @click="router.push('/library')">去书架</n-button>
    </n-empty>

    <template v-else>
      <div class="filter-bar">
        <n-select
          v-model:value="bookFilter"
          class="filter-select"
          :options="bookOptions"
          placeholder="全部书籍"
          clearable
          @update:value="onBookChange"
        />
        <n-select
          v-model:value="unitFilter"
          class="filter-select"
          :options="unitOptions"
          placeholder="全部单元"
          clearable
          :disabled="unitOptions.length === 0"
        />
        <n-checkbox v-model:checked="unresolvedOnly">仅看未解决</n-checkbox>
      </div>

      <div v-if="loading" class="center-box">
        <n-spin size="large" />
      </div>

      <n-empty v-else-if="error" description="加载错题列表失败">
        <div class="muted small empty-extra">{{ error }}</div>
        <n-button @click="reload">重试</n-button>
      </n-empty>

      <n-empty v-else-if="all.length === 0" description="还没有错题记录，继续保持！">
        <div class="muted small empty-extra">做题答错的题目会自动收进错题本</div>
      </n-empty>

      <n-empty v-else-if="displayed.length === 0" description="没有符合筛选条件的错题" />

      <n-list v-else bordered>
        <n-list-item v-for="w in displayed" :key="w.question_id">
          <div class="row">
            <div class="row-main">
              <div class="stem">{{ w.stem }}</div>
              <div class="meta">
                <n-tag type="info" size="small" :bordered="false">{{ w.knowledge_point }}</n-tag>
                <n-tag v-if="w.resolved" type="success" size="small" :bordered="false">
                  已解决
                </n-tag>
                <span class="muted small">错 {{ w.wrong_count }} 次</span>
                <span class="muted small">最近答错 {{ fmtDate(w.last_wrong_at) }}</span>
              </div>
              <div class="muted small source">《{{ w.book_title }}》{{ w.unit_title }}</div>
            </div>
            <div class="row-side">
              <n-button size="small" type="primary" @click="openPractice(w)">重练</n-button>
            </div>
          </div>
        </n-list-item>
      </n-list>
    </template>

    <n-modal
      v-model:show="practiceShow"
      preset="card"
      title="重练错题"
      :style="{ width: '560px', maxWidth: '92vw' }"
      :mask-closable="!practiceBusy"
      :close-on-esc="!practiceBusy"
    >
      <div v-if="practiceLoading" class="center-box small-pad">
        <n-spin size="medium" />
        <span class="muted">正在加载题目…</span>
      </div>

      <template v-else-if="practiceError">
        <n-alert type="error" title="加载题目失败">{{ practiceError }}</n-alert>
        <div class="practice-actions">
          <n-button size="small" @click="retryPractice">重试</n-button>
          <n-button size="small" quaternary @click="practiceShow = false">关闭</n-button>
        </div>
      </template>

      <template v-else-if="practiceQ">
        <div class="kp-line">
          <n-tag type="info" size="small" :bordered="false">
            考点：{{ practiceQ.knowledge_point }}
          </n-tag>
        </div>
        <div class="q-stem">{{ practiceQ.stem }}</div>
        <n-radio-group
          v-if="practiceQ.qtype === 'mcq'"
          v-model:value="practiceAnswer"
          class="q-options"
          :disabled="grading"
        >
          <n-radio v-for="(opt, i) in practiceQ.options" :key="i" :value="opt" :label="opt" />
        </n-radio-group>
        <n-input
          v-else
          v-model:value="practiceAnswer"
          type="textarea"
          :rows="3"
          placeholder="请输入你的答案…"
          :disabled="grading"
        />

        <n-alert
          v-if="practiceResult"
          class="q-result"
          :type="practiceResult.correct ? 'success' : 'error'"
          :title="practiceResult.correct ? '回答正确' : '回答错误'"
        >
          <div v-if="practiceResult.reference_answer" class="q-line">
            <b>参考答案：</b>{{ practiceResult.reference_answer }}
          </div>
          <div v-if="practiceResult.explanation" class="q-line q-explanation">
            {{ practiceResult.explanation }}
          </div>
        </n-alert>

        <div class="practice-actions">
          <n-button
            v-if="!practiceResult || !practiceResult.correct"
            type="primary"
            :loading="grading"
            :disabled="!practiceAnswer.trim()"
            @click="onPracticeSubmit"
          >
            提交答案
          </n-button>
          <n-button v-if="practiceResult" @click="practiceShow = false">关闭</n-button>
        </div>
      </template>
    </n-modal>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import {
  NAlert,
  NButton,
  NCheckbox,
  NEmpty,
  NInput,
  NList,
  NListItem,
  NModal,
  NRadio,
  NRadioGroup,
  NSelect,
  NSpin,
  NTag,
  useMessage,
} from "naive-ui";
import { api, type GradeResult, type QuizQuestion, type WrongQuestion } from "../api";
import { errText, fmtDate } from "../utils";
import { useLibraryStore } from "../stores/library";

const router = useRouter();
const message = useMessage();
const library = useLibraryStore();

const all = ref<WrongQuestion[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);

const bookFilter = ref<number | null>(null);
const unitFilter = ref<number | null>(null);
const unresolvedOnly = ref(true);

const bookOptions = computed(() => library.books.map((b) => ({ label: b.title, value: b.id })));

// 单元选项来自当前已加载的错题列表（列表本身已按书过滤）
const unitOptions = computed(() => {
  const seen = new Map<number, string>();
  for (const w of all.value) {
    if (!seen.has(w.unit_id)) seen.set(w.unit_id, w.unit_title);
  }
  return Array.from(seen, ([value, label]) => ({ label, value }));
});

const displayed = computed(() =>
  all.value.filter(
    (w) =>
      (unitFilter.value === null || w.unit_id === unitFilter.value) &&
      (!unresolvedOnly.value || !w.resolved),
  ),
);

async function reload(): Promise<void> {
  loading.value = true;
  error.value = null;
  try {
    all.value = await api.listWrongQuestions({ bookId: bookFilter.value });
  } catch (e) {
    error.value = errText(e);
    message.error(`加载错题列表失败：${error.value}`);
  } finally {
    loading.value = false;
  }
}

function onBookChange(): void {
  unitFilter.value = null;
  void reload();
}

onMounted(async () => {
  await library.load().catch(() => undefined);
  void reload();
});

// ---- 重练弹窗 ----

const practiceShow = ref(false);
const practiceLoading = ref(false);
const practiceError = ref<string | null>(null);
const practiceQ = ref<QuizQuestion | null>(null);
const currentQuestionId = ref<number | null>(null);
const practiceAnswer = ref("");
const grading = ref(false);
const practiceResult = ref<GradeResult | null>(null);
const attempted = ref(false);

const practiceBusy = computed(() => practiceLoading.value || grading.value);

function openPractice(w: WrongQuestion): void {
  currentQuestionId.value = w.question_id;
  practiceShow.value = true;
  void loadPracticeQuestion(w.question_id);
}

async function loadPracticeQuestion(questionId: number): Promise<void> {
  practiceQ.value = null;
  practiceResult.value = null;
  practiceAnswer.value = "";
  practiceError.value = null;
  practiceLoading.value = true;
  try {
    practiceQ.value = await api.getQuestion(questionId);
  } catch (e) {
    practiceError.value = errText(e);
  } finally {
    practiceLoading.value = false;
  }
}

function retryPractice(): void {
  if (currentQuestionId.value !== null) void loadPracticeQuestion(currentQuestionId.value);
}

async function onPracticeSubmit(): Promise<void> {
  const q = practiceQ.value;
  if (!q || grading.value || !practiceAnswer.value.trim()) return;
  grading.value = true;
  try {
    practiceResult.value = await api.submitAnswer(q.id, practiceAnswer.value);
    attempted.value = true;
    if (practiceResult.value.correct) {
      message.success("回答正确，该错题已标记为已解决");
      practiceShow.value = false;
    }
  } catch (e) {
    message.error(`提交答案失败：${errText(e)}`);
  } finally {
    grading.value = false;
  }
}

// 只要提交过作答，关闭弹窗就刷新列表（答对置已解决、答错更新次数/时间）
watch(practiceShow, (show) => {
  if (!show && attempted.value) {
    attempted.value = false;
    void reload();
  }
});
</script>

<style scoped>
.filter-bar {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  margin-bottom: 16px;
}

.filter-select {
  width: 240px;
}

.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

.row-main {
  min-width: 0;
  flex: 1;
}

.row-side {
  flex-shrink: 0;
}

.stem {
  font-size: 15px;
  font-weight: 500;
  line-height: 1.6;
  margin-bottom: 8px;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  overflow: hidden;
}

.meta {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}

.source {
  margin-top: 4px;
}

.kp-line {
  margin-bottom: 12px;
}

.q-stem {
  font-size: 16px;
  font-weight: 600;
  line-height: 1.7;
  margin-bottom: 16px;
  white-space: pre-wrap;
}

.q-options {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.q-result {
  margin-top: 16px;
}

.q-line {
  white-space: pre-wrap;
}

.q-explanation {
  margin-top: 6px;
}

.practice-actions {
  display: flex;
  justify-content: flex-end;
  gap: 12px;
  margin-top: 20px;
}
</style>
