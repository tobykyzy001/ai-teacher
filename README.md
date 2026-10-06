# AI 老师

一个围绕"读完一本书"的桌面学习助手：把书拆成学习单元，AI 带你领读、答疑、出题测验，错题和知识点进入间隔重复队列，按艾宾浩斯/SM-2 曲线提醒你复习。

## 功能

- **书架**：导入 Markdown / TXT / EPUB / PDF（PDF 解析为实验性），自动切分为 1500–3000 字的学习单元，展示每本书进度
- **领读**：每个单元生成结构化讲解（背景引入 / 重点梳理 / 难点解释）+ 知识点，支持针对当前单元自由提问
- **做题**：AI 按知识点出 3 道单选 + 2 道简答，简答由 AI 判分并点评，自动计算单元掌握度
- **间隔复习**：学完的知识点和错题进入复习队列，间隔按艾宾浩斯基线（1/2/4/7/15/30 天）起步，之后按 SM-2 依据复习成败动态调整
- **今日学习**：打开应用即见"今日复习 + 每本书下一单元"

## 快速开始

环境要求：Node.js + pnpm、Rust stable（MSVC 工具链）、Windows 需 WebView2（Win10/11 一般自带）。

```bash
pnpm install
pnpm tauri dev      # 开发模式（前端热更新）
pnpm tauri build    # 产出可执行文件（当前 bundle.active=false，不打安装包）
```

首次打开后到 **设置** 页填入 LLM 配置（OpenAI 兼容接口，如 DeepSeek / Kimi / 通义）：`base_url`、`api_key`、`model`，点"测试连接"确认。配置保存在系统应用数据目录的 `config.json`，不会进入本仓库。

**无 Key 体验**：以 `AI_TEACHER_MOCK_LLM=1 pnpm tauri dev` 启动，LLM 走内置 mock，可以完整走通"导入 → 领读 → 做题 → 复习"流程（可用 `fixtures/demo-book.md` 试导）。

## 开发

```bash
cd src-tauri && cargo test   # 48 个离线单测（切分、SM-2、DB、学习闭环）
pnpm build                   # vue-tsc 严格检查 + 前端构建
```

架构与契约约定见 [AGENTS.md](AGENTS.md)。

## 路线图

见 [ROADMAP.md](ROADMAP.md)：迭代 1 打磨日常可用性（流式输出、错题本、统计、打包），迭代 2 深化学习能力（划词解释、论文模式、难度分层出题、错题改编），迭代 3 做数据与扩展（统计图表、导出 Anki、概念图谱、云同步、自动更新）。

详细范围、验收标准与取舍原则见该文件。
