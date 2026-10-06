# AGENTS.md — AI 老师

桌面学习应用：导入书（md/txt/epub/pdf）→ AI 领读 → 做题 → SM-2 间隔复习。Tauri v2（Rust 后端）+ Vue 3 前端。

新增功能前先对一遍 [ROADMAP.md](ROADMAP.md)：完成计划项就更新其中状态；做 ROADMAP 没有的东西，先把它补进去再动手。其"取舍原则"一节对架构决策有约束力（单机可用、数据可迁移、调度纯函数）。

## 技术栈与包管理

- 前端：Vue 3 + TypeScript + Vite + Pinia + Naive UI，**用 pnpm**
- 已知坑 1：本机 pnpm 11 默认拒绝依赖安装脚本（esbuild、vue-demi 需要），白名单键名是 **`allowBuilds`**（v11 新名，不是 v10 的 `onlyBuiltDependencies`，也不能写 package.json 的 `pnpm` 字段），已配在 `pnpm-workspace.yaml`；全局版在 `C:\Users\45108\AppData\Local\pnpm\config\config.yaml`
- 已知坑 2：`vite.config.ts` 的 `server.watch.ignored` 必须忽略 `src-tauri/target/**`，否则 `tauri dev` 里 vite 因监视 cargo 正在写的 exe 而 EBUSY 崩溃
- 已知坑 3：安装包目标是 **NSIS**（`tauri.conf.json` `bundle.targets`，简体中文界面配在 **`bundle.windows.nsis.languages`**，注意不是 `bundle.nsis`，直接放 `bundle` 下会 schema 报错）。NSIS 工具链由 CLI 首次构建时自动下载并缓存到 `%LOCALAPPDATA%\tauri\NSIS`，之后离线可打包；若哪天切回 MSI，注意打 MSI 需 WiX 且 WiX 下载走 github.com 可能超时
- 后端：Rust（tauri 2, rusqlite, reqwest, zip+quick-xml, pdf-extract），SQLite + 文件存储，数据全部在用户 app_data_dir

## 常用命令

```bash
pnpm install                # 装前端依赖
pnpm dev                    # 仅前端 vite dev server
pnpm build                  # vue-tsc 严格检查 + 打 dist（提交前必须绿）
pnpm tauri dev              # 拉起完整桌面应用（debug）
cd src-tauri && cargo test  # 后端全部测试（离线、无需 Tauri 运行时，必须全绿）
```

环境变量 `AI_TEACHER_MOCK_LLM=1`：LLM 全部走确定性 mock，可无 API Key 端到端跑通流程；后端测试依赖它。

## 前后端契约（改任何一侧都要同步另一侧）

- `src/api.ts` 是唯一契约源：`api` 对象的 invoke 封装 + TS 类型，与 Rust `src-tauri/src/models.rs` / `commands.rs` 一一对应
- Tauri v2 默认 camelCase(JS) ↔ snake_case(Rust) 参数转换；Rust 命令**不要**加 `#[tauri::command(rename_all = "snake_case")]`
- `QuizQuestion` 绝不能带 `answer` 字段（判分只能在后端 `submit_answer`）

## 目录结构

```
src/                  # Vue 前端（views/ 七个页面，stores/ 仅 library 缓存）
src/api.ts            # 前后端契约
src-tauri/src/
├── lib.rs            # 入口：注册 dialog 插件、AppState、18 个命令
├── commands.rs       # Tauri 命令薄壳 + 可测试的 *_impl 业务逻辑
├── models.rs         # API 类型（serde snake_case）
├── db.rs             # SQLite 迁移与查询（仅建表不 ALTER；含 learning_log/unit_summaries；测试用 open_in_memory）
├── config.rs         # app_data_dir/config.json（api_key 等，勿入库勿进 git）
├── ingest.rs         # 书籍解析与 1500–3000 字单元切分
├── llm.rs            # OpenAI 兼容客户端 + 中文 prompt + mock 模式（长文本输出走 SSE 流式）
└── scheduler.rs      # SM-2 调度（基线间隔 [1,2,4,7,15,30]，ease 2.5/≥1.3）
fixtures/demo-book.md # 端到端与测试用的示例书（3 章）
```

## 核心机制速查

- 单元首次 `start_reading` → 每个知识点建 review_item（明天到期，幂等）；答题错 → 该题对应知识点 review_item 重置为明天
- 单元全部题目都答过 → status=done，mastery=每题最近一次答题的正确率
- 复习通过/失败都走 `scheduler.next_schedule` 更新 ease/reps/interval/due_date，条目永不删除
- 流式（stream=true 常开）：仅领读 explain/答疑 ask/选词问 ask_selection 走 SSE，经 `tauri::ipc::Channel<String>` 推到前端（api.ts `streamArg` 助手）；出题/判分等 JSON 输出必须整段解析，不流式
- 领读上下文：单元首次领读时生成摘要存 `unit_summaries`（幂等）；领读/答疑带前一单元摘要，无摘要降级为它标题 + 开头摘录
- 错题状态无独立字段：从 attempts 按题最近一次记录派生（有答错记录即入错题本，最近答对即"已解决"），重练直接复用 `submit_answer`
- 今日统计源自 `learning_log`（一行一事件：答题记 'answer'、复习记 'review'）：今日已复习 x/y = 今日复习数 /（今日复习数+剩余到期数），连续天数从今天（无记录则昨天）往回数
- 业务规则集中且要测：写测试走 `*_impl` + 内存 DB + mock LLM，不依赖 Tauri 运行时和网络
