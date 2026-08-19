<p align="center">
  <img src="src-tauri/icons/icon-512.png" width="120" alt="Nova Logo" />
</p>

<h1 align="center">Nova</h1>

<p align="center">
  <strong>本地桌面 AI 编码助手 · 可控自动化 · 人机协同工作流</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Vue-3.5-4FC08D?logo=vue.js" alt="Vue 3.5" />
  <img src="https://img.shields.io/badge/Tauri-2.0-FFC131?logo=tauri" alt="Tauri 2" />
  <img src="https://img.shields.io/badge/Rust-2021-DEA584?logo=rust" alt="Rust" />
  <img src="https://img.shields.io/badge/TypeScript-5.6-3178C6?logo=typescript" alt="TypeScript" />
</p>

---

## 📖 简介

**Nova** 是一个本地桌面 AI 编码助手，由 Vue 3 前端与 Tauri 2 + Rust 后端组成。专为实现可控的 AI 自动化、人工干预流程和端到端任务执行而设计。

Nova 不依赖云端执行环境 —— 所有工具执行、文件操作和终端会话都在本地完成，数据完全留在你的设备上。

---

## ✨ 特性亮点

| 类别 | 能力 |
|------|------|
| 🤖 **多协议多模型** | Anthropic Claude / OpenAI Chat Completions / OpenAI Responses / 任意 OpenAI 兼容端点（DeepSeek、Kimi、智谱 GLM、硅基流动、OpenRouter 等），支持动态拉取模型列表 |
| 🛠️ **30 个内置工具** | Shell 执行、文件读写/补丁、代码搜索（内置 ripgrep）、Web 搜索/抓取、浏览器自动化、桌面控制、定时任务、RAG、MCP 等 |
| 🤖 **智能体套件** | 自定义提示词 + 工具/技能/MCP 装备清单，完整替换默认系统提示词；按会话挂载，独立配置 |
| 🧩 **技能系统** | 基于 SKILL.md 的可插拔技能包（30+ 内置），支持启用/停用，停用后对 AI 完全不可见 |
| 🪝 **生命周期钩子** | 12 个钩子入口（会话开始/结束、工具调用前后、压缩前后、Stop 等），可注入消息或中断流程 |
| 🖥️ **对话作用域工作区** | 每个对话拥有独立持久化工作区、Shell 会话、浏览器状态、会话文件 |
| 👁️ **人工干预流程** | 权限申请与审批，敏感操作不会静默执行；Bash 命令经 AST 解析做 fail-closed 权限判定 |
| 🌐 **内置浏览器（Nova Browser）** | 独立窗口，支持导航、快照、点击、输入、元素标注 |
| 🔌 **MCP 集成** | Model Context Protocol：stdio / SSE / Streamable HTTP 三种传输，动态工具桥接 |
| 📚 **RAG 知识库** | 文档上传、分块、向量检索（sqlite-vec），检索增强生成 |
| ⏰ **定时任务** | Cron 调度（croner），持久化与会话级任务，日历式时间选择器 |
| 🧠 **记忆系统** | 跨会话的偏好、规则和事实记忆，自动去重与冲突清理 |
| 🔐 **本地加密** | API Key 通过 AES-GCM 加密存储，主密钥仅存于本机 |
| 🎯 **多模式 Agent** | Agent / Plan / Auto 三种模式灵活切换，状态机驱动 |

---

## 🏗️ 技术栈

### 前端

| 分类 | 技术 |
|------|------|
| 框架 | Vue 3.5 + Pinia 3 + VueUse 14 |
| 构建 | Vite 6 + TypeScript 5.6 |
| 样式 | TailwindCSS 4.2 |
| UI 组件 | reka-ui + shadcn/ui (Vue) + lucide-vue-next |
| 终端模拟 | xterm 6.0 |
| Markdown | markdown-it + marked + mermaid 11 |
| 图表 | echarts 6 |
| 数学公式 | KaTeX |
| 代码高亮 | highlight.js |
| 文档处理 | pdfjs-dist + docx + jszip |

### 后端

| 分类 | 技术 |
|------|------|
| 框架 | Tauri 2 (Rust 2021 Edition) |
| 异步运行时 | Tokio |
| 数据存储 | SQLite (SQLx) + sqlite-vec（向量检索） |
| 搜索引擎 | ripgrep（内置 rg 二进制 + grep-regex 内核） |
| 进程管理 | portable-pty 持久化 Shell 会话 |
| Cron 调度 | croner 3 |
| Token 计数 | tiktoken-rs（o200k_base BPE） |
| Bash 权限判定 | tree-sitter-bash AST 解析（防 `-c`/`eval`/`$()` 绕过） |
| 屏幕控制 | screenshots + enigo |
| 加密 | aes-gcm（API Key 本地加密） |

---

## 🚀 快速开始

### 前置要求

| 依赖 | 最低版本 |
|------|----------|
| Node.js | 18+ |
| Rust | 1.85+ |
| Tauri CLI | 2+ |

### 安装依赖

```bash
git clone <your-repo-url>
cd nova

# 安装前端依赖
npm install

# Rust 依赖会在首次构建时自动拉取
```

### 开发模式

```bash
# 仅启动 Web UI（端口 1420，适合前端调试）
npm run dev

# 启动完整 Tauri 桌面应用（含前后端热重载）
npm run tauri
```

### 生产构建

```bash
# 构建前端资源（含 vue-tsc 类型检查）
npm run build

# 构建桌面应用（自动检测当前平台）
npm run tauri:build

# 指定平台构建
npm run tauri:win:build     # Windows
npm run tauri:mac:build     # macOS
npm run tauri:linux:build   # Linux
```

---

## 📁 项目结构

```
Nova/
├── src/                          # Vue 3 前端
│   ├── components/
│   │   ├── agent/                #   智能体配置/市场页
│   │   ├── chat/                 #   对话 UI（消息、工作区抽屉、导航等）
│   │   ├── hooks/                #   钩子配置页
│   │   ├── layout/               #   布局 + 设置页 Tab（模型/RAG/MCP/技能/记忆…）
│   │   ├── schedule/             #   定时任务页
│   │   └── ui/                   #   基础 UI 组件 (shadcn/ui)
│   ├── features/
│   │   ├── browser/              #   Nova Browser 自动化
│   │   ├── chat/                 #   对话核心逻辑（controllers/services/utils）
│   │   └── workspace/            #   工作区 API
│   └── lib/                      # 公共库（载荷构造、类型、Markdown 渲染等）
├── src-tauri/                    # Tauri + Rust 后端
│   └── src/
│       ├── command/              # Tauri IPC 命令层（18 个模块，96 个命令）
│       ├── llm/
│       │   ├── adapters/         #   API 协议适配（anthropic/openai/responses）
│       │   ├── tools/            #   26 个工具模块（build.rs 自动注册）
│       │   ├── services/         #   21 个服务（智能体/钩子/RAG/MCP/Cron…）
│       │   ├── commands/         #   内部命令（compact/memory/resume）
│       │   ├── query/            #   Agent 模式状态机
│       │   └── utils/            #   系统提示词、权限、token 计数等
│       ├── prompt/               # 系统提示词模板
│       ├── windowTokens/         # 模型上下文窗口预设库
│       └── logging/              # 日志配置
└── public/                       # 静态资源
```

---

## 🏛️ 架构概览

```
┌─────────────────────────────────────────────────────────────────┐
│                        Vue 3 Frontend                           │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────┐   │
│  │ Chat UI  │  │Workspace │  │ Browser  │  │  Settings    │   │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └──────┬───────┘   │
└───────┼──────────────┼──────────────┼───────────────┼───────────┘
        │              │              │               │
        ▼              ▼              ▼               ▼
┌──────────────────── Tauri IPC (96 commands) ─────────────────────┐
└─────────────────────────────┬─────────────────────────────────────┘
                              │
┌─────────────────────────────▼─────────────────────────────────────┐
│                     Rust Backend (Tauri 2)                        │
│  ┌──────────┐  ┌───────────┐  ┌──────────┐  ┌──────────────┐   │
│  │ Commands │  │ LLM Core  │  │  Tools   │  │  Services    │   │
│  │  (IPC)   │  │(Adapters) │  │  (30个)  │  │   (21个)     │   │
│  └──────────┘  └───────────┘  └──────────┘  └──────────────┘   │
│  ┌──────────┐  ┌───────────┐  ┌──────────┐  ┌──────────────┐   │
│  │  SQLite  │  │  Memory   │  │   MCP    │  │ Agent Hooks  │   │
│  │ (SQLx)   │  │  System   │  │  Client  │  │ Skills Cron  │   │
│  └──────────┘  └───────────┘  └──────────┘  └──────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

### 数据流

```
用户消息 → 上下文组装（历史 + RAG + MCP + 记忆 + 会话文件）
    → LLM 调用（协议适配器流式解析）
    → 工具执行循环（权限审批 + 钩子拦截）
    → 状态持久化（SQLite）
    → Tauri Events 实时推送 → 前端更新
```

### 通信方式

| 方向 | 机制 | 用途 |
|------|------|------|
| 前端 → 后端 | Tauri IPC (invoke) | 命令调用、配置读写 |
| 后端 → 前端 | Tauri Events (emit) | 流式响应推送、状态变更通知 |

---

## 🧩 核心模块说明

### LLM 协议适配 (`src-tauri/src/llm/adapters/`)

三种 API 协议适配器，流式事件统一规范化为共享 `Delta` 类型：

| 适配器 | 说明 |
|--------|------|
| `anthropic` | Claude 协议，支持 extended thinking、stop sequences |
| `openai` | OpenAI Chat Completions 兼容协议（默认，覆盖所有兼容端点） |
| `openai_responses` | OpenAI Responses API |

新增提供商无需写代码 —— 在设置页填入 `base_url` + `api_key` + `model`，选择对应协议即可。`fetch_available_models` 支持从兼容端点动态拉取模型列表（内置常见兼容子路径推断）。

### 工具系统 (`src-tauri/src/llm/tools/`)

**零手写注册**：`build.rs` 在编译期扫描 `tools/*/mod.rs` 中导出的 `registrations()`，自动生成工具注册表。新增工具只需创建目录并实现注册函数，无需修改任何中心清单。

| 类别 | 工具 |
|------|------|
| 终端 | `Bash`（AST 权限判定） |
| 文件编辑 | `Read` · `Write` · `Edit` · `MultiEdit` |
| 搜索 | `Grep` · `Glob` · `git_diff` · `web_search` · `web_fetch` |
| 浏览器 | `nova_browser_navigate/click/type/snapshot/reset` |
| 桌面控制 | `computer_use` |
| 规划 | `enter_plan_mode` · `exit_plan_mode` · `TodoWrite` |
| 交互 | `ask_user_question` |
| 定时任务 | `CronCreate` · `CronDelete` · `CronList` |
| 知识库 | `rag_tool` · `remember_global_memory` |
| 技能 | `Skill` |
| MCP | `mcp_auth` · `list_mcp_resources` · `read_mcp_resource` |
| 配置 | `config_tool` |

### 智能体套件 (`src-tauri/src/llm/services/agent_bundles/`)

- **定义**：提示词 + 工具/技能/MCP 服务器装备清单，存储于 `agents/<id>.json`
- **提示词**：完整替换默认系统提示词，实现真正独立的智能体
- **锁定工具**：`EnterPlanMode`/`ExitPlanMode`/`ask_user_question` 恒可用（流程必需）
- **会话挂载**：按会话绑定，进程内缓存 + 写穿透；使用中不允许切换
- **工具过滤**：未勾选的工具对 AI 完全不可见（提示词与执行层双重过滤）

### 技能系统 (`src-tauri/src/llm/services/skills/`)

- 扫描 `skills/` 目录下所有 `SKILL.md`（frontmatter 含 name/description）
- 停用的技能从系统提示词和 SkillTool 中完全移除，不消耗 token
- 内置 30+ 技能：docx/pptx/pdf 处理、前端设计、品牌规范、canvas 设计、调试方法论等

### 钩子系统 (`src-tauri/src/llm/services/hooks/`)

12 个钩子入口，可注入消息、阻止继续、覆盖错误：

| 阶段 | 钩子 |
|------|------|
| 会话 | `session_start` · `session_end` |
| 提示 | `user_prompt_submit` |
| 工具 | `pre_tool_use` · `post_tool_use` · `post_tool_use_failure` |
| 压缩 | `pre_compact` · `post_compact` |
| 停止 | `stop` · `error` |
| 子智能体 | `subagent_start` · `subagent_stop` |

### 记忆系统

- **存储**：SQLite + 文件备份，分 `preference`（偏好）、`rule`（规则）、`fact`（事实）
- **检索**：查询感知，注入持久规则/偏好 + 当前请求相关事实
- **维护**：自动去重、冲突清理，新规则替换过时重复项

### 定时任务

- **调度**：标准 5 字段 Cron（croner 解析），后台调度循环随应用启动
- **存储**：`session`（会话级）/ `durable`（持久化）
- **绑定**：每个任务自动创建并绑定专用对话
- **UI**：日历式时间选择器（单次/每天/每周/每月/自定义），Cron 反解析为可读描述

### 安全与权限

- **API Key 加密**：AES-GCM，主密钥仅存于本机 `master_key` 文件
- **Bash 权限判定**：tree-sitter-bash 解析命令 AST，识别 `-c`/`eval`/`$()`/wrapper 等绕过手段，fail-closed（无法解析则拒绝）
- **权限审批**：弹窗展示具体命令，单次拒绝不记忆

---

## 🔧 扩展指南

### 添加新工具

1. 在 `src-tauri/src/llm/tools/` 下创建新模块目录
2. 实现注册元数据（名称、描述、参数 schema、权限）
3. 在模块 `mod.rs` 中导出 `registrations()`
4. 重新编译 —— `build.rs` 自动发现并注册，无需修改中心清单

```rust
// tools/my_tool/mod.rs
pub(crate) fn registrations() -> Vec<ToolRegistration> {
    vec![ToolRegistration {
        name: "my_tool".into(),
        description: "工具描述".into(),
        parameters: serde_json::json!({ /* JSON Schema */ }),
        // ...
    }]
}
```

### 添加新 IPC 命令

1. 在 `src-tauri/src/command/` 对应模块添加 `#[tauri::command]` 函数
2. 在 `lib.rs` 的 `invoke_handler` 中注册
3. 前端通过 `invoke("command_name", { params })` 调用

### 添加新技能

在应用数据目录 `skills/` 下创建 `my-skill/SKILL.md`：

```markdown
---
name: my-skill
description: 一句话描述何时使用此技能
---

# 技能内容

具体指令、流程、模板……
```

技能目录下的附属文件（脚本、参考文档）会随技能一并暴露给 AI。

### 添加新 LLM 提供商

无需写代码：设置 → 模型 → 添加提供商，填写 `base_url`、`api_key`，选择协议（anthropic / openai / openai_responses），保存即可。支持从 `/v1/models` 端点自动拉取模型列表。

---

## 💻 系统要求

| 项目 | 要求 |
|------|------|
| 操作系统 | Windows 10+、macOS 10.15+、Linux (glibc 2.31+) |
| Node.js | 18+ |
| Rust | 1.85+ |
| Tauri CLI | 2+ |
| 磁盘空间 | ≥ 500MB（含 Rust 编译缓存） |

---

<p align="center">
  <sub>Built with Vue 3 + Tauri 2 + Rust</sub>
</p>
