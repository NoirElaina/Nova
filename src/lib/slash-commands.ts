import type { WorkspaceDiff } from '../features/chat/services/chat-api';

// 命令类型：local=直接执行本地动作；prompt=构造模板消息发送给 AI；skill=触发 SkillTool；
export type SlashCommandType = 'local' | 'prompt' | 'skill';

// 参数模式：options=从二级选项列表选择；free=自由文本参数；none=无参（选中即执行）
export type SlashCommandArgs = 'options' | 'free' | 'none';

export type SlashCommandEntry = {
  name: string;
  description: string;
  type: SlashCommandType;
  args: SlashCommandArgs;
};

// 二级选项：每个命令在 param 阶段展示的候选项
export type SlashParamOption = {
  label: string;
  description?: string;
  value: string;
};

// 内置斜杠命令列表。所有命令都有二级匹配项（args=options）。
export const SLASH_COMMANDS: SlashCommandEntry[] = [
  { name: 'skill', description: '使用指定技能', type: 'skill', args: 'options' },
  { name: 'compact', description: '压缩当前对话', type: 'local', args: 'none' },
  { name: 'memory', description: '查看全局记忆', type: 'local', args: 'options' },
  { name: 'review', description: '审查工作区改动', type: 'prompt', args: 'options' },
  { name: 'init', description: '生成 AGENTS.md 项目说明', type: 'prompt', args: 'options' },
  { name: 'agent', description: '创建智能体（AI 生成配置）', type: 'prompt', args: 'options' },
];

// /skill 二级选项中的"创建新技能"标记值（区别于已有技能名）
export const SKILL_CREATE_VALUE = '__create__';

// 全量命令列表
export const allSlashCommands = (): SlashCommandEntry[] => SLASH_COMMANDS;

// 静态二级选项
export const MEMORY_OPTIONS: SlashParamOption[] = [
  { label: '查看', value: 'view', description: '展示全局记忆条目' },
  { label: '清空', value: 'clear', description: '清除所有全局记忆' },
];

export const REVIEW_OPTIONS: SlashParamOption[] = [
  { label: '未提交改动', value: 'uncommitted', description: '审查当前工作区 diff' },
  { label: '全部改动', value: 'all', description: '审查所有文件（含未跟踪）' },
];

export const INIT_OPTIONS: SlashParamOption[] = [
  { label: '标准', value: 'standard', description: '完整项目说明，含命令/架构/陷阱' },
  { label: '精简', value: 'minimal', description: '仅保留最关键的命令和约束' },
];

export const AGENT_OPTIONS: SlashParamOption[] = [
  { label: '创建智能体', value: 'create', description: '由 AI 采访需求并生成智能体配置' },
  { label: '从当前对话提炼', value: 'extract', description: '把本次对话的工作流沉淀为智能体' },
];

// 解析输入中的斜杠命令。返回命令条目和参数尾部（rest）。
// 匹配范围：内置命令。
export const parseSlashCommand = (text: string): { entry: SlashCommandEntry; rest: string } | null => {
  const trimmed = text.trim();
  if (!trimmed.startsWith('/')) return null;
  const firstSpace = trimmed.indexOf(' ');
  const cmdName = (firstSpace === -1 ? trimmed.slice(1) : trimmed.slice(1, firstSpace)).toLowerCase();
  const entry = allSlashCommands().find((cmd) => cmd.name.toLowerCase() === cmdName);
  if (!entry) return null;
  const rest = firstSpace === -1 ? '' : trimmed.slice(firstSpace + 1).trim();
  return { entry, rest };
};

// /init 模板：style 为 standard / minimal
export const buildInitPrompt = (style: string): string => {
  const isMinimal = style === 'minimal';
  const depthSection = isMinimal
    ? `## 风格：精简
仅保留最关键的命令和硬约束，跳过架构详解。目标是一页速查表。`
    : `## 风格：标准
完整覆盖命令、架构、陷阱，但每条仍需精炼。`;

  return `请扫描当前工作区，生成或更新 AGENTS.md 项目说明文件。

目标：产出一份紧凑的指令文件，帮助后续 AI 会话快速理解项目、避免常见错误。每一条都应回答："没有这条帮助，AI 是否容易踩坑？" 若否，则不写入。

${depthSection}

## 调查顺序
优先读取高价值信息源：
- README、根目录 manifest、workspace 配置、lockfile
- build/test/lint/format/typecheck/codegen 配置
- CI 工作流、pre-commit 配置
- 已有指令文件（AGENTS.md、CLAUDE.md、.cursor/rules 等）

若配置和文档不足以理解架构，再抽样少量代表性代码文件确认入口和包边界。

## 应提取的内容
- 精确的开发命令（尤其是非显而易见的）
- 如何运行单个测试、单个包、聚焦验证步骤
- 必要的命令顺序（如 lint -> typecheck -> test）
- monorepo/多包边界、目录归属、真实入口
- 框架/工具链陷阱：生成代码、迁移、codegen、构建产物、env 加载、dev server
- 测试陷阱：fixtures、集成测试前置条件、snapshot 流程
- 值得保留的现有指令文件中的关键约束

## 排除
- 通用软件建议
- 冗长教程或完整文件树
- 显而易见的语言约定
- 推测性内容

若 AGENTS.md 已存在则就地改进，保留已验证的有用指导，删除过时或冗余内容。

写入位置：项目根目录 AGENTS.md。`;
};

// /review 模板：scope 为范围描述
export const buildReviewPrompt = (scope: string): string => {
  return `请审查当前工作区的代码改动${scope}。

## 审查要点
- 正确性：逻辑错误、边界条件、潜在 bug
- 安全性：注入、权限、敏感信息泄漏
- 性能：不必要的复杂度、N+1、资源泄漏
- 可维护性：命名、抽象层次、重复代码
- 测试：是否需要补充测试

## 改动统计
见下方 diff。请逐文件给出审查意见，最后给出总体评价和优先级建议（阻断/建议/可选）。`;
};

// 把 WorkspaceDiff 格式化为可读文本
export const formatWorkspaceDiff = (diff: WorkspaceDiff): string => {
  if (diff.files.length === 0) {
    return '（工作区无改动）';
  }
  const lines: string[] = [];
  lines.push(`共 ${diff.files.length} 个文件改动（+${diff.totalAdditions} -${diff.totalDeletions}）`);
  for (const file of diff.files) {
    lines.push(`\n--- ${file.path} (${file.changeType}, +${file.additions} -${file.deletions}) ---`);
    for (const line of file.diff) {
      const prefix = line.kind === 'add' ? '+' : line.kind === 'remove' ? '-' : ' ';
      lines.push(`${prefix}${line.text}`);
    }
  }
  return lines.join('\n');
};

// ---------------- /agent 创建智能体模板 ----------------
export const buildCreateAgentPrompt = (appDataDir: string, mode: string): string => {
  const extractSection =
    mode === 'extract'
      ? `## 模式：从当前对话提炼
先回顾本对话中已建立的工作流（我的偏好、常用操作、约定），把它沉淀为智能体的提示词与能力清单，再向我确认。`
      : `## 模式：新建智能体`;

  return `请帮我创建一个 Nova 智能体（Agent Bundle）。

## 存储结构（直接用 Write 工具写文件，写完即可被识别）
- 目录：${appDataDir}\\agents\\<agent-id>\\agent.json
- agent-id：格式如 agent-1a2b3c-4d5e6f（仅字母数字、-、_，需唯一，不要与已有目录重名）
- agent.json 字段（camelCase）：
  - id / name / description：基本信息的字符串
  - prompt：智能体系统提示词（markdown）。非空时完整替换默认提示词，是这个智能体的灵魂，认真写。
  - enabledTools / enabledSkills / enabledMcpServers：null = 全部可用；字符串数组 = 仅勾选项
  - createdAt / updatedAt：Unix 秒时间戳
- 可选私有内容（同目录下）：
  - skills\\<技能名>\\SKILL.md：该智能体专属技能
  - files\\：参考资料目录，智能体使用时 AI 可按绝对路径读取
  - mcp.json：该智能体专属 MCP server 配置

${extractSection}

## 流程
1. 先采访我：智能体叫什么、解决什么问题、角色设定和行为约束、需要哪些工具/技能/MCP（还是全要）。
2. 写入 agent.json（提示词按采访结果认真起草，不要敷衍）。
3. 需要私有技能或资料时一并创建，并向我说明内容。
4. 完成后告诉我：到智能体页刷新即可看到，点「启用」新开对话使用。`;
};

// ---------------- /skill 创建技能模板（/skill 二级选项"创建新技能"） ----------------
export const buildCreateSkillPrompt = (appDataDir: string): string => {
  return `请帮我创建一个 Nova 技能。

## 存储结构（直接用 Write 工具写文件，写完即可被识别）
- 路径：${appDataDir}\\skills\\<技能名>\\SKILL.md
- 技能名：小写英文与连字符（如 pdf-report），是调用时的标识
- 文件格式：YAML frontmatter + Markdown 正文

\`\`\`markdown
---
name: 技能名
description: 一句话说明何时使用该技能
---
（正文：写给 AI 执行的操作指南）
\`\`\`

## 正文写法（决定技能质量）
- 面向执行者：分步骤写清"何时用、先做什么、怎么做、注意什么"，AI 会照此执行。
- 具体可操作：给出确切的命令、参数、文件路径、判断标准，避免空泛描述。
- 只写这个技能该做的事，不重复通用常识。

## 流程
1. 先采访我：技能解决什么问题、什么场景触发、期望的输出。
2. 起草 SKILL.md 内容给我过目（或直接写好后展示全文）。
3. 用 Write 工具写入，完成后告诉我可在设置的技能页查看，对话中用 /skill 调用。`;
};
