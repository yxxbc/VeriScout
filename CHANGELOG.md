# 更新日志

## v0.1.1（2026-10-10）

默认那一轮不再生成 HTML 报告：结论直接写在终端里（Markdown 渲染），要留档再加开关。

### 变更

- **默认输出改为「终端结论」**：`viz.auto_html` 默认 `false`，一轮研究结束只打印模型的收尾答复（标题、要点、链接都渲染好），
  **不写任何文件**。报告仍在，只是变成可选动作。
- **报告工具默认不进入工具面**：3 个报告工具（`create_html` / `edit_html` / `delete_html`）不再发给模型 ——
  工具面即能力，默认运行不会也不能凭空产生文件；越界调用会被明确拒绝
  （`tool 'create_html' is not available in this run: report tools need --html`）。
- **两个开关**：`--html` / `--no-html` 按轮覆盖，REPL 里 `/html on|off` 随时切换，
  配置 `"viz": { "auto_html": true }` 可长期打开；`ves tools` 会显示当轮真实工具面（3 个或 6 个）。
- **终端 Markdown 渲染**：新增 `src/md.rs`，用 `termimad` 渲染标题 / 列表 / 引用 / 代码块 / 表格 / 链接：
  彩色终端带样式并按终端宽度换行，`--no-color` 与管道输出自动去掉语法、不硬换行（依赖只多一个 `termimad`）。
- **提示词随模式切换**：报告模式保留原条款（报告 §1 收录原话、`stats/warnings` 自查、报告文字说人话）；
  答复模式要求「用 Markdown 组织、面向普通读者、不要提占位符」，并明确「这一轮不产出报告，你写的就是交付物」。
- 版本号提升到 `0.1.1`；README / 工具参考 / 配置参考 / 架构设计 / 计划书 / 安全模型同步更新。

## v0.1.0（2026-10-10）

首个可用版本：把「网络检索 + 来源验证 + 可视化」做成一整套封闭工具面。

### 新增

- **封闭工具面**（6 个）：`web_search`、`curl_tool`、`scripts_tool`、`create_html`、`edit_html`、`delete_html`。
  二进制内不含 shell、文件编辑器或通用执行器；注册表封闭性有单元测试兜底。
- **GitHub 发布流水线**：`.github/workflows/ci.yml`（ubuntu / macos 全量测试，Windows 编译 + 冒烟）
  与 `.github/workflows/release.yml`（打 tag 即构建 5 个平台产物、生成 `SHA256SUMS.txt`、
  写好安装说明并用 `gh release create` 发布；支持手动触发按 `Cargo.toml` 版本建 tag）。
- **一行安装脚本**：`install.sh`（macOS / Linux；自动识别平台与 CPU、sha256 校验、
  `-v/-d/-r` 参数与 `VES_*` 环境变量、覆盖安装检测、PATH 提示）与 `install.ps1`（Windows，含用户 PATH 写入）。
- **平台细节**：Linux 产物为静态 musl（Alpine 可用）；Windows 上 `scripts_tool` 默认调用 `python`
  且沙箱资源限制降级（POSIX rlimit 不可用），其余功能不受影响。
- **多引擎检索**：Bing、Wikipedia(zh/en)、DuckDuckGo、SearxNG、Mojeek、
  Google News RSS、Crossref、OpenAlex、arXiv、Hacker News、GitHub、Stack Exchange，
  以及可选的 Tavily / Serper / Brave / 自定义聚合端点。
  结果做黑名单过滤、跨引擎去重、可信度先验打分与时效加权。
- **加固 HTTP 层**：仅 http/https、域名黑名单（域名/通配/正则）、白名单例外、
  私网与回环拦截（含 DNS 解析后二次校验）、响应体积上限、多编码（UTF-8/GB18030/Shift_JIS…）嗅探。
- **Python 网络只读沙箱**：AST 白名单（带行号拒绝）→ 受限 builtins → 导入钩子 → socket 连接钩子 → rlimit → `env_clear`。
- **离线可视化报告**：单文件 HTML（内联 CSS/JS，无 CDN），含主张卡片、来源超链接与可靠性、
  事件关联力图（手写力导向，可拖拽/悬停/高亮）、时间线、溯源页脚；深色/浅色可切换。
- **报告数据模型**：HTML 只是 JSON 模型的投影，`edit_html` 改数据后整体重渲染，避免 HTML 与数据漂移。
- **CLI 与 REPL**：一次性研究、多轮会话、`tools`/`call`/`search`/`fetch`/`script`/`viz`/`config` 子命令，
  `--json` 机器可读输出，`ves call` 可完全绕过模型直接调用工具。
- **本地 AnySearch CLI 接入**：把 `~/.agents/skills/anysearch` 的 CLI 作为一个搜索引擎接进 `web_search`
  （自动探测 `node`/`bash`/`python3` 与 `scripts/anysearch_cli.{js,sh,py}`，未安装则自动跳过）。
  解析其 Markdown 输出（标题 / URL / 摘要 / 日期），纳入同一套黑名单与可信度打分；
  子进程走新的 `proc.rs`（固定 argv、无 shell、硬超时、输出上限），并按 `net.retries` 重试。
  同时把 CLI 的 `extract` 用作**渲染兜底**：`deep=true` 时遇到 JS 站点自动改用它，
  `curl_tool` 新增 `render=true` 显式调用。实测在本机直连 GitHub/Wikipedia 被 TLS 重置的情况下仍能取到结果。
- **模型侧**：OpenAI 兼容客户端（DeepSeek 默认，支持任意兼容端点与本地 Ollama），
  原生 function calling + 文本协议自动降级；上下文预算管理会在超限时压缩早期工具输出。
- **系统提示词强化**：工作流新增「逐条自查」（交卷前核对每个主张都有工具返回的 URL + 原文引文支撑，
  没有就降级/删除）；诚信规则新增「不迎合」条款（不顺着用户预设、主动找反方、区分相关与因果、
  前提可疑先指出）；新增可配置的「说人话」风格（`agent.style`，默认 `plain`：短句、术语先解释、
  结论先行），并把此前未生效的 `agent.language` 字段真正接入提示词。
- **MCP 服务（可被其他 agent 调用）**：`ves mcp` 以 stdio JSON-RPC 暴露 6 个工具 +
  高层 `research`（跑完整研究，返回结论与报告路径），供 Claude Desktop / Cursor / 其他 agent 挂载；
  stdout 只走协议、日志走 stderr、零新依赖，所有请求仍走同一套安全闸门。
- **报告模板 v3**：换成侧边导航 + 概览统计 + 主张筛选/搜索 + 图谱检视面板 + 打印样式的新模板
  （占位符契约不变，渲染层零改动，`{{...}}` 填充与离线自包含测试全过）；
  旧模板留档在 `assets/template/report-v1.html`。
- **报告模板 v4（蓝图墨色主题）**：按 hyxware 的视觉语言重做样式——近黑画布上的工程网格底纹、
  大圆角 Hero 面板（蓝色辉光 + 噪点颗粒 + 品牌水印）、白色主操作按钮、胶囊导航与状态徽章、
  毛玻璃侧栏；深/浅色与打印样式同步适配。噪点用内联 SVG data URI（仍是零外部资源、单文件离线）。
  占位符契约、DOM 结构与绘制脚本逐字节保持，全部测试不变；v3 留档 `assets/template/report-v3.html`。
- **品牌元素（项目 + 报告都有）**：新增项目 logo（`assets/logo.svg` 深色 / `assets/logo-light.svg` 浅色，
  放大镜 + 对勾 = 检索 + 核验）；报告侧栏换成内联 SVG logo，顶栏新增 **GitHub 仓库按钮**
  （地址取自 `Cargo.toml` 的 `repository`，编译期内嵌；留空则按钮自动隐藏，链接带
  `rel="noopener noreferrer"`），并在侧栏、页脚与 `<meta name="generator">` 显示 **版本号**；
  浏览器页签图标（favicon）把 logo 内联成 data URI，报告仍是零外部资源的单文件。
- **README 重写（面向新手）**：logo + 徽章（版本 / Rust 版本 / 平台 / 许可证 / 仓库）、
  一段话讲清「它是什么」，mermaid 流程图，三步上手（装 → 配 Key → 提问），
  常用命令速查表，常见问题 12 条，并把安全、MCP、实测数据、文档索引放到进阶位置。
- **终端只给工具轨迹与可点击的报告位置**：默认输出「每次工具调用一行 + 报告要点 + 可点击链接」，
  路径以 OSC 8 超链接发出（iTerm2 / Ghostty / Kitty / WezTerm / VS Code / Terminal.app 等
  Cmd 或 Ctrl + 点击直接打开；尊重 `NO_COLOR`，`FORCE_HYPERLINK=1|0` 可强制开关）；
  模型的收尾结论**不回显**到终端（它已写进报告 §1），`-v` 才打印完整结论与每一步工具返回，
  `-q` 只留报告链接；报告缺失时结论回退到终端打印（否则信息无处可读）；`ves viz list` 同步可点击。
  终端默认也不再打印工具返回预览（`← tool [ms] …`）。
- **模型的完整结论与工具轨迹写进报告**：一次运行结束时把收尾答复（`Report.conclusion`）与
  完整工具调用日志（`Provenance.steps`：工具、参数摘要、成功/失败、用时、返回大小、错误备注）
  注入报告并重渲染，因此「终端刷过去的细节」在报告 §1 与 §6 有据可查；
  在新会话中重渲染旧报告不会覆盖既有审计记录（有回归测试）。
- **报告模板 v5（审查版式）**：按可读性研究重排——正文 16px / 行高 1.75 / 每行不超过 42em
  （中文约 40 字，对齐 WCAG 1.4.8）、从不两端对齐、状态一律「颜色 + 文字」双编码；
  结构改为 **01 结论**（模型完整答复 + 摘要）→ **02 逐条核对**（一条一行，固定列：
  说法 / 核查结果 / 把握 / 来源编号，引文渐进披露）→ **03 资料来源**（可信度排序 +
  「哪些说法用了它」反查并跳回说法）→ **04 时间线** → **05 关系图** → **06 查证过程**
  （工具调用轨迹表、抓取与被拦截 URL、黑名单规则、模型备注）。新增状态计数 chips
  （点击即筛选）、`/` 聚焦搜索、展开全部证据、行距（`Aa`）切换、阅读进度条；
  打印时自动展开全部证据，直接进 PDF 存档。v4 留档 `assets/template/report-v4.html`。

### 修复

- **模型的原话被占位符挡住**：`finalize` 以前只在结论为空时注入，模型写的「（见最终回答）」这类占位符会留在报告里，
  读者看不到真话。现在收尾答复**无条件覆盖**结论区（模型自己在报告里写的长结论转存为备注，不丢信息）；
  补了「占位符被替换」与「长结论转备注」两个回归测试。

- **浅色按钮点不动**：模板里有两段重复的主题脚本，同一个按钮挂了两个 click 处理（互相抵消，点一次等于没点）；
  「打印」按钮同样注册了两次（会连开两次打印）。删掉重复段后每个按钮只挂一个处理，
  实测点击后 `data-theme`、背景色与按钮文案都真的切换（dark → light → dark，无 console 报错）。

- **模型写的 `#` 标题直接显示在页面上**：markdown 渲染只认 `## ` / `### `，一级标题、有序列表、`> ` 引用与 `---` 分隔线
  都会原样漏出来（报告看着像没写完）。现在 `# `…`###### `、`1. `、`> `、`---` 都正确渲染，并有单元测试。

- **报告与终端文案改成人话**：主张 →「说法」、可靠性 →「可信度」、方法与审计 →「查证过程」、被引用 →「哪些说法用了它」……
  第 02 节新增「状态怎么看」图例（五个状态逐个解释），逐条核对表加了列头（说法 / 核查结果 / 把握），
  首屏把统计换成大数字磁贴、每节用细线分节（杂志式编辑排版，参考 deck.gallery 的报告类目）。

- **提示词：写给读者，不是写给工程师**：终答明确禁止出现工具名、字段名（items/stats/JSON）、内部报错
  与「见最终回答」这类占位符；报告内的 summary / 每条说法 / detail / notes 也要求用大白话写。

- **图谱整块空白（DOM id 冲突）**：v5 模板里 `<section id="graph">` 与图谱 `<svg id="graph">` 撞名，
  `getElementById` 取到的是 section，绘制脚本把节点画进了 HTML 元素里（因此「有节点、没画面」）。
  section 改名 `graph-section` 后图谱恢复；节点标签加了描边，压在连线上也能读清。

- **`--help` / `--version` 长选项无效**：此前只有 `-h` / `-V` 和 `ves help` / `ves version` 生效，
  长选项被解析成无名 switch 后直接落入 REPL。现在两种写法完全等价，并补了回归测试；
  `ves help` 头部也会显示版本号（`ves 0.1.0 — …`）。

- **跑完没报告还甩锅**：以前步数/工具数跑满时会直接要求"不要调用任何工具"，模型只能回一句"我没生成报告"。
  现在收尾前会检查本次是否产出过报告；若没有，先**自动补一轮 `create_html`**（只放开这一个工具；
  文本协议模型走 json 代码块），成功后再要求模型给出含报告文件名的最终答复；
  实在建不出来也会要求模型"如实说明原因"，CLI 同时打印 `⚠ 本次没有生成可视化报告`，不再静默。
- **时间锚点用本地时间**：提示词与任务消息从纯 UTC 改为"本地时间（含 UTC 偏移）+ UTC"双显示
  （`util::iso8601_now_local()`，unix 下走 libc `localtime_r`，其他平台回退 UTC）；
  时效性问题要求先按当前时间算出范围再写检索词。
- **检索词信噪比**：工作流程新增检索词规范（一条查询 2-6 个精准关键词，禁止整句塞入；
  时效性问题必须带时间限定词，如"2026年2月""本周"），`web_search` 工具描述同步更新。
- **不知何时收工**：工作流程新增「何时停止检索」三条收敛标准（子命题均有来源 / 连续两次检索无新信息 /
  预算用掉约 2/3），并要求「证据够一个阶段就先出一版报告，最迟 2/3 预算前必须有第一版，之后用 `edit_html` 增量补」；
  代码层在预算用到约 2/3 时主动注入一条收敛提醒，不再搜到硬上限才停。
- **结论太单薄**：工作流程第 8 步改为「必须有信息量」——核心结论 + 3-5 条带链接要点（覆盖每个子命题）
  + 不确定性与分歧 + 报告文件名，禁止只回一句话。

- **HTML 正文提取缺陷**：原实现把 `<header>` 误判为 `<head>`，导致抓取 Wikipedia 只得到 15 个字符。
  改为**边界感知**标签匹配（标签名后必须是 `>`、空白或 `/`），并在遇到未闭合结构时保留已提取内容。
  实测提取量 15 → 40,013 字符；新增 2 条回归测试。
- **网络抖动**：本机对部分域名存在**间歇性 TLS 重置**，同一 URL 时通时断。
  新增 `net.retries`（默认 3）+ 线性退避：传输层失败对所有方法重试，429/5xx 只对 GET/HEAD 重试。
  实测 `example.com`、`en.wikipedia.org`、`reuters.com` 由经常失败转为稳定成功。
- **黑名单误伤**：`baidu.com` 规则把 `baike.baidu.com` 一起拦掉，新增 `allowed` 白名单机制并放行该域名。
- **输出不一致**：`ves search --json` 与 `web_search` 工具返回结构不同，CLI 改为直接复用工具的 JSON。
- **主张被静默丢弃**（端到端实测发现）：模型把 `confidence` 写成字符串、`source_ids` 写成单个字符串时，
  严格反序列化整批失败并被 `unwrap_or_default()` 静默吞掉，报告里 6 条主张变成 0 条。
  现改为**逐元素容错解析** + 按下标报告跳过原因：
  数字/百分比字符串、单值↔列表、对象映射（`{"1": {...}}`）、`null` 都能纠正；
  空 `claim`/`label`、缺失 `from/to` 会被明确拒绝而不是变成空卡片。
  实测：`confidence: "88%"`、`source_ids: "1"`、`reliability: "80%"` 均正确入库，垃圾元素报 `items[2] was skipped: empty claim`。
  系统提示词同步要求模型创建后核对返回的 `stats`/`warnings` 并用 `edit_html` 补齐。

### 验证

- `cargo test`：**104 项全绿**；`cargo build` / `cargo build --release` 通过。
- 真实网络实测：Bing / Wikipedia 检索与解析、Wikipedia 正文抓取、黑名单拦截、
  `file://` 与 `169.254.169.254` 拒绝、`import subprocess` 沙箱拒绝、报告生成与增量修改。
- **真实模型端到端**：DeepSeek 后端完成一次完整研究（10 步 / 25 次工具调用 / 140 秒），
  自主检索并抓取一手来源（Rustonomicon、The Rust Book、RustBelt POPL 2018、Wikipedia），
  产出分级结论与 `visualizations/rust-data-race-compile-time.html`；
  并在结论中**如实报告环境限制**（DDG/SearxNG 返回空、脚本沙箱因 DNS 解析到私有地址而拒绝访问 Wikipedia，改用 curl_tool 成功）。
- `ves mcp` 协议冒烟：`initialize`（回显协议版本）、`tools/list`、工具错误路径（`isError=true`）、
  未知方法（`-32601`）均实测通过。
- 演示产物：`visualizations/demo-rust-2026.html`（真实检索结果 + 抓取引文）、`visualizations/rust-data-race-compile-time.html`（模型自主产出）。

### 已知限制

- DDG / Mojeek / GitHub / Stack Exchange 在本机网络下常被反爬或 TLS 重置；已做降级与如实报错，但覆盖率受限。
- 需要登录或强 JS 渲染的站点不在覆盖范围（无浏览器自动化）。
- Windows 分支未验证。
