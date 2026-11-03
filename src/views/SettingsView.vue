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
        <template v-for="t in THEME_OPTIONS" :key="t.key">
          <!-- 竖向分隔：左边 4 个是配色主题，右边「跟随系统」跟的是系统明暗，
               不是第 5 种配色，用一道线把两组分开 -->
          <span v-if="t.dividerBefore" class="theme-divider" aria-hidden="true" />
          <button
            class="theme-chip"
            :class="{ active: themeStore.key === t.key, split: t.dividerBefore }"
            :data-swatch="t.swatch"
            :style="{ borderColor: t.border }"
            :title="t.label"
            :aria-label="t.label"
            :aria-pressed="themeStore.key === t.key"
            @click="themeStore.setTheme(t.key)"
          >
            <span class="theme-chip-check" aria-hidden="true">✓</span>
          </button>
        </template>
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

    <n-card title="备份与同步" class="about-card">
      <n-alert type="info" class="tip" :bordered="false">
        快照式备份：每次同步整包替换，<strong>不做两端的字段合并</strong>。适合"同一时间只在一台设备上学习"；
        两台设备交叉使用会互相覆盖。凭据只存本机，永不随备份上传。
      </n-alert>

      <n-form label-placement="top">
        <n-form-item label="服务器地址">
          <n-input
            v-model:value="syncForm.base_url"
            placeholder="坚果云填 https://dav.jianguoyun.com/dav/"
          />
        </n-form-item>
        <n-form-item label="账号">
          <n-input v-model:value="syncForm.username" placeholder="坚果云填登录邮箱" />
        </n-form-item>
        <n-form-item label="应用专用密码">
          <n-input
            v-model:value="syncForm.password"
            type="password"
            show-password-on="click"
            placeholder="坚果云：「账户信息 → 安全选项 → 添加应用密码」生成"
          />
        </n-form-item>
        <n-form-item label="远端目录（可留空）">
          <n-input v-model:value="syncForm.folder" placeholder="留空则使用 ai-teacher" />
        </n-form-item>
      </n-form>

      <div class="switch-row">
        <n-switch v-model:value="syncForm.enabled" />
        <div>
          <div>自动同步</div>
          <div class="muted small">
            开启后，答题、复习、领读等写操作完成 10 秒内自动上传进度（约 63KB）；导入或删除书时另传整包（含插图，约 3.4MB）。
            退出应用前会补传一次。默认关闭。
          </div>
        </div>
      </div>

      <n-space>
        <n-button type="primary" :loading="syncSaving" @click="onSaveSync">保存</n-button>
        <n-button :loading="syncTesting" @click="onTestSync">测试连接</n-button>
        <n-button :loading="syncing" @click="onUpload(false)">备份进度</n-button>
        <n-button :loading="syncing" @click="onUpload(true)">备份全部</n-button>
        <n-button :loading="listing" @click="onListRemote">查看远端</n-button>
      </n-space>

      <n-alert
        v-if="syncTestError || syncTestResult"
        class="test-result"
        :type="syncTestError || syncTestResult?.ok === false ? 'error' : 'success'"
        :title="syncTestError || syncTestResult?.ok === false ? '连接失败' : '连接成功'"
      >
        {{ syncTestError ?? syncTestResult?.message }}
      </n-alert>

      <div v-if="localStats" class="sync-stats">
        <div class="about-label">本机当前</div>
        <div class="muted small">
          {{ describeStats(localStats.stats) }} · 数据版本 {{ localStats.schema_version }}
        </div>
      </div>

      <div v-if="remoteSnapshots.length" class="sync-remote">
        <div class="about-label">远端快照</div>
        <div v-for="s in remoteSnapshots" :key="s.name" class="snapshot-row">
          <div class="snapshot-main">
            <div class="snapshot-name">{{ s.name }}</div>
            <div class="muted small">
              {{ formatSize(s.size) }} · {{ s.modified || "时间未知" }}
              <template v-if="s.manifest">
                · {{ s.manifest.includes_books ? "含书籍与插图" : "仅进度" }}
                · {{ describeStats(s.manifest.db_stats) }}
              </template>
              <template v-else> · 无法解析清单</template>
            </div>
          </div>
          <n-button
            size="small"
            :disabled="!s.manifest || restoring === s.name"
            :loading="restoring === s.name"
            @click="onRestore(s)"
          >
            恢复到本机
          </n-button>
        </div>
      </div>
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
  NInput,
  NSpace,
  NSpin,
  NSwitch,
  useMessage,
} from "naive-ui";
import { api, type ConnectionTestResult, type LlmSettings } from "../api";
import type {
  BackupStats,
  LocalStats,
  RemoteSnapshot,
  SyncResult,
  WebDavSettings,
} from "../api";
import { BUILD_HASH, BUILD_TIME, BUILD_VERSION } from "../build-info";
import { errText } from "../utils";
import { triggerNow } from "../auto-sync";
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

// ---------- WebDAV 备份/同步 ----------
const syncForm = reactive<WebDavSettings>({
  enabled: false,
  base_url: "",
  username: "",
  password: "",
  folder: "",
});
const syncSaving = ref(false);
const syncTesting = ref(false);
const syncing = ref(false);
const listing = ref(false);
const restoring = ref<string | null>(null);
const syncTestResult = ref<ConnectionTestResult | null>(null);
const syncTestError = ref<string | null>(null);
const localStats = ref<LocalStats | null>(null);
const remoteSnapshots = ref<RemoteSnapshot[]>([]);

async function loadSync(): Promise<void> {
  try {
    Object.assign(syncForm, await api.getSyncSettings());
  } catch (e) {
    message.error(`加载同步设置失败：${errText(e)}`);
  }
  try {
    localStats.value = await api.getSyncLocalStats();
  } catch {
    /* 统计拿不到不影响其他操作 */
  }
}

onMounted(loadSync);

async function onSaveSync(): Promise<void> {
  syncSaving.value = true;
  try {
    await api.saveSyncSettings({ ...syncForm });
    message.success("同步设置已保存");
    // 刚打开自动同步就试一次，让用户立刻看到是否真的生效
    if (syncForm.enabled && syncForm.password) triggerNow();
  } catch (e) {
    message.error(`保存失败：${errText(e)}`);
  } finally {
    syncSaving.value = false;
  }
}

async function onTestSync(): Promise<void> {
  syncTesting.value = true;
  syncTestResult.value = null;
  syncTestError.value = null;
  try {
    // 先存再测：后端测的是磁盘上的设置
    await api.saveSyncSettings({ ...syncForm });
    syncTestResult.value = await api.testSyncConnection({ ...syncForm });
  } catch (e) {
    syncTestError.value = errText(e);
  } finally {
    syncTesting.value = false;
  }
}

async function onUpload(withBooks: boolean): Promise<void> {
  syncing.value = true;
  try {
    await api.saveSyncSettings({ ...syncForm });
    const r = await api.syncUpload({ ...syncForm }, withBooks);
    message.success(r.message);
    // 备份成功后刷新本地统计与远端列表，让用户立刻看到结果
    localStats.value = await api.getSyncLocalStats();
    await refreshRemote();
  } catch (e) {
    message.error(`备份失败：${errText(e)}`);
  } finally {
    syncing.value = false;
  }
}

async function onListRemote(): Promise<void> {
  listing.value = true;
  try {
    await api.saveSyncSettings({ ...syncForm });
    remoteSnapshots.value = await api.syncListRemote({ ...syncForm });
    if (!remoteSnapshots.value.length) message.info("远端还没有备份");
  } catch (e) {
    message.error(`读取远端失败：${errText(e)}`);
  } finally {
    listing.value = false;
  }
}

async function refreshRemote(): Promise<void> {
  try {
    remoteSnapshots.value = await api.syncListRemote({ ...syncForm });
  } catch {
    /* 列表刷新失败不打断主流程 */
  }
}

async function onRestore(s: RemoteSnapshot): Promise<void> {
  // 恢复会整包覆盖本机数据，必须二次确认并明确说出"会丢掉什么"
  const incoming = s.manifest;
  const current = localStats.value?.stats;
  const lines = [
    `即将用远端快照覆盖本机数据，之后本机的学习进度会被替换。`,
    "",
    `远端：${incoming ? describeStats(incoming.db_stats) : "内容未知"}`,
    `本机：${current ? describeStats(current) : "未知"}`,
    "",
    `快照时间：${incoming?.created_at ?? s.modified}`,
    "",
    `恢复前会自动把本机当前状态另存为 ai_teacher.db.before-restore，恢复错了可以找回。`,
    `确定继续吗？`,
  ];
  if (!window.confirm(lines.join("\n"))) return;

  restoring.value = s.name;
  try {
    const r = await api.syncRestore({ ...syncForm }, s.name, true);
    message.success(r.message);
    localStats.value = await api.getSyncLocalStats();
    // 恢复后各页面缓存的数据都过期了，提示用户刷新
    message.info("数据已替换，请切换到其他页面以加载最新内容", { duration: 5000 });
  } catch (e) {
    message.error(`恢复失败：${errText(e)}`);
  } finally {
    restoring.value = null;
  }
}

function describeStats(s: BackupStats): string {
  return `${s.books} 本书 · ${s.units} 个单元（已完成 ${s.done_units}）· ${s.attempts} 条答题 · ${s.review_items} 条复习`;
}

function formatSize(n: number): string {
  if (n < 1024) return `${n} 字节`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
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

/* 分隔线：与色块等高（44px）并垂直居中，左右等距，
   让它读起来是"分组"，而不是一根不小心多出来的竖线 */
.theme-divider {
  width: 1px;
  height: 44px;
  align-self: center;
  margin: 0 var(--space-1);
  background: var(--separator);
  flex-shrink: 0;
}

.theme-chip-check {
  position: absolute;
  inset: 0;
  display: none;
  align-items: center;
  justify-content: center;
  font-size: var(--text-footnote);
  font-weight: 700;
  /* 白色勾 + 深色投影描边：在纯白、纯黑、半黑半白三种底上都读得出来。
     不能用 mix-blend-mode: difference —— "跟随系统"是左右分割的底，
     反色后的勾会一半反白一半反黑、从中间断成两截 */
  color: #fff;
  text-shadow:
    0 0 2px rgba(0, 0, 0, 0.55),
    0 0 1px rgba(0, 0, 0, 0.6);
}

.theme-chip.active .theme-chip-check {
  display: flex;
}

/* "跟随系统"的底是左右半黑半白，勾选无论什么颜色都会有一半读不出来
   （白勾消失在白半、黑勾消失在黑半），硬画就成了断成两截的残字。
   这一档不画勾，只留外圈 accent 选中环——环是画在色块外的，两半都清楚 */
.theme-chip.split.active .theme-chip-check {
  display: none;
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

/* ---------- 备份与同步 ---------- */
.sync-stats,
.sync-remote {
  margin-top: var(--space-4);
  padding-top: var(--space-3);
  border-top: 1px solid var(--separator);
}

/* 一条快照：左侧信息、右侧按钮；窄屏自动堆叠，避免按钮被挤成两行 */
.snapshot-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-2) 0;
}

.snapshot-row + .snapshot-row {
  border-top: 1px solid var(--separator);
}

.snapshot-main {
  min-width: 0;
}

.snapshot-name {
  font-size: var(--text-callout);
  font-weight: 600;
  color: var(--label);
  word-break: break-all;
}

@media (max-width: 820px) {
  .snapshot-row {
    flex-direction: column;
    align-items: flex-start;
  }
}
</style>
