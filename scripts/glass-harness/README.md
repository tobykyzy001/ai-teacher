# glass-harness — 无头浏览器视觉验证

给"磨砂玻璃 / Liquid Glass 到底长什么样"提供**能看的证据**。真实 Tauri 窗口无法截屏
（本机 `screencapture` 报 `could not create image from display`，没有录屏权限），
所以改用 headless Microsoft Edge 渲染同一个 Vue 应用。

本目录只做视觉验证，**不属于产品代码**，也不被 `pnpm build` 打包。

---

## 文件

| 文件 | 作用 |
| --- | --- |
| `harness.html` | 假 Tauri 环境 + 真实 `#/路由`，把整个应用渲染出来 |
| `mobile-frame.html` | 手机尺寸套框（见下面"布局视口最小 492px"） |
| `capture.sh` | 批量截图脚本 |
| `swatches.html` | 材质对照页：old/HEAD 配方 vs new/`--mat-*` 配方 × 三种背景 |
| `states.html` | 按压契约页：圆钮/胶囊 的 rest / hover / active / focus |
| `recipes.css` | 上面的配方定义（old 冻结自 HEAD，new 读工作树令牌） |

产物目录约定：

- `before/` = **HEAD 干净检出**（`/tmp/ai-teacher-head`，port 5320）
- `after/` = **工作树当前**（repo 根，port 5310）

⚠ `baseline/` 这个名字**已废弃**：最初那批截图是在 Lead 正改 `src/style.css` 的
同一分钟里拍的，混进了改造后的材质，不能当"改造前"。现在只用 before / after。

---

## 怎么跑

```bash
# 1) 工作树自己的 dev server（不要碰别人的 5199 / 1420）
cd /Users/zj/Coding/ai-teacher && pnpm dev --port 5310 --strictPort

# 2) （可选）HEAD 干净检出，用于 before/
rm -rf /tmp/ai-teacher-head && mkdir -p /tmp/ai-teacher-head
cd /Users/zj/Coding/ai-teacher && git archive HEAD | tar -x -C /tmp/ai-teacher-head
ln -s /Users/zj/Coding/ai-teacher/node_modules /tmp/ai-teacher-head/node_modules
mkdir -p /tmp/ai-teacher-head/scripts
cp -R /Users/zj/Coding/ai-teacher/scripts/glass-harness /tmp/ai-teacher-head/scripts/glass-harness
cd /tmp/ai-teacher-head && pnpm dev --port 5320 --strictPort

# 3) 截图
cd scripts/glass-harness
./capture.sh after  http://localhost:5310 1280,860
./capture.sh before http://localhost:5320 1280,860     # 在 /tmp 那份副本里执行
./capture.sh after  http://localhost:5310 1280,860 reading   # 只重拍 slug 含 reading 的
```

`capture.sh <out-dir> [base-url] [size] [slug-filter...]`
—— slug 过滤是"包含"匹配，可给多个（任一命中即拍）。

---

## 踩过的坑（改动前先读）

### 1. Edge 截完图不退出

`--headless=old --screenshot=...` 写完 PNG 后进程会**常驻 60s+ 不退出**。
前台 `wait` 会一直挂着。`capture.sh` 的做法是后台起进程 → 轮询 PNG 出现 →
多等 0.5s 让文件写完整 → `kill`。

`--headless=new` 在本机 Edge 154 上直接卡死，只能用 `--headless=old`。

### 2. `--force-prefers-reduced-motion` 不能省

应用有 `.page-fade` 路由过渡（`App.vue`，out-in 约 90+120ms）。没有这个旗标时，
截图经常正好落在过渡中间，**整页渲染成 ~25% 不透明度**——看起来像"材质全糊了"，
其实是动画没跑完。加上之后 `transition-duration` 被压到 0.01ms，稳定可复现。

### 3. macOS 上布局视口最小宽度 ~492px

`--window-size=390,844` 出来的 PNG 确实是 390×844，但页面里 `window.innerWidth = 492`
—— 浏览器按 492 排版，再被裁到 390 宽。表现是"手机上 Tab 栏第 4 项和正文右侧被切掉"，
看着像应用溢出，其实是截图工具的假象（`?debug=1` 会在左上角打印实测的
`innerWidth / clientWidth / dpr`，一眼确认）。

绕过办法：`mobile-frame.html` 把应用塞进一个**精确 390×844 的 iframe**，
iframe 内的 `innerWidth` 就是 390，`position: fixed` 也相对 iframe 视口。
`capture.sh` 对宽度 <900 的尺寸自动走这条路。

### 4. 每个进程必须有独立 `--user-data-dir`

主题是靠 `localStorage['ai-teacher:theme']` 决定的。共用一个 profile 会串主题
（上一条截图写的 night 会漏给下一条）。`capture.sh` 每次 `mktemp -d` 一个。

### 5. 主题值格式是**裸字符串**，不是 JSON

`src/theme-mode.ts` 的 `loadTheme()` 是 `localStorage.getItem(THEME_KEY)` 后
直接和 `ThemeKey` 比较，所以存 `night`，**不是** `"night"`。

### 6. `backdrop-filter` 在 headless 里是真的会渲染的

已经用对角条纹色块验证过：加了 `blur(20px)` 的格子把条纹糊平了，没加的保持锐利。
所以截图里的模糊/折射差异是真实渲染结果，不是降级伪造的。

---

## mock 的形状（`harness.html`）

必须在 `/src/main.ts` 之前装好 `window.__TAURI_INTERNALS__`。做法是**经典内联 script**
（不是 module）：它在解析期同步执行，而 Vite 注入的 `/@vite/client` 与 `main.ts`
都是 defer 的，所以顺序天然成立，不需要等待或 hack。

`@tauri-apps/api/core` 实际只碰这几个成员（见 `node_modules/@tauri-apps/api/core.js`）：

| 成员 | 谁在用 |
| --- | --- |
| `invoke(cmd, args, opts)` | 所有命令 |
| `transformCallback(cb, once)` | `Channel` 构造（流式参数） |
| `unregisterCallback(id)` | `Channel.cleanupCallback` |
| `convertFileSrc(path, protocol)` | EPUB 封面 / 插图（`asset://`） |
| `metadata.currentWindow.label` | `getCurrentWindow()`（`main.ts` 拖窗） |
| `metadata.currentWebview.label` | `getCurrentWebview()` |

另外 `isTauri()` 读的是 `globalThis.isTauri`，要一起设。

`convertFileSrc` 在网页里没有 `asset:` 协议可走，返回一段内联 SVG data URI 占位，
这样书架封面图的 `<img>` 分支仍然可见（否则全部退化成哈希渐变大字封面）。

### 已 mock 的命令

`get_settings` `save_settings` `test_llm_connection` `take_thinking_notice`
`list_books` `import_book` `remove_book` `get_book_detail`
`get_today` `get_chapter_content` `get_unit_content`
`start_chapter_reading` `start_reading` `ask_chapter` `ask` `ask_chapter_selection` `ask_selection`
`set_unit_skipped` `set_chapter_skipped`
`list_wrong_questions` `get_question` `generate_quiz` `regenerate_quiz`
`submit_answer` `start_review` `submit_review`
`get_sync_settings` `save_sync_settings` `test_sync_connection` `get_sync_local_stats`
`sync_upload` `sync_list_remote` `sync_restore` `sync_auto_flush`
`plugin:dialog|open` / `save` / `message`
`plugin:window|start_dragging` / `toggle_maximize` / `is_fullscreen`

未登记的命令不会抛错，只在 console 打 `[harness] 未 mock 的命令：xxx` 并返回 `null`。
加新页面时先看这一行，再补 handler。

### 各路由的数据要求

- `#/today` — 只要 `get_today`（`TodayPlan`：`due_reviews[]` / `next_units[]` /
  `reviewed_today` / `due_total` / `streak_days`）。`due_reviews` 非空才会出现玻璃胶囊按钮。
- `#/library` — 只要 `list_books`。给一条带 `cover_path` 的、几条不带的，
  两种封面分支（封图 vs 哈希渐变）才会同时出现。
- `#/settings` — `get_settings` + `get_sync_settings` + `get_sync_local_stats`。
  主题色板在"外观"卡片里，改 `?theme=` 就能验证是否切对。
- `#/reading/:unitId` — 需要 `get_chapter_content`，返回 `ChapterContent`。
  侧箭头只在 `prev_chapter_unit_id` / `next_chapter_unit_id` 非 null 时渲染，
  mock 里两个都给上，两侧箭头才都出现。`saved_session` 非 null 时会直接展示讲解、
  不触发流式领读——这是让阅读页一次渲染完的关键（否则会停在"正在领读…"）。
- `#/wrong` — `list_wrong_questions`；`QuizQuestion` **绝不能带 `answer` 字段**
  （契约要求，判分只在后端）。
- `#/books/1` — `get_book_detail`。

---

## 静态材质页的两个注意点

### `new/*` 配方读"真身"，不抄快照

`swatches.html` / `states.html` 都 `<link rel="stylesheet" href="/src/style.css?direct">`
—— Vite 的 `?direct` 返回**未加工的 CSS 原文**（不加 HMR 包装），所以页面里拿到的
`--mat-*` 就是工作树当前值。

第一版是把数值抄进 `recipes.css` 的，Lead 随后把 `--mat-control-fill` 从 .52 改到 .80、
`--mat-edge` 从白改暗，对照页立刻说谎。现在改成读真身，代价是这两个页面
**必须经 Vite 打开**（`file://` 双击不行）。

### `old/*` 必须冻结

HEAD 的 `--glass-*` 令牌在工作树里已经删了，所以 old 系列的令牌值**写死在
`recipes.css` 里**（逐字来自 `git show HEAD:src/style.css`），只把选择器改成
`.old-*` 并去掉 `!important`（对照页里没有 naive 内联 style 要压）。

作用域挂在**每个 `.old-*` 类自己身上**。踩过的坑：一开始只声明在共同的
`.old-scope` 上，而 `class="tile old-chrome-button"` 没带那个类，
`var(--glass-sheen)` 全解析为空 → 四个 old 格子完全透明，
看着像"HEAD 的玻璃根本不存在"，正好把结论看反。

### 三条背景带，而且每个格子的底必须**逐像素相同**

`#F2F2F7`（近白，最难看出玻璃）/ 彩色高频"照片"底（最容易糊成奶白、露脏边）/
`#1c1c1e`（暗底，该靠"比背景亮"而不是投影）。单看白底会骗过自己。

⚠ **踩过的坑**：第一版把照片底画成一条跨屏渐变，一行 4 个格子分别落在
橙 / 粉 / 蓝 / 绿四段上（实测 x=30 → `(253,215,199)`，x=538 → `(144,152,221)`），
于是量出来的"材质差异"其实是背景差异，整页结论作废。

现在每个格子是两层结构：`.tile` 画**底**（同一张 232×120 图、
`background-size` 锁死、`no-repeat`），`.mat` 是材质层 `inset:0` 盖上去。
任何两格的底因此完全一致，横向差异只可能来自材质。每带第一个格子是 `bare`
（无材质层），作为"裸底"参照。

### 别在 `.mat` 上写 `transform: scale()`

按压的 `scale(0.96)` 在真实控件上作用于**整个控件**（玻璃 + 文字 + 它覆盖的底）。
但在本页里 `.mat` 只是"盖在底上的一层"，在它上面 scale 会把玻璃缩进去、
四周露出一圈没被玻璃盖住的底 —— 看着像"按下时边缘漏底"，其实是探针页假象。
第一版踩过：`new/press` 的"细节保留率"量出 124%，**比裸底还高**，
数值明显异常才发现。缩放效果请去 `states.html` 看（那里 scale 作用在整个控件上）。

### 量出来的数（`after/swatches.png`，照片底，只取格子左右无文字边距）

| 配方 | 底色保留的细节（std，相对裸底） |
| --- | --- |
| bare（裸底） | 100%（std 35.0） |
| old/chrome-button / old/side-arrow | 46% / 43% |
| old/default-button | 76% |
| new/chrome / new/control / new/overlay | 15% / 26% / 11% |
| new/press | 124% |

结论：新配方**更实**（透过去的底更少，19–26% vs 旧版 43–76%），
所以"文字压不住"的问题改善了；代价是在近白底上玻璃更接近一块实色片，
存在感更依赖 `--mat-edge` 的一圈描边和投影。这条要不要调，取决于想要多"透"。

---

## 看截图时要判断什么

- **奶白感**：亮色下 `--mat-control-fill` 若接近不透明白，在 `#f2f2f7` 页底上
  只剩 6/255 的差 → 看着像一块脏白斑。差异应来自**边缘与投影**，不是增白。
- **脏边**：32px 圆钮上如果叠了多层边（顶部白线 + 内描边 + 底部暗线），
  边缘会糊成两像素脏圈。一道发丝环就够。
- **点了没反馈**：`states.png` 里 rest 与 active 必须一眼看出不同
  （亮色：变暗；暗色：变亮变实）。看不出就是 `--mat-control-fill-active`
  与 `fill` 距离太小。
- **色偏**：`saturate` 超过 ~180% 在书封彩图背后会刺眼。
