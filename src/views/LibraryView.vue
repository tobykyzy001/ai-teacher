<template>
  <div class="page">
    <div class="page-header">
      <div>
        <h2 class="page-title">书架</h2>
        <div class="page-subtitle">点击书籍封面，查看单元列表与学习进度</div>
      </div>
      <n-button type="primary" :loading="importing" @click="onImport">导入图书</n-button>
    </div>

    <div v-if="importing" class="center-box">
      <n-spin size="large" />
      <span class="muted">正在导入图书…</span>
    </div>

    <div v-else-if="store.loading && store.books.length === 0" class="center-box">
      <n-spin size="large" />
    </div>

    <n-empty v-else-if="loadError && store.books.length === 0" description="加载书架失败">
      <div class="muted small empty-extra">{{ loadError }}</div>
      <n-button class="btn-tinted" @click="refresh(true)">重试</n-button>
    </n-empty>

    <n-empty v-else-if="store.books.length === 0" description="还没有导入图书，点击右上角「导入图书」">
      <div class="muted small empty-extra">支持 Markdown / TXT / EPUB / PDF（PDF 需为可选中文字的版本，扫描件请先 OCR）</div>
      <n-button type="primary" @click="onImport">导入图书</n-button>
    </n-empty>

    <div v-else class="book-grid">
      <div
        v-for="book in store.books"
        :key="book.id"
        class="book-tile"
        @click="openBook(book)"
      >
        <div class="cover" :style="coverStyle(book.title)">
          <span class="cover-title">{{ book.title }}</span>
          <span class="cover-format">{{ book.format.toUpperCase() }}</span>
          <n-button class="cover-close" quaternary title="删除" @click.stop="askRemove(book)">
            <n-icon :size="13"><Close /></n-icon>
          </n-button>
        </div>
        <div class="tile-progress">
          <div
            class="tile-progress-fill"
            :style="{ width: `${percent(book.done_units, book.total_units)}%` }"
          />
        </div>
        <div class="muted small tile-count">{{ book.done_units }}/{{ book.total_units }} 单元已学</div>
      </div>
    </div>

    <n-modal v-model:show="confirmShow" :mask-closable="!removing">
      <n-card class="confirm-card" title="删除图书" size="small" :bordered="false">
        <div>确定要删除《{{ pending?.title }}》吗？</div>
        <div class="muted small confirm-hint">
          该书的阅读进度、做题与复习记录将一并删除，且无法恢复。
        </div>
        <template #footer>
          <div class="modal-actions">
            <n-button :disabled="removing" @click="confirmShow = false">取消</n-button>
            <n-button type="error" :loading="removing" @click="doRemove">删除</n-button>
          </div>
        </template>
      </n-card>
    </n-modal>
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { open } from "@tauri-apps/plugin-dialog";
import { Close } from "@vicons/ionicons5";
import {
  NButton,
  NCard,
  NEmpty,
  NIcon,
  NModal,
  NSpin,
  useMessage,
} from "naive-ui";
import { api, type BookSummary } from "../api";
import { errText, percent } from "../utils";
import { useLibraryStore } from "../stores/library";

const message = useMessage();
const router = useRouter();
const store = useLibraryStore();

const importing = ref(false);
const confirmShow = ref(false);
const pending = ref<BookSummary | null>(null);
const removing = ref(false);
const loadError = ref<string | null>(null);

/* 苹果图书风封面：没有封面图的书，按书名定色生成"大字封面"。
   对同一书名色彩恒定（按标题哈希选一组相邻深化色）。 */
const COVER_PALETTES: [string, string][] = [
  ["#6f94bd", "#54719e"],
  ["#a9776d", "#8a5c52"],
  ["#6f9461", "#567c49"],
  ["#a3856d", "#856f5a"],
  ["#7c7db3", "#60619e"],
  ["#b0867c", "#916a5f"],
  ["#5f9ea0", "#4b8385"],
  ["#a97d95", "#88627a"],
];

function coverStyle(title: string): Record<string, string> {
  let h = 0;
  for (const ch of title) {
    h = (h * 31 + (ch.codePointAt(0) ?? 0)) >>> 0;
  }
  const [a, b] = COVER_PALETTES[h % COVER_PALETTES.length];
  return { backgroundImage: `linear-gradient(150deg, ${a}, ${b})` };
}

async function refresh(force = false): Promise<void> {
  try {
    await store.load(force);
    loadError.value = null;
  } catch (e) {
    loadError.value = errText(e);
    message.error(`加载书架失败：${loadError.value}`);
  }
}

// 已有缓存时静默刷新，避免导航回来看到旧进度
onMounted(() => refresh(store.loaded));

async function onImport(): Promise<void> {
  if (importing.value) return;
  let path: string | null = null;
  try {
    path = await open({
      multiple: false,
      title: "选择要导入的图书",
      filters: [{ name: "电子书", extensions: ["md", "txt", "epub", "pdf"] }],
    });
  } catch (e) {
    message.error(`无法打开文件选择框：${errText(e)}`);
    return;
  }
  if (!path) return;
  importing.value = true;
  try {
    const book = await api.importBook(path);
    store.upsert(book);
    message.success(`《${book.title}》导入成功`);
  } catch (e) {
    message.error(`导入失败：${errText(e)}`);
  } finally {
    importing.value = false;
  }
}

function openBook(book: BookSummary): void {
  router.push(`/books/${book.id}`);
}

function askRemove(book: BookSummary): void {
  pending.value = book;
  confirmShow.value = true;
}

async function doRemove(): Promise<void> {
  const book = pending.value;
  if (!book || removing.value) return;
  removing.value = true;
  try {
    await store.remove(book.id);
    message.success(`已删除《${book.title}》`);
    confirmShow.value = false;
    pending.value = null;
  } catch (e) {
    message.error(`删除失败：${errText(e)}`);
  } finally {
    removing.value = false;
  }
}
</script>

<style scoped>
.book-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(148px, 1fr));
  gap: var(--space-5) var(--space-4);
}

@media (min-width: 821px) {
  .book-grid {
    grid-template-columns: repeat(auto-fill, minmax(184px, 1fr));
    gap: var(--space-6) var(--space-5);
  }
}

.book-tile {
  cursor: pointer;
}

/* 封面：2:3 书型比例 + 书脊 + 微投影，按压微缩（iOS 手感） */
.cover {
  position: relative;
  aspect-ratio: 2 / 3;
  border-radius: var(--radius-card);
  overflow: hidden;
  box-shadow:
    inset 0 0 0 0.5px rgba(255, 255, 255, 0.18),
    0 6px 18px rgba(0, 0, 0, 0.14);
  transition: transform var(--dur-fast) var(--spring);
}

.book-tile:active .cover {
  transform: scale(0.97);
}

.cover::before {
  content: "";
  position: absolute;
  inset: 0 auto 0 0;
  width: 12px;
  background: linear-gradient(
    90deg,
    rgba(0, 0, 0, 0.24),
    rgba(255, 255, 255, 0.1) 65%,
    rgba(255, 255, 255, 0.3) 92%,
    rgba(0, 0, 0, 0.08)
  );
}

.cover-title {
  position: absolute;
  left: 24px;
  right: 12px;
  top: 14px;
  color: rgba(255, 255, 255, 0.95);
  font-size: var(--text-subhead);
  font-weight: 600;
  line-height: 1.45;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.18);
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 4;
  overflow: hidden;
}

.cover-format {
  position: absolute;
  left: 24px;
  bottom: 12px;
  font-size: var(--text-caption2);
  letter-spacing: 0.08em;
  color: rgba(255, 255, 255, 0.72);
}

.cover-close {
  position: absolute;
  top: 8px;
  right: 8px;
  width: 24px;
  height: 24px;
  padding: 0;
  border-radius: 50%;
  background: rgba(0, 0, 0, 0.24) !important;
  color: rgba(255, 255, 255, 0.94);
}

.tile-progress {
  height: 3px;
  border-radius: 2px;
  background: var(--fill-3);
  margin: 12px 2px 5px;
  overflow: hidden;
}

.tile-progress-fill {
  height: 100%;
  border-radius: 2px;
  background: var(--accent);
  transition: width var(--dur) var(--spring);
}

.tile-count {
  padding: 0 2px;
}

.confirm-card {
  width: 420px;
}

.confirm-hint {
  margin-top: 8px;
}

.modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 12px;
}
</style>
