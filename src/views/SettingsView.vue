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
  </div>
</template>

<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";
import {
  NAlert,
  NButton,
  NCard,
  NForm,
  NFormItem,
  NInput,
  NSpace,
  NSpin,
  useMessage,
} from "naive-ui";
import { api, type ConnectionTestResult, type LlmSettings } from "../api";
import { errText } from "../utils";

const message = useMessage();

const form = reactive<LlmSettings>({ base_url: "", api_key: "", model: "" });
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
  margin-bottom: 16px;
}

.test-result {
  margin-top: 16px;
}
</style>
