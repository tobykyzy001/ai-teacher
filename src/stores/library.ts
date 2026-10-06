import { defineStore } from "pinia";
import { ref } from "vue";
import { api, type BookSummary } from "../api";

export const useLibraryStore = defineStore("library", () => {
  const books = ref<BookSummary[]>([]);
  const loaded = ref(false);
  const loading = ref(false);

  let inflight: Promise<void> | null = null;

  async function load(force = false): Promise<void> {
    if (inflight) return inflight;
    if (loaded.value && !force) return;
    const run = (async () => {
      loading.value = true;
      try {
        books.value = await api.listBooks();
        loaded.value = true;
      } finally {
        loading.value = false;
        inflight = null;
      }
    })();
    inflight = run;
    await run;
  }

  function upsert(book: BookSummary): void {
    if (!loaded.value) return;
    const i = books.value.findIndex((b) => b.id === book.id);
    if (i === -1) {
      books.value.push(book);
    } else {
      books.value[i] = book;
    }
  }

  async function remove(bookId: number): Promise<void> {
    await api.removeBook(bookId);
    books.value = books.value.filter((b) => b.id !== bookId);
  }

  return { books, loaded, loading, load, upsert, remove };
});
