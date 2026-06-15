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
  <img src="https://img.shields.io/badge/License-MIT-blue" alt="License" />
</p>

---

## 📖 简介

**Nova** 是一个本地桌面 AI 编码助手系统，由 Vue 3 前端与 Tauri 2 + Rust 后端组成。专为实现可控的 AI 自动化、人工干预流程和端到端任务执行而设计。

Nova 不依赖云端执行环境 —— 所有工具执行、文件操作和终端会话都在本地完成，数据完全留在你的设备上。

---

## ✨ 特性亮点

| 类别 | 能力 |
|------|------|
| 🤖 **多模型 AI 对话** | 多轮对话、流式响应，支持 Claude / OpenAI / AWS Bedrock / 自定义提供商 |
| 🛠️ **27 个内置工具** | Shell 执行、文件补丁、Web 搜索/获取、浏览器自动化、RAG、定时任务、MCP 等 |
| 🖥️ **对话作用域工作区** | 每个对话拥有独立持久化工作区、Shell 会话、浏览器状态 |
| 👁️ **人工干预流程** | 权限申请、用户确认、审批流程，敏感操作不会静默执行 |
| 🌐 **内置浏览器（Nova Browser）** | 独立窗口，支持导航、快照、点击、输入、元素标注 |
| 🔌 **MCP 集成** | Model Context Protocol 服务注册、资源读取和工具调用 |
| 📚 **RAG 知识库** | 文档上传与检索增强生成 |
| ⏰ **定时任务** | Cron 调度，支持持久化和会话级任务 |
| 🧠 **记忆系统** | 跨会话的偏好、规则和事实记忆，自动去重与冲突清理 |
| 🖱️ **屏幕控制** | Computer Use：截图、鼠标/键盘操作 |
| 🎯 **多模式 Agent** | Agent / Plan / Auto 三种模式灵活切换 |

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
| Markdown | markdown-it + marked + mermaid |
| 图表 | echarts 6 |
| 数学公式 | KaTeX |
| 代码高亮 | highlight.js |
| 文档处理 | pdfjs-dist + docx |

### 后端

| 分类 | 技术 |
|------|------|
| 框架 | Tauri 2 (Rust 2021 Edition) |
| 异步运行时 | Tokio |
| 数据存储 | SQLite (SQLx) |
| 搜索引擎 | ripgrep (内置 rg 二进制) |
| 进程管理 | 持久化 Shell 会话 |

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
# 克隆仓库
git clone https://github.com/your-org/nova.git
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
# 构建前端资源
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
│   ├── components/               # 组件库
│   │   ├── chat/                 #   对话 UI 组件
│   │   ├── layout/               #   布局组件
│   │   ├── agent/                #   Agent 相关组件
│   │   ├── schedule/             #   定时任务组件
│   │   └── ui/                   #   基础 UI 组件 (shadcn/ui)
│   ├── features/                 # 功能模块
│   │   ├── browser/              #   Nova Browser 功能
│   │   ├── chat/                 #   对话核心逻辑
│   │   └── workspace/            #   工作区管理
│   └── lib/                      # 公共库函数
│       ├── chat-payloads.ts      #   消息载荷构造
│       ├── chat-types.ts         #   对话类型定义
│       ├── markdown-render.ts    #   Markdown 渲染
│       └── utils.ts              #   通用工具
├── src-tauri/                    # Tauri + Rust 后端
│   └── src/
│       ├── command/              # Tauri IPC 命令层（70+ 命令）
│       ├── llm/                  # LLM 核心模块
│       │   ├── tools/            #   27 个工具实现
│       │   ├── providers/        #   LLM 提供商适配
│       │   ├── services/         #   18 个服务模块
│       │   └── utils/            #   工具函数
│       ├── logging/              # 日志配置
│       └── prompt/               # 提示词模板
├── public/                       # 静态资源
└── dist/                         # 构建输出
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
┌─────────────────── Tauri IPC Bridge ────────────────────────────┐
└─────────────────────────────┬───────────────────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────────────┐
│                     Rust Backend (Tauri 2)                       │
│  ┌──────────┐  ┌───────────┐  ┌──────────┐  ┌──────────────┐  │
│  │ Commands │  │ LLM Core  │  │  Tools   │  │  Services    │  │
│  │ (IPC)    │  │(Providers)│  │ (27个)   │  │  (18个)      │  │
│  └──────────┘  └───────────┘  └──────────┘  └──────────────┘  │
│  ┌──────────┐  ┌───────────┐  ┌──────────┐                    │
│  │  SQLite  │  │  Memory   │  │  MCP     │                    │
│  │ (SQLx)   │  │  System   │  │  Client  │                    │
│  └──────────┘  └───────────┘  └──────────┘                    │
└─────────────────────────────────────────────────────────────────┘
```

### 数据流

```
用户消息 → 上下文组装（历史 + RAG + MCP + 记忆）→ LLM 调用
    → 工具执行循环 → 状态持久化 → Tauri Events 实时推送 → 前端更新
```

### 通信方式

| 方向 | 机制 | 用途 |
|------|------|------|
| 前端 → 后端 | Tauri IPC (invoke) | 命令调用、配置读写 |
| 后端 → 前端 | Tauri Events (emit) | 流式响应推送、状态变更通知 |

---

## 🧩 核心模块说明

### LLM 提供商 (`src-tauri/src/llm/providers/`)

支持多种 LLM 提供商，统一流式事件规范化：

| 提供商 | 说明 |
|--------|------|
| Claude (Anthropic) | 主要适配，完整支持 tool_use 协议 |
| OpenAI | 兼容 OpenAI API 格式 |
| AWS Bedrock | Amazon 托管模型接入 |
| Custom | 自定义 API 端点适配 |

### 工具系统 (`src-tauri/src/llm/tools/`)

模块化工具注册机制，每个工具自描述注册元数据：

| 类别 | 工具 |
|------|------|
| 终端 | `execute_bash` · `execute_powershell` · `reset_shell_session` |
| 文件编辑 | `apply_patch` |
| 搜索 | `web_search` · `web_fetch` · rg (via bash) |
| 浏览器 | `nova_browser_navigate` · `nova_browser_click` · `nova_browser_type` · `nova_browser_snapshot` · `nova_browser_reset` |
| 桌面控制 | `computer_use` |
| 规划 | `enter_plan_mode` · `exit_plan_mode` · `plan_for_approval` |
| 目标管理 | `create_goal` · `update_goal` · `get_goal` |
| 定时任务 | `CronCreate` · `CronDelete` · `CronList` |
| 知识库 | `rag_tool` · `remember_global_memory` |
| 技能 | `Skill` |
| MCP | `mcp_auth` · `list_mcp_resources` · `read_mcp_resource` |
| 配置 | `config_tool` |
| 其他 | `ask_user_question` · `tool_search` · `Sleep` |

### 记忆系统

- **存储方式**：文件备份，位于应用数据目录 `memory/`
- **记忆分类**：`preference`（偏好）、`rule`（规则）、`fact`（事实）
- **检索策略**：查询感知，注入持久规则/偏好 + 当前请求相关事实
- **维护机制**：自动去重、冲突清理，新规则替换过时重复项

### 定时任务

- **调度格式**：标准 5 字段 Cron（分 时 日 月 周）
- **存储模式**：`session`（内存级）/ `durable`（持久化至 `scheduled_tasks.json`）
- **会话绑定**：每个任务自动创建并绑定专用对话

---

## 🔧 扩展指南

### 添加新工具

1. 在 `src-tauri/src/llm/tools/` 下创建新模块文件
2. 实现工具注册元数据（名称、描述、参数 schema、权限）
3. 在 `src-tauri/src/llm/tools/mod.rs` 中注册该工具
4. 工具将自动对 LLM 可用，无需修改全局 `match` 分支

```rust
// 示例：工具模块结构
pub struct MyTool;

impl MyTool {
    pub fn registration() -> ToolRegistration {
        ToolRegistration {
            name: "my_tool",
            description: "工具描述",
            parameters: serde_json::json!({ /* JSON Schema */ }),
            permissions: vec![Permission::ReadFile],
        }
    }

    pub async fn execute(params: Value) -> Result<ToolResult> {
        // 工具逻辑
    }
}
```

### 添加新 IPC 命令

1. 在 `src-tauri/src/command/` 下添加命令函数
2. 使用 `#[tauri::command]` 宏标注
3. 在 `lib.rs` 的 `invoke_handler` 中注册
4. 前端通过 `invoke("command_name", { params })` 调用

### 添加新 LLM 提供商

1. 在 `src-tauri/src/llm/providers/` 下创建提供商模块
2. 实现请求构造和流式响应解析
3. 将流事件规范化为共享 `Delta` 类型（通过 `stream_runner.rs`）
4. 在提供商注册表中添加入口

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

## 📄 License

[MIT](LICENSE)

---

<p align="center">
  <sub>Built with ❤️ using Vue 3 + Tauri 2 + Rust</sub>
</p>
