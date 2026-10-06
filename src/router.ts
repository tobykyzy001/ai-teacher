import { createRouter, createWebHashHistory } from "vue-router";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", redirect: "/today" },
    { path: "/today", component: () => import("./views/TodayView.vue") },
    { path: "/library", component: () => import("./views/LibraryView.vue") },
    { path: "/books/:bookId", component: () => import("./views/BookDetailView.vue") },
    { path: "/reading/:unitId", component: () => import("./views/ReadingView.vue") },
    { path: "/quiz/:unitId", component: () => import("./views/QuizView.vue") },
    { path: "/settings", component: () => import("./views/SettingsView.vue") },
  ],
});
