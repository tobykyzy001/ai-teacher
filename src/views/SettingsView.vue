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
        <div class="muted small">腹有诗书气自华、抛物线</div>
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
  NInput,
  NSpace,
  NSpin,
  NSwitch,
  useMessage,
} from "naive-ui";
import { api, type ConnectionTestResult, type LlmSettings } from "../api";
import { BUILD_HASH, BUILD_TIME, BUILD_VERSION } from "../build-info";
import { errText } from "../utils";

const message = useMessage();

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
