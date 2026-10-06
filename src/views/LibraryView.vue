<template>
  <div class="page">
    <div class="page-header">
      <div>
        <h2 class="page-title">书架</h2>
        <div class="page-subtitle">点击书籍卡片，查看单元列表与学习进度</div>
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
      <n-button @click="refresh(true)">重试</n-button>
    </n-empty>

    <n-empty v-else-if="store.books.length === 0" description="还没有导入图书，点击右上角「导入图书」">
      <div class="muted small empty-extra">支持 Markdown / TXT / EPUB / PDF（PDF 解析为实验性）</div>
      <n-button type="primary" @click="onImport">导入图书</n-button>
    </n-empty>

    <div v-else class="book-grid">
      <n-card
        v-for="book in store.books"
        :key="book.id"
        class="book-card"
        hoverable
        @click="openBook(book)"
      >
        <div class="book-top">
          <span class="book-title">{{ book.title }}</span>
          <n-button
            class="del-btn"
            size="tiny"
            quaternary
            type="error"
            @click.stop="askRemove(book)"
          >
            删除
          </n-button>
        </div>
        <div class="book-meta">
          <n-tag size="small" :bordered="false">{{ book.format.toUpperCase() }}</n-tag>
          <span class="muted small">导入于 {{ fmtDate(book.added_at) }}</span>
        </div>
        <n-progress
          type="line"
          :percentage="percent(book.done_units, book.total_units)"
          :show-indicator="false"
          :height="8"
          :border-radius="4"
        />
        <div class="muted small book-count">{{ book.done_units }}/{{ book.total_units }} 单元已学</div>
      </n-card>
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
import {
  NButton,
  NCard,
  NEmpty,
  NModal,
  NProgress,
  NSpin,
  NTag,
  useMessage,
} from "naive-ui";
import { api, type BookSummary } from "../api";
import { errText, fmtDate, percent } from "../utils";
import { useLibraryStore } from "../stores/library";

const message = useMessage();
const router = useRouter();
const store = useLibraryStore();

const importing = ref(false);
const confirmShow = ref(false);
const pending = ref<BookSummary | null>(null);
const removing = ref(false);
const loadError = ref<string | null>(null);

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
  grid-template-columns: repeat(auto-fill, minmax(250px, 1fr));
  gap: 16px;
}

.book-card {
  cursor: pointer;
}

.book-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.book-title {
  flex: 1;
  min-width: 0;
  font-size: 15px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.del-btn {
  flex-shrink: 0;
  opacity: 0;
  transition: opacity 0.15s ease;
}

.book-card:hover .del-btn {
  opacity: 1;
}

.book-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 10px 0 14px;
}

.book-count {
  margin-top: 6px;
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
