<template>
  <div class="page">
    <div class="page-header">
      <div>
        <h2 class="page-title">今日学习</h2>
        <div class="page-subtitle">完成到期复习，继续学习新单元</div>
      </div>
      <n-button quaternary size="small" @click="refresh">刷新</n-button>
    </div>

    <div v-if="loading && !plan" class="center-box">
      <n-spin size="large" />
    </div>

    <n-empty v-else-if="error" description="加载今日任务失败">
      <div class="muted small empty-extra">{{ error }}</div>
      <n-button @click="refresh">重试</n-button>
    </n-empty>

    <template v-else-if="plan">
      <n-empty
        v-if="plan.due_reviews.length === 0 && plan.next_units.length === 0"
        description="今天没有任务，去书架导入一本书吧。"
      >
        <n-button type="primary" @click="router.push('/library')">去书架</n-button>
      </n-empty>

      <section v-if="plan.due_reviews.length > 0" class="section">
        <div class="section-head">
          <h3 class="section-title">今日复习</h3>
          <n-tag type="warning" size="small" round>{{ plan.due_reviews.length }}</n-tag>
        </div>
        <n-list bordered>
          <n-list-item v-for="item in plan.due_reviews" :key="item.id">
            <div class="row">
              <div class="row-main">
                <div class="row-title">{{ item.knowledge_point }}</div>
                <div class="muted small">来自《{{ item.book_title }}》{{ item.unit_title }}</div>
              </div>
              <div class="row-side">
                <n-tag size="small" :bordered="false">到期日：{{ item.due_date }}</n-tag>
                <n-button size="small" type="primary" @click="beginReview(item)">开始复习</n-button>
              </div>
            </div>
          </n-list-item>
        </n-list>
      </section>

      <section v-if="plan.next_units.length > 0" class="section">
        <div class="section-head">
          <h3 class="section-title">继续学习</h3>
        </div>
        <n-list bordered>
          <n-list-item v-for="s in plan.next_units" :key="s.unit.id">
            <div class="row">
              <div class="row-main">
                <div class="muted small">《{{ s.book_title }}》下一单元</div>
                <div class="row-title">{{ s.unit.title }}</div>
              </div>
              <div class="row-side">
                <n-button size="small" @click="router.push(`/reading/${s.unit.id}`)">
                  开始学习
                </n-button>
              </div>
            </div>
          </n-list-item>
        </n-list>
      </section>
    </template>

    <n-modal
      v-model:show="reviewShow"
      preset="card"
      title="复习"
      :style="{ width: '560px', maxWidth: '92vw' }"
      :mask-closable="!busy"
      :close-on-esc="!busy"
    >
      <div v-if="taskLoading" class="center-box small-pad">
        <n-spin size="medium" />
        <span class="muted">正在加载复习内容…</span>
      </div>

      <template v-else-if="taskError">
        <n-alert type="error" title="加载复习内容失败">{{ taskError }}</n-alert>
        <div class="review-actions">
          <n-button size="small" @click="retryTask">重试</n-button>
          <n-button size="small" quaternary @click="reviewShow = false">关闭</n-button>
        </div>
      </template>

      <template v-else-if="task">
        <div class="muted small review-source">
          来自《{{ task.item.book_title }}》{{ task.item.unit_title }} · 到期日 {{ task.item.due_date }}
        </div>

        <template v-if="task.question">
          <div class="kp-line">
            <n-tag type="info" size="small" :bordered="false">
              考点：{{ task.item.knowledge_point }}
            </n-tag>
          </div>
          <div class="q-stem">{{ task.question.stem }}</div>
          <n-radio-group
            v-if="task.question.qtype === 'mcq'"
            v-model:value="answer"
            class="q-options"
            :disabled="result !== null"
          >
            <n-radio v-for="(opt, i) in task.question.options" :key="i" :value="opt" :label="opt" />
          </n-radio-group>
          <n-input
            v-else
            v-model:value="answer"
            type="textarea"
            :rows="3"
            placeholder="请输入你的答案…"
            :disabled="result !== null"
          />

          <n-alert
            v-if="result"
            class="q-result"
            :type="result.correct ? 'success' : 'error'"
            :title="result.correct ? '回答正确' : '回答错误'"
          >
            <div v-if="result.reference_answer" class="q-line">
              <b>参考答案：</b>{{ result.reference_answer }}
            </div>
            <div v-if="result.explanation" class="q-line q-explanation">{{ result.explanation }}</div>
          </n-alert>

          <div class="review-actions">
            <n-button
              v-if="result === null"
              type="primary"
              :loading="grading"
              :disabled="!answer.trim()"
              @click="onGrade"
            >
              提交答案
            </n-button>
            <n-button v-else type="primary" :loading="finishing" @click="onContinue">继续</n-button>
          </div>
        </template>

        <template v-else>
          <div class="kp-big">{{ task.item.knowledge_point }}</div>
          <div class="kp-hint muted">你还记得这个知识点吗？</div>
          <div class="review-actions center">
            <n-button type="error" ghost :loading="finishing" @click="finishItem(false)">
              没记住
            </n-button>
            <n-button type="primary" :loading="finishing" @click="finishItem(true)">记住了</n-button>
          </div>
        </template>
      </template>
    </n-modal>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import {
  NAlert,
  NButton,
  NEmpty,
  NInput,
  NList,
  NListItem,
  NModal,
  NRadio,
  NRadioGroup,
  NSpin,
  NTag,
  useMessage,
} from "naive-ui";
import {
  api,
  type GradeResult,
  type ReviewItem,
  type ReviewTask,
  type TodayPlan,
} from "../api";
import { errText } from "../utils";

const router = useRouter();
const message = useMessage();

const plan = ref<TodayPlan | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);

const reviewShow = ref(false);
const taskLoading = ref(false);
const taskError = ref<string | null>(null);
const task = ref<ReviewTask | null>(null);
const currentItemId = ref<number | null>(null);
const answer = ref("");
const grading = ref(false);
const result = ref<GradeResult | null>(null);
const finishing = ref(false);

const busy = computed(() => taskLoading.value || grading.value || finishing.value);

async function refresh(): Promise<void> {
  loading.value = true;
  error.value = null;
  try {
    plan.value = await api.getToday();
  } catch (e) {
    error.value = errText(e);
    message.error(`加载今日任务失败：${error.value}`);
  } finally {
    loading.value = false;
  }
}

onMounted(refresh);

function beginReview(item: ReviewItem): void {
  currentItemId.value = item.id;
  reviewShow.value = true;
  void loadTask(item.id);
}

async function loadTask(itemId: number): Promise<void> {
  task.value = null;
  result.value = null;
  answer.value = "";
  taskError.value = null;
  taskLoading.value = true;
  try {
    task.value = await api.startReview(itemId);
  } catch (e) {
    taskError.value = errText(e);
  } finally {
    taskLoading.value = false;
  }
}

function retryTask(): void {
  if (currentItemId.value != null) void loadTask(currentItemId.value);
}

async function onGrade(): Promise<void> {
  const q = task.value?.question;
  if (!q || grading.value || result.value !== null || !answer.value.trim()) return;
  grading.value = true;
  try {
    result.value = await api.submitAnswer(q.id, answer.value);
  } catch (e) {
    message.error(`提交答案失败：${errText(e)}`);
  } finally {
    grading.value = false;
  }
}

function onContinue(): void {
  if (result.value) void finishItem(result.value.correct);
}

async function finishItem(passed: boolean): Promise<void> {
  const item = task.value?.item;
  if (!item || finishing.value) return;
  finishing.value = true;
  try {
    await api.submitReview(item.id, passed);
  } catch (e) {
    message.error(`提交复习结果失败：${errText(e)}`);
    finishing.value = false;
    return;
  }
  try {
    const p = await api.getToday();
    plan.value = p;
    const remaining = p.due_reviews.filter((r) => r.id !== item.id);
    if (remaining.length > 0) {
      await loadTask(remaining[0].id);
    } else {
      reviewShow.value = false;
      if (p.due_reviews.length === 0) {
        message.success("复习完成，今日复习已全部完成！");
      }
    }
  } catch (e) {
    message.error(`刷新今日任务失败：${errText(e)}`);
    reviewShow.value = false;
  } finally {
    finishing.value = false;
  }
}
</script>

<style scoped>
.section {
  margin-bottom: 24px;
}

.section-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
}

.section-title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
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

.row-title {
  font-weight: 600;
  font-size: 15px;
  margin-bottom: 4px;
  line-height: 1.6;
}

.row-side {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-shrink: 0;
}

.review-source {
  margin-bottom: 16px;
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

.review-actions {
  display: flex;
  justify-content: flex-end;
  gap: 12px;
  margin-top: 20px;
}

.review-actions.center {
  justify-content: center;
}

.kp-big {
  font-size: 22px;
  font-weight: 700;
  text-align: center;
  padding: 28px 8px 8px;
  line-height: 1.6;
}

.kp-hint {
  text-align: center;
}
</style>
