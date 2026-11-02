<template>
  <div class="page">
    <div class="page-header">
      <div>
        <h2 class="page-title">设置</h2>
        <div class="page-subtitle">配置大模型接口，用于 AI 领读、出题与批改</div>
      </div>
    </div>

    <n-alert type="info" class="tip" :bordered="false">
      Mock 模式：启动后端时设置环境变量 <code>AI_TEACHER_MOCK_LLM=1</code>，可无需 API Key 体验完整功能。
    </n-alert>

    <n-card title="模型接口">
      <n-spin :show="loading">
        <n-alert v-if="loadError" type="error" class="tip" title="设置加载失败">
          {{ loadError }}。你仍然可以直接填写并保存。
        </n-alert>
        <n-form label-placement="top">
          <n-form-item label="接口地址（Base URL）">
            <n-input
              v-model:value="form.base_url"
              placeholder="例如 https://api.openai.com/v1"
              :disabled="loading"
            />
          </n-form-item>
          <n-form-item label="API Key">
            <n-input
              v-model:value="form.api_key"
              type="password"
              show-password-on="click"
              placeholder="sk-…"
              :disabled="loading"
            />
          </n-form-item>
          <n-form-item label="模型名称">
            <n-input v-model:value="form.model" placeholder="例如 gpt-4o-mini" :disabled="loading" />
          </n-form-item>
        </n-form>
        <n-space>
          <n-button type="primary" :loading="saving" :disabled="loading" @click="onSave">保存</n-button>
          <n-button :loading="testing" :disabled="loading" @click="onTest">测试连接</n-button>
        </n-space>
        <n-alert
          v-if="testError || testResult"
          class="test-result"
          :type="testError || testResult?.ok === false ? 'error' : 'success'"
          :title="testError || testResult?.ok === false ? '连接失败' : '连接成功'"
        >
          {{ testError ?? testResult?.message }}
        </n-alert>
      </n-spin>
    </n-card>

    <!-- 外观：全局主题。改这里等于改 <html data-theme>，整个应用（侧栏、各页、
         阅读正文）立刻换色；阅读页的 Aa 菜单改的是同一个 store，两处永远一致 -->
    <n-card title="外观" class="about-card">
      <div class="theme-row" role="group" aria-label="主题">
        <button
          v-for="t in THEME_OPTIONS"
          :key="t.key"
          class="theme-chip"
          :class="{ active: themeStore.key === t.key }"
          :style="{ background: t.swatch, borderColor: t.border }"
          :title="t.label"
          :aria-label="t.label"
          :aria-pressed="themeStore.key === t.key"
          @click="themeStore.setTheme(t.key)"
        >
          <!-- 跟随系统没有单一颜色，用环形对比图标表示"明暗自适应" -->
          <n-icon v-if="t.icon === 'contrast'" class="theme-chip-glyph" :size="22">
            <ContrastOutline />
          </n-icon>
          <span v-else class="theme-chip-check" aria-hidden="true">✓</span>
        </button>
      </div>
      <div class="muted small theme-hint">
        当前：{{ currentThemeLabel }}。主题影响整个应用，不只是阅读正文。
      </div>
    </n-card>

    <n-card title="阅读行为" class="about-card">
      <div class="switch-row">
        <n-switch v-model:value="form.auto_start_reading" :disabled="loading" />
        <div>
          <div>打开新章节时自动开始 AI 领读</div>
          <div class="muted small">
            默认关闭：进入章节不调用 AI，手动点"开始领读"才生成讲解；开启后会对每个全新章节自动领读（消耗 token）。无论开关，已生成过的讲解都会直接展示、不重复生成。
          </div>
        </div>
      </div>
      <div class="switch-row">
        <n-switch v-model:value="thinkingEnabled" :disabled="loading" />
        <div>
          <div>思考模式</div>
          <div class="muted small">
            默认关闭（提速）。关闭时向接口发送 reasoning_effort=none，打开时发送 reasoning_effort=medium；
            只走 OpenAI 兼容协议。模型不支持该参数时会自动去掉参数、按模型默认继续，并提示一次。
            思考内容本身永远不会显示在页面上（思考期间只显示"正在思考"）。
          </div>
        </div>
      </div>
      <n-button type="primary" class="save-behavior" :loading="saving" :disabled="loading" @click="onSave">
        保存
      </n-button>
    </n-card>

    <n-card title="关于" class="about-card">
      <div class="app-identity">
        <div class="app-name">AI 伴读</div>
        <div class="muted small">版本 {{ BUILD_VERSION }}</div>
        <div class="muted small">构建 {{ BUILD_HASH }} · {{ BUILD_TIME }}</div>
      </div>
      <div class="about-section">
        <div class="about-label">开发者</div>
        <div class="muted small">tobykyzy001</div>
      </div>
      <div class="about-section">
        <div class="about-label">隐私说明</div>
        <div class="muted small">
          学习数据与配置保存在本机应用数据目录，不上传服务器；LLM 请求直连你配置的接口地址，不经过第三方中转。
        </div>
      </div>
      <div class="about-section">
        <div class="about-label">帮助与反馈</div>
        <div class="muted small">
          用构建信息对照最新代码是否已生效；覆盖安装或升级不会丢失学习进度与隐私配置。
        </div>
      </div>
    </n-card>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import {
  NAlert,
  NButton,
  NCard,
  NForm,
  NFormItem,
  NIcon,
  NInput,
  NSpace,
  NSpin,
  NSwitch,
  useMessage,
} from "naive-ui";
import { ContrastOutline } from "@vicons/ionicons5";
import { api, type ConnectionTestResult, type LlmSettings } from "../api";
import { BUILD_HASH, BUILD_TIME, BUILD_VERSION } from "../build-info";
import { errText } from "../utils";
import { useThemeStore } from "../stores/theme";
import { THEME_OPTIONS } from "../theme-mode";

const message = useMessage();

// 主题：与阅读页 Aa 菜单共用同一个 store
const themeStore = useThemeStore();
const currentThemeLabel = computed(
  () => THEME_OPTIONS.find((t) => t.key === themeStore.key)?.label ?? "纸白",
);

const form = reactive<LlmSettings>({
  base_url: "",
  api_key: "",
  model: "",
  auto_start_reading: false,
  disable_thinking: true,
});
// UI 上的「思考模式」开关：开 = 明确要求思考，关 = 明确关闭（存的是 disable_thinking）
const thinkingEnabled = computed({
  get: () => !form.disable_thinking,
  set: (v: boolean) => {
    form.disable_thinking = !v;
  },
});
const loading = ref(true);
const loadError = ref<string | null>(null);
const saving = ref(false);
const testing = ref(false);
const testResult = ref<ConnectionTestResult | null>(null);
const testError = ref<string | null>(null);

async function loadSettings(): Promise<void> {
  loading.value = true;
  loadError.value = null;
  try {
    Object.assign(form, await api.getSettings());
  } catch (e) {
    loadError.value = errText(e);
    message.error(`加载设置失败：${loadError.value}`);
  } finally {
    loading.value = false;
  }
}

onMounted(loadSettings);

async function onSave(): Promise<void> {
  saving.value = true;
  try {
    await api.saveSettings({ ...form });
    message.success("设置已保存");
  } catch (e) {
    message.error(`保存失败：${errText(e)}`);
  } finally {
    saving.value = false;
  }
}

async function onTest(): Promise<void> {
  testing.value = true;
  testResult.value = null;
  testError.value = null;
  try {
    // 后端测试的是已保存的设置，先保存当前表单再测试
    await api.saveSettings({ ...form });
    testResult.value = await api.testLlmConnection();
  } catch (e) {
    testError.value = errText(e);
  } finally {
    testing.value = false;
  }
}
</script>

<style scoped>
/* ---------- 外观：主题色板 ---------- */
.theme-row {
  display: flex;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.theme-chip {
  position: relative;
  width: 44px;
  height: 44px;
  padding: 0;
  border-radius: 50%;
  border: 1px solid;
  cursor: pointer;
  /* 选中环用 accent（主题色），未选中只留细边——不用投影，内容层零投影 */
  outline: none;
  transition:
    transform var(--dur-fast) var(--spring),
    box-shadow var(--dur-fast) ease;
}

.theme-chip:hover {
  transform: scale(1.06);
}

/* 选中态：外圈 accent 描边 + 内侧留白，指示器不压在色块上 */
.theme-chip.active {
  box-shadow:
    0 0 0 2px var(--bg-grouped-2),
    0 0 0 3.5px var(--accent);
}

/* 跟随系统的图标片：无填充，图标用正文色描边。
   底色透明 → 在白色卡片上靠 border 划出圆形边界（border 由 :style 的 borderColor 给），
   颜色用 --label 而非 --label-2：对比图标本身线条细，再淡化就糊了 */
.theme-chip-glyph {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--label);
}

.theme-chip-check {
  position: absolute;
  inset: 0;
  display: none;
  align-items: center;
  justify-content: center;
  font-size: var(--text-footnote);
  font-weight: 700;
  /* 勾选色与色块底色反差由主题 accent 决定 */
  color: var(--accent);
  mix-blend-mode: difference;
}

.theme-chip.active .theme-chip-check {
  display: flex;
}

.theme-hint {
  margin-top: var(--space-3);
}

.tip {
  margin-bottom: var(--space-4);
}

.test-result {
  margin-top: var(--space-4);
}

.about-card {
  margin-top: var(--space-4);
}

.about-card .small {
  line-height: 1.75;
}

.switch-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  min-height: var(--hit);
}

.switch-row + .switch-row {
  margin-top: var(--space-3);
}

.save-behavior {
  margin-top: var(--space-3);
}

.app-identity {
  padding-bottom: var(--space-4);
  margin-bottom: var(--space-4);
  border-bottom: 0.5px solid var(--separator);
}

.app-name {
  font-size: var(--text-title2);
  font-weight: 700;
  letter-spacing: -0.01em;
  color: var(--label);
  margin-bottom: var(--space-1);
}

.about-section + .about-section {
  margin-top: var(--space-3);
}

.about-label {
  font-size: var(--text-footnote);
  font-weight: 600;
  color: var(--label);
  margin-bottom: var(--space-1);
}
</style>
