/**
 * 自动同步调度。
 *
 * 设计要点（改动前先读）：
 *
 * 1. **debounce，不是"每次写都传"**：答一道题会写 5 处（attempts / learning_log /
 *    review_items / units …），连答 5 道就是 25 次写。不 debounce 会连发 25 次上传，
 *    既浪费带宽，也可能撞上服务端限流（各家额度不同，且不会在响应里告知）。
 * 2. **只在写命令成功后触发**：见 api.ts 的 WRITE_COMMANDS。翻页、打开书架这类读操作
 *    不写库，不应触发同步。
 * 3. **退出前 flush**：debounce 窗口里可能压着最后一次学习（往往是最重要的那次），
 *    页面卸载时不补传就会丢。
 * 4. **失败静默**：自动同步失败不该弹窗打断学习，只记录状态，下次写操作会自动重试
 *    （后端的脏标记在上传失败时会保留）。
 */

import { api, setWriteHook, type SyncResult } from "./api";

/** 写操作停止多久后上传。太短会连发，太长会让"退出即关机"丢数据 */
const DEBOUNCE_MS = 10_000;

let timer: ReturnType<typeof setTimeout> | null = null;
let running = false;
let installed = false;
/** 最近一次自动同步的结果，供界面展示（不弹窗） */
let lastResult: SyncResult | null = null;
let lastError: string | null = null;

function schedule(): void {
  if (timer !== null) clearTimeout(timer);
  timer = setTimeout(() => {
    timer = null;
    void flush();
  }, DEBOUNCE_MS);
}

/** 立即执行一次自动同步（若开关关闭或无脏数据，后端直接返回 null） */
async function flush(): Promise<void> {
  if (running) {
    // 正在传就别叠加，再排一次让本次写入不被漏掉
    schedule();
    return;
  }
  running = true;
  try {
    const r = await api.syncAutoFlush();
    if (r) {
      lastResult = r;
      lastError = null;
    }
  } catch (e) {
    lastError = e instanceof Error ? e.message : String(e);
  } finally {
    running = false;
  }
}

/**
 * 安装自动同步：注册写命令钩子 + 卸载前 flush。
 * 幂等，重复调用只生效一次（HMR 下也不会重复装监听）。
 */
export function installAutoSync(): void {
  if (installed) return;
  installed = true;

  setWriteHook(() => schedule());

  // 页面卸载（关窗、刷新）时补传。beforeunload 里做异步网络请求可靠性有限，
  // 但 Tauri 的 WebView 会等同步调用返回，实践中足够；同时兜住 HMR 重启。
  const onBeforeUnload = (): void => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    void flush();
  };
  window.addEventListener("beforeunload", onBeforeUnload);
  // 应用退到后台（移动端常见路径）也补一次
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden" && timer !== null) {
      clearTimeout(timer);
      timer = null;
      void flush();
    }
  });
}

export function getAutoSyncStatus(): { result: SyncResult | null; error: string | null } {
  return { result: lastResult, error: lastError };
}

/** 设置页切换开关后立刻试一次，让用户马上看到是否生效 */
export function triggerNow(): void {
  void flush();
}
