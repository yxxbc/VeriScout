<p align="center"><img src="../assets/logo.svg" alt="VeriScout" width="72"></p>

# 文档索引

> 版本 `ves 0.1.0` · `cargo test` 110 项全绿 · 新手建议先读 [README 的「三分钟上手」](../README.md#2-三分钟上手)

VeriScout（`ves`）的文档按「先看结论，再看细节」的顺序组织：

| 顺序 | 文档 | 适合谁 | 你会得到什么 |
| --- | --- | --- | --- |
| 1 | [../README.md](../README.md) | 所有人 | 它是什么、怎么装、怎么跑第一条命令、6 个工具一览、当前实测状态 |
| 2 | [计划书.md](计划书.md) | 决策者 / 评审 | 背景与问题、目标与非目标、需求清单、关键设计决策与取舍、里程碑、风险、验收标准 |
| 3 | [架构设计.md](架构设计.md) | 开发者 | 模块职责、运行时结构、完整时序、关键类型、排序算法、上下文预算、渲染管线、扩展指南 |
| 4 | [工具参考.md](工具参考.md) | 使用者 / 集成方 | 6 个工具的参数表、返回值、示例、错误情形与选择速查 |
| 5 | [安全模型.md](安全模型.md) | 安全评审 | 威胁模型、五道闸 + 提示词闸、能力清单、审计、**已知局限**、加固建议 |
| 6 | [配置参考.md](配置参考.md) | 运维 / 高级用户 | 全部配置字段、示例配置、环境变量、凭据解析顺序、FAQ |

## 一分钟版本

```bash
# 安装（macOS / Linux；Windows 见 README 的 install.ps1）
curl -fsSL https://raw.githubusercontent.com/yxxbc/VeriScout/main/install.sh | sh

ves "要核实的问题"        # 检索 → 抓原文 → 分级 → 出图 → 结论
ves repl                 # 交互模式
ves tools                # 看看它到底能做什么（只有 6 个）
```

产物：`visualizations/<name>.html`（单文件离线报告）+ `<name>.json`（数据模型，可继续 edit_html）。

## 设计一句话

**能力即边界**：agent 能做什么由工具注册表决定，而不是由提示词约束 ——
二进制里没有 shell、没有文件读写、没有通用执行器，所以被网页诱导也无从下手。
