<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo.svg">
    <source media="(prefers-color-scheme: light)" srcset="assets/logo-light.svg">
    <img src="assets/logo.svg" alt="VeriScout" width="92" height="92">
  </picture>
</p>

<h1 align="center">VeriScout</h1>

<p align="center"><b>给它一个问题，它自己上网查、核对来源，先把结论用大白话说清楚；要留档就加一个开关，得到一份可以离线打开的证据报告。</b></p>
<p align="center"><code>ves</code> —— Rust 编写的网络查证小助手</p>

<!-- 仓库地址指向 https://github.com/yxxbc/VeriScout；改地址时同步替换 Cargo.toml 的 repository 字段与本文件两处引用即可 -->
<p align="center">
  <a href="https://github.com/yxxbc/VeriScout"><img alt="GitHub 仓库" src="https://img.shields.io/badge/GitHub-仓库-181717?logo=github&style=flat-square"></a>
  <a href="https://github.com/yxxbc/VeriScout/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/yxxbc/VeriScout/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/yxxbc/VeriScout/releases/latest"><img alt="最新版本" src="https://img.shields.io/github/v/release/yxxbc/VeriScout?style=flat-square&label=release"></a>
  <img alt="版本" src="https://img.shields.io/badge/version-0.1.1-e3e9f0?labelColor=0a0b0e&style=flat-square">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-1.75%2B-dea584?logo=rust&style=flat-square">
  <img alt="许可证" src="https://img.shields.io/badge/license-MIT-blue?style=flat-square">
  <img alt="平台" src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey?style=flat-square">
</p>

---

## 目录

1. [这是什么](#1-这是什么)
2. [三分钟上手](#2-三分钟上手)
3. [常用命令速查](#3-常用命令速查)
4. [报告长什么样](#4-报告长什么样)
5. [常见问题](#5-常见问题)
6. [安全说明（通俗版）](#6-安全说明通俗版)
7. [让其他 AI 工具调用它](#7-让其他-ai-工具调用它)
8. [开发者专区](#8-开发者专区)
9. [当前状态与实测数据](#9-当前状态与实测数据)
10. [文档索引](#10-文档索引)
11. [许可证](#11-许可证)

---

## 1. 这是什么

**一句话**：你给它一个问题（比如「这条消息是真的吗」「某技术现在发展到哪一步了」），它会上网检索、打开原文、摘出证据、给每条主张标明可信程度，最后生成一份**单文件 HTML 报告**——所有结论都能点回原始链接。

它擅长这三类事：

| 场景 | 例子 |
| --- | --- |
| 核实传言、消息 | 「这条消息说 XX，是真的吗？」 |
| 做主题调研 | 「固态电池产业化进展到哪一步了？」 |
| 梳理争议与时间线 | 「关于 XX 的争论，双方论据是什么？按时间排一下」 |

它**不擅长**（有意为之）：聊天、写代码、操作你的电脑。它只有一个身份——**查证员**。

```mermaid
flowchart LR
  A[你的问题] --> B[多引擎检索]
  B --> C[打开原文<br>摘录引文]
  C --> D[交叉核对<br>主张分级]
  D --> E[生成离线<br>HTML 报告]
  E --> F[双击打开<br>链接回原文]
```

### 和「直接问 AI」有什么不同

| | 直接问 AI | VeriScout |
| --- | --- | --- |
| 来源 | 模型记忆里可能有、也可能编 | 每条结论都带工具**实际访问过**的 URL 与引文 |
| 证据 | 一段话 | 逐条核对 + 来源档案 + 关系图 + 时间线 |
| 事后 | 无法复核 | 报告离线可看、可分享、可打印，链接随时回查 |

---

## 2. 三分钟上手

### 第 0 步：准备

- 一台 macOS / Linux / Windows 电脑；
- **方式 A / B 安装**（下面第 1 步）：不需要 Rust，直接拿预编译包；
- **方式 C 从源码构建**：需要 [Rust 工具链](https://rustup.rs/)（1.75 及以上；安装就一行：`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`）；
- 一个大模型 API Key。默认用 DeepSeek（便宜、中文好），任何 OpenAI 兼容接口都能用（OpenAI、通义、Kimi、本地 Ollama……）。注册地址：[platform.deepseek.com](https://platform.deepseek.com/)。

> 不懂「API Key」？可以理解为**给程序用的账号密码**：在模型官网注册后，到「API Keys」页面创建一个，形如 `sk-xxxxxxxx`。

### 第 1 步：安装

**方式 A（推荐）：一行脚本**——自动识别系统与 CPU、校验 sha256、装进 `~/.local/bin`（或 `/usr/local/bin`）：

macOS / Linux：

```bash
curl -fsSL https://raw.githubusercontent.com/yxxbc/VeriScout/main/install.sh | sh
```

Windows（PowerShell）：

```powershell
irm https://raw.githubusercontent.com/yxxbc/VeriScout/main/install.ps1 | iex
```

脚本支持 `-v v0.1.0`（指定版本）、`-d /usr/local/bin`（指定目录）等参数，也可以设 `VES_VERSION`、`VES_INSTALL_DIR`、`VES_REPO`、`VES_BASE_URL`。

**方式 B：下载预编译包**（[最新 release](https://github.com/yxxbc/VeriScout/releases/latest)）

| 平台 | 文件 |
| --- | --- |
| Linux x86_64（静态，Alpine 可用） | `ves-v0.1.1-x86_64-unknown-linux-musl.tar.gz` |
| Linux arm64（静态） | `ves-v0.1.1-aarch64-unknown-linux-musl.tar.gz` |
| macOS Apple Silicon | `ves-v0.1.1-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `ves-v0.1.1-x86_64-apple-darwin.tar.gz` |
| Windows x64 | `ves-v0.1.1-x86_64-pc-windows-msvc.zip` |

每个包里有可执行文件 + `README.md` + `LICENSE` + `CHANGELOG.md`；校验：`sha256sum -c SHA256SUMS.txt`。

**方式 C：从源码构建**（需要 Rust）

```bash
git clone https://github.com/yxxbc/VeriScout.git
cd VeriScout
cargo build --release        # 产物在 target/release/ves
cargo install --path .       # 或装到 ~/.cargo/bin
```

> **平台说明**：Linux 产物是静态 musl，任何发行版（含 Alpine）都能直接跑。Windows 上 `scripts_tool` 的 Python 沙箱是**降级模式**——需要 Python 在 PATH（默认调用 `python`），POSIX 的资源限制不可用；检索、抓取、报告等功能不受影响。

### 第 2 步：配置 API Key（三种方式，任选其一）

**方式 A（推荐）：环境变量**

```bash
export DEEPSEEK_API_KEY="sk-你的key"
# 想永久生效，就把上面这行加到 ~/.zshrc 或 ~/.bashrc 末尾
```

**方式 B：写进配置文件**

```bash
./target/release/ves config init     # 生成 ./.ves/config.json
# 打开它，把 "api_key": "" 填成 "api_key": "sk-你的key"
```

**方式 C：什么都不做**

如果你机器上已经存过 DeepSeek 凭据（默认会读 `~/.dsh/.credentials.yaml` 与 `~/.ves/credentials.yaml`），它会自动找到，**开箱即用**。

验证一下：

```bash
./target/release/ves config show
# api_key 一行应显示 sk-xxxx…（长度）via …；显示 <missing> 就是还没配好
```

### 第 3 步：问第一个问题

```bash
./target/release/ves "一次性纸杯到底能不能回收"
```

终端只做两件事：实时打印精简的「干活轨迹」（每次工具调用一行），结束时给**可点击的报告位置**——现代终端里 Cmd / Ctrl + 点击就能直接打开：

```
▶ web_search q="一次性纸杯 回收" scope=all
▶ curl_tool https://www.example.com/recycling-faq
▶ create_html name=disposable-cup-recycling title="一次性纸杯能不能回收"

📄 报告  disposable-cup-recycling.html
   12 条说法 · 19 个来源 · 9 条有来源支持 · 来源平均可信度 78/100
   /Users/你的名字/VeriScout/visualizations/disposable-cup-recycling.html
   ↳ 完整结论、工具调用轨迹、拦截记录都在报告 §1 与 §6
```

**模型的结论不会刷在终端**——它已经写进报告 §1，终端只回工具调用与报告位置。想让终端打印完整结论与每一步工具返回，加 `-v`；只想拿到报告链接（比如写进脚本），加 `-q`。

一次完整研究大约 1–3 分钟、几十次模型调用（DeepSeek 上通常是几毛钱级别）。想先零成本试试，用第 3 节的 `ves search` 只检索、不调用模型。

### 第 4 步：打开报告

双击那个 HTML 文件即可——**完全离线**，不联网、不依赖外部资源，发微信、拷 U 盘都能看。报告的顺序是按「审查」排的：

1. **结论**：先看一句话判断与数字，再读**原话（模型收尾答复的逐字全文）**——终端不再回显它，`-v` 才打印；
2. **逐条核对**：报告把问题拆成一条条说法，每条一行，右边固定列写着核查结果 / 把握 / 来源编号，展开「证据」看原文引文；
3. **资料来源**：按可信度排序；「哪些说法用了它」列反查——点编号跳回那条说法；
4. **时间线** 与 **关系图**（拖动节点，点节点看详情）；
5. **查证过程**：每次工具调用、真正打开过的网址、被安全策略挡下的请求、模型留下的备注——终端刷过去的细节全部留档。

顶栏有搜索、状态筛选、`Aa` 行距切换、**打印 / PDF**；键盘按 `/` 聚焦搜索框，`Esc` 关闭浮层。打印时会自动展开全部证据，直接进 PDF 存档。

### 第 5 步：交互模式（可选）

```bash
./target/release/ves        # 不带参数 = 进入对话模式
ves> 帮我核实一下这条消息：……
ves> /help                  # 看看有哪些命令
ves> /viz                   # 列出已生成的报告
ves> /new                   # 清空上下文，换个话题
ves> /exit
```

---

## 3. 常用命令速查

| 我想…… | 命令 | 说明 |
| --- | --- | --- |
| 研究一个问题 | `ves "你的问题"` | 完整流程，**默认在终端给结论**，不写任何文件 |
| 顺便留一份报告 | `ves --html "你的问题"` | 额外产出离线 HTML 报告到 `visualizations/` |
| 只搜一下（不花模型钱） | `ves search "关键词" --limit 5` | 直接看检索结果，不经过大模型 |
| 搜新闻 / 学术 / 代码 | `ves search "关键词" --scope news` | scope：`web` / `news` / `academic` / `code` / `qa` / `all` |
| 深挖：连原文一起抓 | `ves search "关键词" --deep` | 顺带打开前几个结果页 |
| 只看某个网页内容 | `ves fetch https://example.com` | 抓取并转成可读文本 |
| 看网页的响应头 | `ves fetch https://example.com --headers` | 排障时有用 |
| 渲染 JS 重的页面 | `ves fetch https://spa.example.com --render` | 走本地 AnySearch 渲染兜底 |
| 跑一小段 Python | `ves script --code 'print("hi")'` | 在**网络只读沙箱**里执行 |
| 看它有哪些工具 | `ves tools` | 默认 3 个；加 `--html` 变 6 个，这就是全部能力 |
| 直接调用某个工具 | `ves call web_search --args '{"query":"rust"}'` | 脚本化 / 排障 |
| 列出已生成的报告 | `ves viz list` | 也可以 `ves viz open <名字>` 直接打开 |
| 检查配置与密钥 | `ves config show` | 密钥自动打码 |
| 看版本 | `ves --version` | 当前：`ves 0.1.0` |

任何命令都可以加 `--json` 输出机器可读结果（适合写脚本）；加 `-v` 打印完整结论与工具轨迹；加 `-q` 只输出结论（报告模式下则是报告链接）。

---

## 4. 报告长什么样

> 报告是**可选动作**：默认只在终端给结论，加 `--html`（或配置 `viz.auto_html=true`）才生成下面这份文件。

一份报告 = 一个 HTML 文件，按「先结论、再证据、后审计」排成六节：

| 节 | 内容 | 审查时怎么用 |
| --- | --- | --- |
| 01 结论 | 一行导语（几条支持、几条没查实、来源怎么分布）+ 模型写的摘要 + **原话**（收尾答复逐字全文） | 只看一节就能拿走结论 |
| 02 逐条核对 | 一条一行：说法 / 核查结果 / 把握 / 来源编号，列头固定、展开看原文引文，另有「状态怎么看」图例 | 逐条复核，用筛选器定位争议项 |
| 03 资料来源 | 标题、域名、类型、可信度，以及「哪些说法用了它」反查 | 判断哪条来源在承重 |
| 04 时间线 | 关键事件按时间排列 | 判断先后与因果方向 |
| 05 关系图 | 可拖拽的力导向图（因果 / 支持 / 矛盾 / 先后 / 相关 / 提及） | 看整体结构，点节点读详情 |
| 06 查证过程 | 每次工具调用（工具、参数、结果、用时、返回大小）、真正打开过的网址、被安全策略挡下的请求、安全拦截规则、来源平均可信度、模型留下的备注 | 复查「它到底干了什么」 |

版式是按可读性规则 + 杂志式编辑排版做的，不是按仪表盘堆的：

- 首屏是「封面条」：标题、研究问题、一行导语，加一排**大数字**（几条说法 / 几个来源 / 几条有支持 / 来源平均可信度）——先看数字，再决定读不读；
- 正文 16px、行高 1.75、每行不超过 42em（中文约 40 字），**从不两端对齐**（WCAG 1.4.8）；
- 每节用一条细线 + 大号节号分开，导语一句话说清这一节看什么；
- 状态一律「颜色 + 文字」双编码并附图例，色弱也能读；
- 细节用渐进披露：说法与核查结果一眼扫完，引文、查证记录按需展开（NN/g）；
- 顶栏有搜索、状态筛选、展开全部证据、`Aa` 行距切换、`深/浅色` 与打印，`/` 聚焦搜索框。

报告的「名牌」也齐全：侧栏是项目 logo 与版本号，顶栏有 **GitHub 仓库按钮**，浏览器页签显示 logo 图标；深色 / 浅色一键切换（默认跟随系统），打印时自动展开全部证据，可直接导出 PDF 归档。

---

## 5. 常见问题

**Q1：我没有 API Key，能用吗？**

完整研究需要模型来判断与写作，所以跑不了；但 `ves search` / `ves fetch` / `ves script` 不经过模型，可以先玩起来。没配 Key 时程序会明确提示怎么配，不会假装在干活。

**Q2：它会不会乱动我的电脑？**

不会，这是第一设计原则：**能力即边界**。它的工具面被硬性收窄成少数几个专用工具（默认只有 3 个检索/核实工具），二进制里**没有 shell、没有文件编辑器、没有通用执行器**——就算被网页里的恶意文字诱导，也无手可用。详见[第 6 节](#6-安全说明通俗版)。

**Q3：为什么默认不生成 HTML 报告了？**

从 0.1.1 起，默认那一轮只在终端把结论说清楚（Markdown 渲染：标题、要点、链接都在），**不往磁盘写任何东西**——留不留档由你决定。要报告加一个开关即可：

```bash
ves --html "你的问题"        # 这一轮要报告
ves --no-html "你的问题"      # 配置里开了报告，这一轮又不要
```

想以后每次都要，就在配置里写 `"viz": { "auto_html": true }`；开会话时也可以用 `/html on` 随时切换。报告模式下工具面会从 3 个变回 6 个（多出报告工具），终端最后只给一行可点击的报告位置。

**Q4：为什么这条说法被标成「未核实」？**

报告里的状态只代表**证据强度**：找不到可靠来源 ≠ 事情是假的。常见原因是来源质量差、互相矛盾，或这次没抓到一手材料。第 02 节的「状态怎么看」图例把五个状态各解释了一遍；点开来源自己判断即可。

**Q4b：报告里为什么没有模型的原话？**

原话是运行结束时自动写入的（还会把模型在报告里写的旧版结论保留成备注）。如果你看到的是「这份报告里没有模型的原话」提示，说明它由旧版本生成——重新运行一次即可；也可以 `ves call edit_html --args '{"name":"报告名","set_conclusion":"…"}'` 手动补上。

**Q5：报告存在哪？怎么删？**

默认在当前目录的 `visualizations/`（一个 HTML + 一个同名 JSON 数据文件）；默认那轮不生成报告，加 `--html` 或 `viz.auto_html=true` 才有。删除用：

```bash
ves call delete_html --args '{"name":"报告名"}'
```

直接删文件也可以，只是记得把配套 JSON 一起删掉。

**Q6：想换成别的模型 / 本地 Ollama？**

编辑 `./.ves/config.json`：

```jsonc
{
  "llm": {
    "provider": "ollama",
    "base_url": "http://127.0.0.1:11434/v1",
    "model": "qwen2.5:14b",
    "api_key_env": "",        // Ollama 不需要 key
    "api_key": ""
  }
}
```

换成 OpenAI 同理：`base_url` 用 `https://api.openai.com/v1`、`api_key_env` 用 `OPENAI_API_KEY`。

**Q7：我想让它用英文写报告？**

```jsonc
{ "agent": { "language": "en", "style": "plain" } }
```

`language` 控制输出语言，`style` 控制表达风格（默认 `plain`：短句、术语先解释、结论先行）。

**Q8：某些网站抓不到 / 老是失败？**

先看终端里的错误信息（它会如实说明是超时、被拒还是被安全策略拦截）。内置重试与退避（`net.retries`）；JS 很重的站点用 `--render` 走本地 AnySearch 渲染兜底。被黑名单误伤的域名可以在配置里加白名单。

**Q9：一次研究要多久、花多少钱？**

1–3 分钟、几十次模型调用；DeepSeek 上通常是几毛钱级别。想省钱：`--max-steps 8` 限制步数，或先用 `ves search` 摸清材料再研究。

**Q10：「MCP」是什么？我该关心吗？**

只用命令行可以不关心。如果你在用 Claude Desktop、Cursor 这类工具，可以把它挂上去（[第 7 节](#7-让其他-ai-工具调用它)），让那些 AI 获得「上网查证 + 出报告」的能力。

**Q11：怎么升级？**

```bash
# 一行脚本装的：再跑一遍即可（会自动覆盖旧版本）
curl -fsSL https://raw.githubusercontent.com/yxxbc/VeriScout/main/install.sh | sh

# 源码装的：
cd VeriScout && git pull && cargo build --release && cargo install --path .
```

**Q12：能在脚本 / 其他程序里调用吗？**

能。所有命令支持 `--json` 输出；`ves call <tool> --args '{...}'` 可以完全绕过模型直接调工具。

**Q13：常见报错怎么读？**

| 现象 | 含义与处理 |
| --- | --- |
| `TLS ... reset` / 连接被重置 | 网络对某些站点不稳定；已自动重试，其它引擎会继续 |
| `429` / `rate limited` | 请求太快；内置退避，稍等重试即可 |
| `403` | 目标站反爬；换来源，或用 `--render` |
| `api_key: <missing>` | Key 没配好，回到[第 2 步](#2-三分钟上手) |

---

## 6. 安全说明(通俗版)

> 一句话：**它只能「上网看」，不能「动手改」**。完整威胁模型与已知局限见 [docs/安全模型.md](docs/安全模型.md)。

### 它总共只会这 6 件事（默认只用前 3 件）

| 工具 | 作用 | 关键约束 |
| --- | --- | --- |
| `web_search` | 多引擎聚合检索 | 域名黑名单过滤；结果带可信度先验与跨引擎票数 |
| `curl_tool` | 抓取单个网页并转成可读文本 | 仅 http/https；禁内网 / 回环地址；响应体积封顶；不做写盘（除非显式 `save=true`） |
| `scripts_tool` | 沙箱里跑 Python（正则、JSON、统计） | AST 白名单 + 受限 builtins + 网络只读；禁文件 / 进程 / 动态执行 |
| `create_html` | 生成报告 | **默认不提供**（`--html` 或 `viz.auto_html=true` 才开放）；只写 `visualizations/`；拒绝非 http(s) 来源链接 |
| `edit_html` | 修改报告 | 同上，只在报告模式下提供；改 JSON 模型再整体重渲染，HTML 与数据不会漂移 |
| `delete_html` | 删除报告 | 同上；路径经 slug 校验，无法越出报告目录 |

> 表格是**注册表**（二进制里就这 6 个）。默认那一轮只把前 3 个交给模型 —— 工具面即能力，报告工具连 schema 都不发，所以默认运行不会、也不能凭空产生文件。越界调用会被直接拒绝（`tool 'create_html' is not available in this run`）。

### 它**没有**的能力

- 没有 shell：不能执行系统命令、装软件、改系统设置；
- 没有通用文件读写：不能改你的代码、配置或别的文件；
- 不能访问内网、路由器、云元数据接口（SSRF 防护，DNS 解析后二次校验）；
- 网页内容一律当作**不可信数据**：遇到「忽略上面的指令」这类文字只记录、不执行。

### 已知边界（诚实说明）

- 报告里点开外部链接时，是你的浏览器直接访问对方网站（这是链接的正常行为）；
- Python 沙箱是**语言级**隔离，不是容器级硬隔离；
- 来源「可靠性评分」是启发式先验（基于域名背景），不针对单条内容下结论。

---

## 7. 让其他 AI 工具调用它

`ves mcp` 启动一个标准 **MCP（Model Context Protocol）stdio 服务**：把「跑完整研究」和 6 个底层工具暴露给别的 agent（Claude Desktop、Cursor、各类 agent 框架）。MCP 侧的工具是**显式**调用，所以一直给全 6 个；受 `viz.auto_html` 影响的是其中的 `research`（它跑的是同一个 agent 循环）。

```jsonc
// MCP 客户端配置示例（Claude Desktop / Cursor 等）
{
  "mcpServers": {
    "veriscout": {
      "command": "/绝对路径/VeriScout/target/release/ves",
      "args": ["mcp"]
    }
  }
}
```

挂上之后，你对 Claude 说「用 veriscout 查一下 XX 并给我报告」，它就会调用 `research` 工具：一轮完整研究 → 返回结论 + 报告路径。所有请求仍走同一套安全闸门，不新增任何能力。

---

## 8. 开发者专区

### 构建与测试

```bash
cargo build --release          # 常规构建
cargo test                     # 120 项单元测试（封闭工具面 / 离线自包含 / 提示词契约 / Markdown 渲染 / MCP 协议）
```

### 发布与安装脚本

打一个 tag 就会自动构建全平台产物并发布 Release：

```bash
git tag v0.1.0 && git push origin v0.1.0     # 触发 .github/workflows/release.yml
```

- `.github/workflows/release.yml`：5 个平台产物（Linux x64 / arm64 静态 musl、macOS Intel / Apple Silicon、
  Windows x64）+ `SHA256SUMS.txt`，再用 `gh release create` 发布，release notes 里带安装说明；
  手动运行（workflow_dispatch）会按 `Cargo.toml` 的版本号建 tag。
- `.github/workflows/ci.yml`：ubuntu / macos 跑全量测试，Windows 做 `cargo check` 与 release 构建冒烟；
  推 tag 前想先验一遍，直接 `cargo test`。
- `install.sh` / `install.ps1`：安装脚本，自动识别平台、校验 sha256、写入用户目录；
  `VES_VERSION` / `VES_INSTALL_DIR` / `VES_REPO` / `VES_BASE_URL` 可覆盖行为（后者也方便内网镜像与测试）。

如果 `~/.cargo` 不可写（沙箱环境），把缓存放进工作区：

```bash
CARGO_HOME=./.cache/cargo CARGO_TARGET_DIR=./target cargo build --release
```

### 项目结构

```
src/
  main.rs        入口与参数解析
  cli.rs         命令行 / REPL / 子命令
  agent.rs       研究循环（预算、收敛、报告兜底）
  llm.rs         OpenAI 兼容客户端（原生 function calling + 文本协议降级）
  config.rs      配置与凭据解析
  http.rs        加固 HTTP 层（协议/黑名单/私网/体积/编码）
  blacklist.rs   域名规则与可信度先验
  search/        多引擎检索（providers/*）
  tools/         注册表 6 个工具（默认只把前 3 个交给模型；html_tools 负责报告生成与重渲染）
  html/          报告数据模型 + 渲染器（HTML 只是 JSON 模型的投影）
  md.rs          终端 Markdown 渲染（termimad 皮肤策略：彩色 / 纯文本 + 宽度）
  mcp.rs         MCP stdio 服务
  proc.rs        子进程封装（固定 argv、无 shell、硬超时）
  util.rs        文本 / 日期 / URL 归一化
assets/
  logo.svg / logo-light.svg   品牌 logo（报告页签图标也从它内联生成）
  template/report.html        报告模板（编译期内嵌；改完要重新 build）
  python/ves_sandbox.py       脚本沙箱
docs/            计划书 / 架构设计 / 工具参考 / 安全模型 / 配置参考
```

### 品质护栏

- **封闭工具面**有测试兜底：断言注册表恰好 6 个工具、默认只暴露 3 个，`bash/edit/write` 不可能出现；
- **默认不落盘**有测试兜底：断言默认配置的工具面只有 3 个、`create_html` 不在其中，越界调用被拒；
- **离线报告**有测试兜底：输出不含外链资源、含内联 favicon 与版本号、无占位符残留；
- **提示词契约**有测试兜底：断言包含「逐条自查 / 不迎合 / 说人话」条款；
- **MCP 协议**有测试兜底：协议版本回显、`tools/list`、错误以 `isError` 返回、未知方法 `-32601`。

---

## 9. 当前状态与实测数据

**代码与质量**

- 全部模块已实现；**104 项单元测试全绿**（`cargo test`）。
- 工具注册表封闭性、报告离线自包含、提示词契约、MCP 协议均有测试兜底。

**真实网络实测（本机 2026-02）**

| 项目 | 结果 |
| --- | --- |
| `web_search` Bing | ✅ 一次检索稳定返回 8 条并正确解析跳转链接 |
| `web_search` **AnySearch（本地 CLI）** | ✅ 实测 2–4 秒返回结果（含本机直连被 TLS 重置的站点）；抖动时按 `net.retries` 重试，失败则如实报错、让其它引擎继续 |
| `curl_tool render=true`（渲染 JS 页面） | ✅ 返回 4,247 字节 Markdown |
| `web_search` Wikipedia(zh/en) | ✅ 返回并跨语言去重 |
| 黑名单生效 | ✅ `baike.baidu.com` 被 `baidu.com` 规则命中（已加白名单例外） |
| `curl_tool` 抓取 | ✅ Wikipedia 正文 40,013 字符并转为可读文本 |
| 拒绝 `file://` / `169.254.169.254` / 黑名单域 | ✅ 三类均被拒绝并给出可读原因 |
| `scripts_tool` 拒绝 `import subprocess` | ✅ 带行号拒绝 |
| `create_html` + `edit_html` | ✅ 单文件报告、改数据后重渲染一致；无外链资源、无占位符残留 |
| **真实模型端到端** | ✅ 10 步 / 25 次工具调用完成一次完整研究，产出 `visualizations/rust-data-race-compile-time.html`（8 来源、8 事件、8 关联），引用 Rustonomicon、The Rust Book、RustBelt(POPL'18) 等一手来源 |
| DDG / SearxNG / Mojeek / GitHub / StackExchange | ⚠️ 本机网络对部分站点做 TLS 重置或反爬；已实现多实例轮换、降级与**如实报错** |

**资源占用（release 实测）**

| 指标 | 实测值 |
| --- | --- |
| 冷启动 | 3.0 ms（20 次进程启动均值） |
| 常驻内存 | 2.4 MB（最大常驻集） |
| 二进制体积 | 3.79 MB（单文件，无运行时依赖） |

**路线图**

- 长会话上下文压缩、`--json` 全链路脚本化的压力测试；
- 打包脚本（`install.sh`）与 Windows 分支；
- 更多垂直引擎与报告模板主题。

里程碑与验收标准见 [docs/计划书.md](docs/计划书.md)。

---

## 10. 文档索引

| 文档 | 适合谁 | 内容 |
| --- | --- | --- |
| [docs/README.md](docs/README.md) | 所有人 | 文档导航（先看哪一篇） |
| [docs/计划书.md](docs/计划书.md) | 决策者 / 评审 | 背景、目标、需求、设计决策、里程碑、风险、验收标准 |
| [docs/架构设计.md](docs/架构设计.md) | 开发者 | 模块职责、运行时结构、时序、关键类型、扩展点、测试策略 |
| [docs/工具参考.md](docs/工具参考.md) | 使用者 / 集成方 | 工具的完整参数表、返回值、示例、错误情形（含默认只开放哪几个） |
| [docs/安全模型.md](docs/安全模型.md) | 安全评审 | 威胁模型、逐层防护、能力清单、已知局限 |
| [docs/配置参考.md](docs/配置参考.md) | 运维 / 高级用户 | 全部配置字段、示例文件、环境变量、FAQ |

---

## 11. 许可证

[MIT](LICENSE)。

**免责声明**：本工具生成的是**研究辅助材料**，不构成法律、医疗、投资或任何专业意见；模型与检索都可能出错，请以报告里的原始链接为准自行判断。使用请遵守目标网站的服务条款与所在地法律法规。

---

<p align="center"><sub>VeriScout · <code>ves</code> · 让每一次「查一下」都有据可查</sub></p>

