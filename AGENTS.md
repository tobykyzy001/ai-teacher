# AGENTS.md — AI 伴读

桌面学习应用：导入书（md/txt/epub/pdf）→ AI 领读 → 做题 → SM-2 间隔复习。Tauri v2（Rust 后端）+ Vue 3 前端。

新增功能前先对一遍 [ROADMAP.md](ROADMAP.md)：完成计划项就更新其中状态；做 ROADMAP 没有的东西，先把它补进去再动手。其"取舍原则"一节对架构决策有约束力（单机可用、数据可迁移、调度纯函数）。

## 技术栈与包管理

- 前端：Vue 3 + TypeScript + Vite + Pinia + Naive UI，**用 pnpm**
- 已知坑 1：本机 pnpm 11 默认拒绝依赖安装脚本（esbuild、vue-demi 需要），白名单键名是 **`allowBuilds`**（v11 新名，不是 v10 的 `onlyBuiltDependencies`，也不能写 package.json 的 `pnpm` 字段），已配在 `pnpm-workspace.yaml`；全局版在 `C:\Users\45108\AppData\Local\pnpm\config\config.yaml`
- 已知坑 2：`vite.config.ts` 的 `server.watch.ignored` 必须忽略 `src-tauri/target/**`，否则 `tauri dev` 里 vite 因监视 cargo 正在写的 exe 而 EBUSY 崩溃
- 已知坑 3：安装包目标 Windows 是 **NSIS**、macOS 是 **DMG**（`tauri.conf.json` `bundle.targets: ["nsis", "dmg"]`，CLI 按当前平台自动挑选；简体中文界面配在 **`bundle.windows.nsis.languages`**，注意不是 `bundle.nsis`，直接放 `bundle` 下会 schema 报错）。NSIS 工具链由 CLI 首次构建时自动下载并缓存到 `%LOCALAPPDATA%\tauri\NSIS`，之后离线可打包；若哪天切回 MSI，注意打 MSI 需 WiX 且 WiX 下载走 github.com 可能超时
- 已知坑 4：本地图片（EPUB 插图）走 asset 协议展示，配置在 `tauri.conf.json` `app.security.assetProtocol`（enable + scope `$APPDATA/books/**`）就够了；**不要**在 capabilities 里加 `core:asset-protocol:default`——这个 Tauri 版本没有这个权限项，加了编译直接报 Permission not found
- 已知坑 5：应用图标（`src-tauri/icons/`）是 tauri-build 在 **cargo build 时**嵌入 Mach-O 二进制的（target/debug/build/ai-teacher-*/out/ 里的哈希文件）。改图标后 `pnpm tauri dev` 不会因 icons/ 变化触发 Rust 重编译，老 icon 会一直留在 Dock——必须 `touch src-tauri/build.rs`（或任一 .rs）强制重嵌再重启。mac 用圆角 squircle 图（824/1024 形状 + 透明边距，按 Big Sur 栅格），Windows 用满幅方形 ico；macOS 不会像 iOS 自动套圆角模板，圆角必须烘进 icns
- 已知坑 6（发版）：tauri-action 必须显式传 `tagName`（`${{ github.ref_name }}`）才会建/挂 Release——该输入在 action.yml 里**没有默认值**，不传时构建成功但日志末尾 `No releaseId or tagName provided, skipping all uploads...`，什么都发不出去；别指望它自动识别 tag 触发。现用 `tauri-action@v1`（README 当前主版本，v0 同病）
- 已知坑 7（iOS 链接）：Xcode 27 的 swiftc 会把 swift-rs 静态库（libTauri.a 等）里 `@_cdecl` 符号编成**局部符号**（nm 小写 `t`），报 `_run_plugin_command`/`_log_stdout`/`_on_webview_created`/`_register_plugin`/`_init_plugin_dialog` undefined——不是搜索路径问题。swift-rs 用 rustup llvm-tools 的 llvm-objcopy 重新全局化：**先 `rustup component add llvm-tools`，再删掉 `target/<triple>/<profile>/build/{tauri,tauri-plugin-*}-*/out/swift-rs` 强制重跑 build 脚本**（`cargo clean -p tauri` 清不掉这份 OUT_DIR，warning 会被缓存回放）。验证：`nm -gU libTauri.a | grep _run_plugin_command` 应为大写 `T`
- 已知坑 8（iOS 脚手架）：macOS 27 上 brew 彻底不可用（`unknown or unsupported macOS version: :dunno`），`tauri ios init` 死在 brew 装 libimobiledevice。绕法：仓库内 `scripts/ios-init-stubs/` 的 stub 版 `brew`（`list` 回显包名+假版本号、`install/reinstall` 返回 0）+ `pod`（空操作）插到 PATH 最前再 init，随后 xcodegen 正常生成 `src-tauri/gen/apple/ai-teacher.xcodeproj`；模拟器走 simctl 不需要 libimobiledevice。GUI 启动的 Xcode 没有 nvm/cargo 的 PATH，Build Rust Code 阶段会找不到 pnpm——已在 gen/apple/project.yml 的脚本前补 `export PATH="$HOME/.nvm/...:$HOME/.cargo/bin:$PATH"` 再 xcodegen 重生成（注意 `tauri ios init` 重跑会覆盖回写，需重补）
- 已知坑 9（iOS 真机）：**`tauri ios dev` 每次运行会按 tauri.conf 重新同步 gen/apple 工程**（手改 pbxproj 会被回写），签名配置必须落在 `src-tauri/tauri.ios.conf.json`（平台专用覆盖，桌面端不读），内含 iOS 专用 identifier `com.libolun.aiteacher`（`com.example.aiteacher` 太泛被苹果拒收："not available"）+ `bundle.iOS.developmentTeam`（首次需在 Xcode GUI 选一次 Team 生成 Apple Development 证书，此后 CLI 走 `xcodebuild -allowProvisioningUpdates` 自动建描述文件）；免费证书真机首启需在手机上「设置→通用→VPN与设备管理」信任开发者；`vite.config.ts` 已支持 `TAURI_DEV_HOST`（真机 dev 从局域网加载所必需，官方模板标配）；`src-tauri/Info.ios.plist`（会被 iOS 打包自动 merge，加的是 NSLocalNetworkUsageDescription——dev 包从局域网 dev server 加载需「本地网络」权限）；**用户日常使用用 `pnpm tauri ios build` 出 IPA 安装（内嵌 dist 离线可用），`ios dev` 仅开发迭代用**
- iOS 命令：`PATH="$PWD/scripts/ios-init-stubs:$PATH" pnpm tauri ios init --ci`（首次生成 gen/apple，gitignored 不入库）；`pnpm tauri ios dev "iPhone Duo"`（模拟器：vite + cargo + simctl 部署）；`pnpm tauri ios build` → `xcrun devicectl device install app --device <UDID> gen/apple/build/arm64/ai-teacher.ipa` → `xcrun devicectl device process launch --device <UDID> com.libolun.aiteacher`；容器写文件：`devicectl device copy to --domain-type appDataContainer --domain-identifier <id> --source f --destination "Library/Application Support/<id>/f"`；Duo 双屏截图默认截到黑屏的 LCD-1，加 `--display 1` 才是主屏；**iOS 27.1 runtime 只建得动 iPhone Duo 机型，普通 iPhone 模拟器改用本机自带的 iOS 27.0 runtime**：`xcrun simctl create "iPhone 16 Pro" com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro com.apple.CoreSimulator.SimRuntime.iOS-27-0`（`simctl runtime list` 可见 27.0/27.1 两个 Ready 镜像）
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

发版走 GitHub Actions（`.github/workflows/release.yml`）：`git tag v<x.y.z> && git push origin v<x.y.z>` 触发，windows-latest 出 NSIS、macos-latest 出 arm64 DMG、ubuntu-latest 出 AppImage（Linux 通用单文件包，Arch 等发行版从 Release 下载即用，首次报 fuse 错误时 `sudo pacman -S fuse2`；构建依赖按 Tauri 官方清单装在 workflow 的 ubuntu 条件步骤），建**草稿** Release 待改完说明再发布；`bundle.targets` 混列各平台格式（nsis/dmg/appimage）是安全的——CLI 只构当前平台支持的、其余跳过，本地 mac 出不了 Linux 包，AppImage 只能靠 CI 验证；tag 版本号必须与 `tauri.conf.json` 的 `version` 一致（CI 会跑 `pnpm build` 里的 vue-tsc 严格检查，红了直接失败，不会出包）。

## 前后端契约（改任何一侧都要同步另一侧）

- `src/api.ts` 是唯一契约源：`api` 对象的 invoke 封装 + TS 类型，与 Rust `src-tauri/src/models.rs` / `commands.rs` 一一对应
- Tauri v2 默认 camelCase(JS) ↔ snake_case(Rust) 参数转换；Rust 命令**不要**加 `#[tauri::command(rename_all = "snake_case")]`
- `QuizQuestion` 绝不能带 `answer` 字段（判分只能在后端 `submit_answer`）

## 目录结构

```
src/                  # Vue 前端（views/ 七个页面，stores/ 仅 library 缓存）
src/api.ts            # 前后端契约
src-tauri/src/
├── lib.rs            # 入口：注册 dialog 插件、AppState、27 个命令
├── commands.rs       # Tauri 命令薄壳 + 可测试的 *_impl 业务逻辑
├── models.rs         # API 类型（serde snake_case）
├── db.rs             # SQLite 迁移与查询（仅建表不 ALTER；确需修改已有表的列时，必须在提交说明中注明原因；含 learning_log/unit_summaries/chapter_explanations/quiz_batches；测试用 open_in_memory）
├── config.rs         # app_data_dir/config.json（api_key、auto_start_reading、disable_thinking 等，勿入库勿进 git；新字段靠 serde default 兼容旧文件）
├── ingest.rs         # 书籍解析与 1500–3000 字单元切分；EPUB 插图抽到 books/<id>/images/，正文插 `![alt](文件名)` 标记段，前端原位渲染
├── llm.rs            # OpenAI 兼容客户端 + 中文 prompt + mock 模式（长文本输出走 SSE 流式；disable_thinking 开关注入 OpenAI 兼容的 reasoning_effort=none/medium，模型 400/422 拒绝时去掉参数重试一次并留一次性提示 take_thinking_notice；思考内容剥离见 AnswerFilter / strip_thinking）
└── scheduler.rs      # SM-2 调度（基线间隔 [1,2,4,7,15,30]，ease 2.5/≥1.3）
fixtures/demo-book.md # 端到端与测试用的示例书（3 章）
```

## 核心机制速查

- 单元首次 `start_reading` → 每个知识点建 review_item（明天到期，幂等）；答题错 → 该题对应知识点 review_item 重置为明天
- 章（大节）= 阅读/领读单位，单元（小节）= 做题/复习单位：章节归属**不入库**，读时现算——优先重解析源文件的目录层级（EPUB NCX/nav 顶层条目为大节，`parse_file_grouped` + `unit_group_titles` 用与导入相同的切分确定性对齐单元序），源文件缺失/单元数失配时回退按单元标题推导（去"（N）"后缀）；连续聚合 + 超 3 万字强制拆段见 `group_units_into_chapters`；章级命令 `get_chapter_content`/`start_chapter_reading`/`ask_chapter`/`ask_chapter_selection` 参数仍是 unit_id（后端解析所属章）；章领读对章内每节幂等登记（与单元领读共用 `ensure_unit_registered`），章正文喂 LLM 超 1.5 万字取头尾（`cap_llm_input`）；章领读讲解持久化于 `chapter_explanations` 表（键=章首节 unit_id，重开页面直接展示，"重新领读"覆盖）；阅读页翻章入口 = 两侧悬浮箭头（贴**正文列**左右外缘而非窗口最左/最右——正文列 `padding: 0 40px` 固定预留 40px 箭头位；箭头挂在 `position: fixed` 的整条纵向定位带 `.side-zone` 里、`align-items: center` 钉在视口中线，**无论正文多长都不随滚动跑**；定位带宽度 = 正文列到窗口边缘的实际距离，由 `ResizeObserver` 量 `mainColEl` 得 `colRect`（左：left，右：innerWidth-right）写进内联 `width`；**阅读页滚动分到两列内部、菜单列不滚**——锁外层在 **App.vue 自己的 scoped 样式**里用 `.main:has(> .reading-page)`（`height: 100vh` 写死 + overflow hidden，**不要**只靠 grid/flex 的 `minmax(0,1fr)` 百分比链——链上任何一环没撑开，外层就被内容顶长、又退回整页滚动，表现为"滚正文时 AI 栏跟着滚"），正文列 `.main-col` 与领读列 `.ai-col` 各自 `overflow-y: auto` + `overscroll-behavior: contain` + `scrollbar-gutter: stable`；`.main` **必须保留 `n-layout-content`**——它的撑满剩余宽度（以及 <900px 时收成纵向布局）来自该组件自身样式，换成裸 div 会变成"内容多宽就多宽"：书架卡片被挤成单列窄条、阅读页 AI 栏掉到正文下方；`.main:has()` 规则照常能命中它，两条滚动条落在各自列右缘；**别把这条规则写进 ReadingView 的 `<style scoped>`**——`:global(.main…)` 会把 `.reading-page` 的 data 限定一起丢掉、选择器永远命中不了（这正是"外层一直没锁住、只剩一条整页滚动条"的根因）；箭头用 `position: fixed` 纵向钉在视口中线，左右位置由 `colRect`（`mainColEl` 实测矩形：left、innerWidth-right）写内联 `left`/`right`，**不要**用 CSS calc 百分比推算——flex 计算值会滞后、右箭头会掉进 AI 栏；`mainColEl` 是 v-if 之后的节点，必须 `watch(mainColEl)` 挂 ResizeObserver（onMounted 时还是 null）；scoped 里跨组件选择器写 `:global(.main.reading-scroll-host) > :deep(.page.reading-page)`（父级要 :global 才命中 App.vue 节点、子级要 :deep 保留本页 data 属性，防误伤其他页打包产物里的同名 `.reading-page`）；窄屏（<900px）退回单列整页滚动、箭头改 absolute；箭头**常驻**显示：悬浮小圆钮 32×32（半透明白底 + 轻阴影，悬停变实变绿），钉在视口中线——不做"靠近才显形"，用户会找不到；**不能用 `background: transparent`**：macOS WKWebView 对背景透明的原生 `<button>` 会画成浅灰方块；**也不要用 `opacity` 做默认减淡**，会让底下的 reader 背景透上来；首/末章单侧隐藏；章节标题栏右侧另保留「← 上一章 / 下一章 →」按钮）/ ←→ 键 / 双指横滑（wheel 监听挂在 `pageEl` 而非 window、只认 |deltaX|>|deltaY| 的横向滑动、同一手势累计 deltaX 200px 或触屏位移 70px 触发；1s 冷却按手势结束/触发时刻计，均走 `goChapter`，见 ReadingView.vue）；侧边栏"继续阅读"= localStorage 记录的最近阅读路由（`App.vue`，键 `ai-teacher:last-reading`）
- 注意：`pnpm build` 只产出前端 dist，桌面应用必须 `pnpm tauri dev`/build 才会带上新前后端；判断当前应用是哪次构建 → 设置页底部"关于"显示 v版本 + git 短哈希（有未提交改动带 `-dirty`）+ 构建时间（vite.config.ts `define.__BUILD_INFO__` 注入，前端读 `src/build-info.ts`）
- 单元全部题目都答过 → status=done，mastery=每题最近一次答题的正确率
- 测验结果页去向：错题过半（含一半）显示"建议回本章重读"警告 + 一键跳 `/reading/:unitId` 按钮，全对仅庆祝文案、不给重读入口
- 出题量按单元正文字数浮动（`llm.rs plan_quiz`：约每 500 字 1 题、3–8 题封顶、单选:简答≈3:2），不固定 5 题；mock 出题同规则，测试 `setup_unit` 正文 2550 字即对应 5 题
- 单元状态 skipped（无需掌握）：书详情页章/小节 ✔ 标记（`set_unit_skipped`/`set_chapter_skipped`），进度 done_units 计入 done+skipped、标记时删除该单元复习项；领读不回退（`ensure_unit_registered` 仅 unread→reading）、重新出题回退 reading、答完全部题转 done；取消标记回 unread、复习项随下次领读幂等重建
- 重新出题（`regenerate_quiz`）：旧题整批退役**不删除**（`quiz_batches` 表记活跃批次边界 id，`list_questions`/`count_questions`/`latest_attempts` 只看当前批次），错题本/复习项/作答记录继续引用旧题、重练照旧；单元已 done/skipped 则退回 reading 并清空 mastery，新批次答完重算
- 复习通过/失败都走 `scheduler.next_schedule` 更新 ease/reps/interval/due_date，条目永不删除
- 流式（stream=true 常开）：仅领读 explain/答疑 ask/选词问 ask_selection（含对应章级 start_chapter_reading/ask_chapter/ask_chapter_selection）走 SSE，经 `tauri::ipc::Channel<String>` 推到前端（api.ts `streamArg` 助手）；出题/判分等 JSON 输出必须整段解析，不流式；部分网关把思考独白直接拼进 `content`（以 `</think>` 结尾、无开标签），`AnswerFilter` 先压住前缀、见到结束标记后只推答案，非流式 `chat` 用 `strip_thinking` 同样剥离；"该模型是否内联思考"按 (base_url, model, disable_thinking) 缓存在内存，首次未知时整段缓冲、结束再放行；旧库里的讲解/摘要读取时也过 `strip_thinking`（`chapter_explanations` / `unit_summaries`），避免旧思考继续展示或污染上下文；思考开关默认关闭，只按 OpenAI 兼容协议发 `reasoning_effort`（none/medium），模型 400/422 拒绝时去掉参数重试一次、留一次性提示 `take_thinking_notice`，厂商方言留待以后做独立协议适配
- 阅读页划词问 AI **不设选中字数上限**（选多长都能问）；选中超过 500 字时，弹窗顶部提示「已选中 N 字的大段正文…可能产生更多费用」，只提示不拦截；选中原文整段原样随 prompt 发出，后端不截断 quote；划词时克隆保存 Range（`pendingRange`），**弹窗关闭后恢复真实选区 + 「问 AI」按钮回原位**（想再问不用再划），只有用户点正文空白才真正清除；WKWebView 会把选区变动丢在合成层缓存里不刷新（弹窗开/关时正文上残留空白高亮残影），弹窗打开（下一帧 + `@after-enter`）与关闭后都对正文列做 `transform` 合成层重建强制重绘（`forceReaderRepaint`）
- 领读上下文：单元首次领读时生成摘要存 `unit_summaries`（幂等）；领读/答疑带前一单元摘要，无摘要降级为它标题 + 开头摘录
- 错题状态无独立字段：从 attempts 按题最近一次记录派生（有答错记录即入错题本，最近答对即"已解决"），重练直接复用 `submit_answer`
- 今日统计源自 `learning_log`（一行一事件：答题记 'answer'、复习记 'review'）：今日已复习 x/y = 今日复习数 /（今日复习数+剩余到期数），连续天数从今天（无记录则昨天）往回数
- 前端输入+按钮的统一模式（勿回退）：提交/提问按钮不写 `:disabled="!x.trim()"` 锁空值，改绑 `@mousedown="commitComposingInput"`（utils.ts）+ 点击时空值 `message.warning`——中文输入法组词期间 naive-ui 不更新 v-model（`handleInput` 里 isComposing 直接 return），按钮若因旧值禁用，第一次点击只被用来上屏组词、事件被吞，必须点两次；回车提交用 `@keydown.enter` 且过滤 `isComposing || keyCode === 229`（候选词确认的回车不算提交）
- 设计令牌单一来源（严格态 Apple HIG + Liquid Glass，勿回退凭感觉）：`src/style.css` 顶部 `:root` 是全部色/字/距/圆角/动效令牌（Apple text styles 34→11、8pt 栅格、同心圆角、iOS 语义动态色 light/dark 双套），`src/theme.ts` 的 `appleThemeLight/Dark` 与之一一对应、暗色走 naive `darkTheme` 随 `prefers-color-scheme` 切换；视图样式一律用变量，禁止硬编码色值/字号/圆角/间距。Liquid Glass 只允许功能层（侧栏/Tab/悬浮钮/浮层/底部 sheet），内容层（卡片/列表/正文）禁止玻璃与投影（HIG Materials 原文 "Don't use Liquid Glass in the content layer"）；accent 只给主要操作/选中态/可交互元素
- 业务规则集中且要测：写测试走 `*_impl` + 内存 DB + mock LLM，不依赖 Tauri 运行时和网络
