<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Card } from '@/components/ui/card';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { Avatar, AvatarFallback } from '@/components/ui/avatar';
import { Separator } from '@/components/ui/separator';
import { Textarea } from '@/components/ui/textarea';
import { Input } from '@/components/ui/input';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import {
  Ban,
  Bot,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Copy,
  Hash,
  Layers,
  MessageSquare,
  Network,
  RefreshCw,
  Send,
  Sparkles,
  Terminal,
  User,
} from 'lucide-vue-next';
import {
  type AgentNode,
  type BoardChannel,
  type BoardPost,
  getAgentGraph,
  getBoardChannels,
  getBoardPosts,
  postBoardMessage,
  interruptAgent,
  sendAgentMessage,
} from '../../../features/chat/services/chat-api';
import {
  initSubagentEvents,
  selectedSubagentId,
  subagentsFor,
  type SubagentEntry,
} from '../../../features/chat/services/subagents';

const props = defineProps<{
  conversationId: string | null;
}>();

type ViewMode = 'all' | 'graph' | 'board';
const viewMode = ref<ViewMode>('all');

const loading = ref(false);
const graphNodes = ref<AgentNode[]>([]);
const channels = ref<BoardChannel[]>([]);
const activeChannel = ref<string>('general');
const posts = ref<BoardPost[]>([]);
const newPostContent = ref('');
const isSubmitting = ref(false);
const expandedNodeIds = ref<Set<string>>(new Set());
const copiedTaskId = ref<string | null>(null);
const copiedReportId = ref<string | null>(null);
const isTaskExpanded = ref(false);
const directiveInput = ref('');
const isSendingDirective = ref(false);
const isInterrupting = ref<string | null>(null);

const handleInterrupt = async (nodeId: string) => {
  if (!props.conversationId) return;
  isInterrupting.value = nodeId;
  try {
    await interruptAgent(props.conversationId, nodeId);
    const node = unifiedNodes.value.find((n) => n.id === nodeId);
    if (node) {
      node.status = 'error';
    }
  } catch (err) {
    console.error('Failed to interrupt agent:', err);
  } finally {
    isInterrupting.value = null;
  }
};

const handleSendDirective = async (nodeId: string) => {
  const trimmed = directiveInput.value.trim();
  if (!props.conversationId || !trimmed) return;
  isSendingDirective.value = true;
  try {
    await sendAgentMessage(props.conversationId, nodeId, trimmed);
    directiveInput.value = '';
  } catch (err) {
    console.error('Failed to send directive to agent:', err);
  } finally {
    isSendingDirective.value = false;
  }
};

const messageScrollRef = ref<HTMLElement | null>(null);
const traceScrollRef = ref<HTMLElement | null>(null);

let unlistenGraph: UnlistenFn | null = null;
let unlistenPost: UnlistenFn | null = null;
let unlistenChannel: UnlistenFn | null = null;
let isDisposed = false;
let loadDataSeq = 0;
let switchChannelSeq = 0;

// 从 subagents 响应式模块中获取当前会话的实时子代理
const subagents = computed<SubagentEntry[]>(() => subagentsFor(props.conversationId));

// 合并统一的节点列表：优先使用 graphNodes，如果 graphNodes 暂未同步则融入 subagents
type UnifiedNode = {
  id: string;
  role: string;
  nickname: string;
  task: string;
  status: 'running' | 'done' | 'error';
  createdAt: number;
  elapsedMs?: number;
  reportPreview?: string | null;
  detail?: string | null;
  subagentEntry?: SubagentEntry | null;
};

const unifiedNodes = computed<UnifiedNode[]>(() => {
  const result: UnifiedNode[] = [];
  const processedIds = new Set<string>();

  // 先处理图谱节点
  for (const gn of graphNodes.value) {
    processedIds.add(gn.id);
    const matchedSub = subagents.value.find(
      (s) => s.subId === gn.id || gn.id.includes(s.subId) || s.subId.includes(gn.id),
    );
    result.push({
      id: gn.id,
      role: gn.role,
      nickname: gn.nickname || 'ResearchAgent',
      task: gn.task,
      status: gn.status === 'open' ? 'running' : 'done',
      createdAt: gn.createdAt,
      elapsedMs: matchedSub?.elapsedMs,
      reportPreview: gn.resultPreview || matchedSub?.reportPreview,
      detail: matchedSub?.detail,
      subagentEntry: matchedSub ?? null,
    });
  }

  // 再融入尚未在图谱中的实时 subagent
  for (const s of subagents.value) {
    if (!processedIds.has(s.subId)) {
      result.push({
        id: s.subId,
        role: 'researcher',
        nickname: 'ResearchSubagent',
        task: s.task,
        status: s.phase === 'running' ? 'running' : s.phase === 'error' ? 'error' : 'done',
        createdAt: s.startedAt,
        elapsedMs: s.elapsedMs,
        reportPreview: s.reportPreview,
        detail: s.detail,
        subagentEntry: s,
      });
    }
  }

  return result;
});

// 当前选中的子代理条目（默认选中首个或活跃的）
const activeSelectedNodeId = ref<string | null>(null);

const currentSelectedNode = computed<UnifiedNode | null>(() => {
  if (activeSelectedNodeId.value) {
    const found = unifiedNodes.value.find((n) => n.id === activeSelectedNodeId.value);
    if (found) return found;
  }
  if (selectedSubagentId.value) {
    const found = unifiedNodes.value.find((n) => n.id === selectedSubagentId.value);
    if (found) return found;
  }
  return unifiedNodes.value[0] ?? null;
});

// 计算统计指标
const activeAgentCount = computed(() => unifiedNodes.value.filter((n) => n.status === 'running').length);
const closedAgentCount = computed(() => unifiedNodes.value.filter((n) => n.status === 'done').length);
const totalAgentCount = computed(() => unifiedNodes.value.length);

const loadData = async () => {
  const seq = ++loadDataSeq;
  const targetConversationId = props.conversationId;
  if (!targetConversationId) {
    graphNodes.value = [];
    channels.value = [];
    posts.value = [];
    loading.value = false;
    return;
  }

  loading.value = true;
  try {
    const [graphRes, channelsRes, postsRes] = await Promise.all([
      getAgentGraph(targetConversationId),
      getBoardChannels(targetConversationId),
      getBoardPosts(targetConversationId, activeChannel.value),
    ]);
    if (isDisposed || seq !== loadDataSeq || props.conversationId !== targetConversationId) {
      return;
    }
    graphNodes.value = graphRes;
    channels.value = channelsRes;
    posts.value = postsRes;
  } catch (err) {
    if (!isDisposed && seq === loadDataSeq) {
      console.warn('Failed to load collaboration data:', err);
    }
  } finally {
    if (!isDisposed && seq === loadDataSeq) {
      loading.value = false;
    }
  }
};

const switchChannel = async (channelName: string) => {
  activeChannel.value = channelName;
  const seq = ++switchChannelSeq;
  const targetConversationId = props.conversationId;
  if (!targetConversationId) return;
  try {
    const res = await getBoardPosts(targetConversationId, channelName);
    if (
      isDisposed ||
      seq !== switchChannelSeq ||
      props.conversationId !== targetConversationId ||
      activeChannel.value !== channelName
    ) {
      return;
    }
    posts.value = res;
    scrollToBottom();
  } catch (err) {
    console.warn('Failed to load posts for channel:', channelName, err);
  }
};

const scrollToBottom = () => {
  void nextTick(() => {
    if (messageScrollRef.value) {
      const scrollViewport = messageScrollRef.value.querySelector('[data-slot="scroll-area-viewport"]');
      if (scrollViewport) {
        scrollViewport.scrollTop = scrollViewport.scrollHeight;
      }
    }
  });
};

const handleSendPost = async () => {
  const content = newPostContent.value.trim();
  if (!content || !props.conversationId || isSubmitting.value) return;

  isSubmitting.value = true;
  try {
    const post = await postBoardMessage(
      props.conversationId,
      activeChannel.value,
      content,
      undefined,
      'User',
      'user',
    );
    posts.value.push(post);
    newPostContent.value = '';
    scrollToBottom();
  } catch (err) {
    console.warn('Failed to post board message:', err);
  } finally {
    isSubmitting.value = false;
  }
};

const toggleNodeExpand = (nodeId: string) => {
  const next = new Set(expandedNodeIds.value);
  if (next.has(nodeId)) {
    next.delete(nodeId);
  } else {
    next.add(nodeId);
  }
  expandedNodeIds.value = next;
};

const selectNode = (nodeId: string) => {
  activeSelectedNodeId.value = nodeId;
  selectedSubagentId.value = nodeId;
};

const copyText = async (text: string, type: 'task' | 'report', id: string) => {
  try {
    await navigator.clipboard.writeText(text);
    if (type === 'task') {
      copiedTaskId.value = id;
      setTimeout(() => { copiedTaskId.value = null; }, 1500);
    } else {
      copiedReportId.value = id;
      setTimeout(() => { copiedReportId.value = null; }, 1500);
    }
  } catch {
    // 忽略剪贴板错误
  }
};

const formatTime = (ts: number) => {
  if (!ts) return '';
  const date = new Date(ts);
  return `${date.getHours().toString().padStart(2, '0')}:${date.getMinutes().toString().padStart(2, '0')}:${date.getSeconds().toString().padStart(2, '0')}`;
};

const formatElapsed = (ms?: number) => {
  if (typeof ms !== 'number') return '';
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m${seconds % 60}s`;
};

const getInitials = (name: string) => {
  if (!name) return '?';
  const trimmed = name.trim();
  if (trimmed.length <= 2) return trimmed;
  return trimmed.slice(0, 2).toUpperCase();
};

const getRoleBadgeVariant = (role: string): 'default' | 'secondary' | 'outline' => {
  if (role === 'user') return 'secondary';
  if (role.includes('lead') || role.includes('master')) return 'default';
  return 'outline';
};

watch(
  () => props.conversationId,
  () => {
    void loadData();
    activeSelectedNodeId.value = null;
  },
  { immediate: true },
);

onMounted(() => {
  isDisposed = false;
  void initSubagentEvents();

  void listen<{ conversationId: string; node: AgentNode }>('agent-graph-updated', (event) => {
    if (props.conversationId && event.payload.node && event.payload.conversationId === props.conversationId) {
      const updated = event.payload.node;
      const idx = graphNodes.value.findIndex((n) => n.id === updated.id);
      if (idx >= 0) {
        graphNodes.value[idx] = updated;
      } else {
        graphNodes.value.push(updated);
      }
    }
  }).then((unlisten) => {
    if (isDisposed) {
      unlisten();
      return;
    }
    unlistenGraph = unlisten;
  });

  void listen<{ conversationId: string; post: BoardPost }>('board-post-created', (event) => {
    if (props.conversationId && event.payload.post && event.payload.conversationId === props.conversationId) {
      const p = event.payload.post;
      if (p.channel === activeChannel.value && !posts.value.some((x) => x.id === p.id)) {
        posts.value.push(p);
        scrollToBottom();
      }
      const chan = channels.value.find((c) => c.name === p.channel);
      if (chan) {
        chan.messageCount += 1;
      }
    }
  }).then((unlisten) => {
    if (isDisposed) {
      unlisten();
      return;
    }
    unlistenPost = unlisten;
  });

  void listen<{ conversationId: string; channel: BoardChannel }>('board-channel-created', (event) => {
    if (
      props.conversationId &&
      event.payload.channel &&
      event.payload.conversationId === props.conversationId
    ) {
      const ch = event.payload.channel;
      if (!channels.value.some((c) => c.name === ch.name)) {
        channels.value.push(ch);
      }
    }
  }).then((unlisten) => {
    if (isDisposed) {
      unlisten();
      return;
    }
    unlistenChannel = unlisten;
  });
});

onBeforeUnmount(() => {
  isDisposed = true;
  unlistenGraph?.();
  unlistenPost?.();
  unlistenChannel?.();
  unlistenGraph = null;
  unlistenPost = null;
  unlistenChannel = null;
});
</script>

<template>
  <TooltipProvider>
    <div class="flex h-full flex-col bg-background text-foreground">
      <!-- 统一顶栏 -->
      <div class="flex h-11 shrink-0 items-center justify-between border-b px-3">
        <!-- 标题与运行状态徽章 -->
        <div class="flex items-center gap-2">
          <div class="flex h-6 w-6 items-center justify-center rounded-md bg-primary/10 text-primary">
            <Network class="h-3.5 w-3.5" />
          </div>
          <span class="text-xs font-semibold">协同与智能体中心</span>
          <Badge v-if="activeAgentCount > 0" variant="outline" class="h-5 gap-1 border-emerald-500/30 bg-emerald-500/10 text-[10px] text-emerald-600 dark:text-emerald-400">
            <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-500" />
            {{ activeAgentCount }} 运行中
          </Badge>
          <Badge v-else-if="totalAgentCount > 0" variant="secondary" class="h-5 text-[10px]">
            已交付 {{ closedAgentCount }} 个
          </Badge>
          <Badge v-else variant="secondary" class="h-5 text-[10px] text-muted-foreground">
            就绪
          </Badge>
        </div>

        <!-- 视图切换与刷新 -->
        <div class="flex items-center gap-2">
          <Tabs v-model="viewMode" class="w-auto">
            <TabsList class="h-7 p-0.5 bg-muted">
              <TabsTrigger value="all" class="h-6 px-2 text-[11px] gap-1">
                <Layers class="h-3 w-3" />
                全景
              </TabsTrigger>
              <TabsTrigger value="graph" class="h-6 px-2 text-[11px] gap-1">
                <Bot class="h-3 w-3" />
                轨迹与详情
                <span v-if="totalAgentCount > 0" class="ml-0.5 opacity-70">({{ totalAgentCount }})</span>
              </TabsTrigger>
              <TabsTrigger value="board" class="h-6 px-2 text-[11px] gap-1">
                <MessageSquare class="h-3 w-3" />
                协作看板
              </TabsTrigger>
            </TabsList>
          </Tabs>

          <Separator orientation="vertical" class="h-4" />

          <Tooltip>
            <TooltipTrigger as-child>
              <Button
                variant="ghost"
                size="icon"
                class="h-7 w-7 text-muted-foreground hover:text-foreground"
                :disabled="loading"
                @click="loadData"
              >
                <RefreshCw class="h-3.5 w-3.5" :class="{ 'animate-spin': loading }" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">
              <p class="text-[11px]">刷新协同网络</p>
            </TooltipContent>
          </Tooltip>
        </div>
      </div>

      <!-- 主视图区域 -->
      <div class="flex min-h-0 flex-1 flex-col overflow-hidden">
        <!-- 模式 1: 全景视图 (上下分屏) -->
        <template v-if="viewMode === 'all'">
          <!-- 上半区：智能体集群节点及当前详情快照 -->
          <div class="flex max-h-[46%] min-h-[170px] flex-col border-b">
            <div class="flex h-7 shrink-0 items-center justify-between bg-muted/30 px-3 text-[11px] font-medium text-muted-foreground">
              <div class="flex items-center gap-1.5">
                <Bot class="h-3 w-3" />
                <span>智能体拓扑与实时记录</span>
                <span class="opacity-60">({{ totalAgentCount }})</span>
              </div>
              <div class="flex items-center gap-2 text-[10px]">
                <span v-if="closedAgentCount > 0" class="flex items-center gap-0.5">
                  <CheckCircle2 class="h-2.5 w-2.5 text-muted-foreground" />
                  交付 {{ closedAgentCount }}
                </span>
              </div>
            </div>

            <ScrollArea class="flex-1 p-2">
              <div v-if="unifiedNodes.length === 0" class="flex flex-col items-center justify-center py-6 text-center text-xs text-muted-foreground">
                <Bot class="mb-2 h-7 w-7 opacity-30" />
                <p>当前暂无子智能体参与协作</p>
                <p class="mt-0.5 text-[10px] opacity-70">
                  执行深度调研或并行任务时，派发的子智能体及其 ReAct 执行轨迹将在此实时汇聚呈现
                </p>
              </div>

              <div v-else class="space-y-2">
                <Card
                  v-for="node in unifiedNodes"
                  :key="node.id"
                  class="gap-2 rounded-lg border p-2.5 shadow-none transition-colors hover:border-foreground/20"
                  :class="activeSelectedNodeId === node.id ? 'border-primary/50 bg-primary/5' : ''"
                  @click="selectNode(node.id)"
                >
                  <div class="flex items-center justify-between">
                    <div class="flex items-center gap-1.5">
                      <Badge
                        v-if="node.status === 'running'"
                        variant="outline"
                        class="h-4 gap-1 border-emerald-500/30 bg-emerald-500/10 px-1 text-[9px] text-emerald-600 dark:text-emerald-400"
                      >
                        <span class="h-1 w-1 animate-pulse rounded-full bg-emerald-500" />
                        运行中
                        <span v-if="node.subagentEntry?.lines.length" class="opacity-80">
                          ({{ node.subagentEntry.lines.length }} 动作)
                        </span>
                      </Badge>
                      <Badge
                        v-else-if="node.status === 'done'"
                        variant="secondary"
                        class="h-4 gap-1 px-1 text-[9px]"
                      >
                        <CheckCircle2 class="h-2.5 w-2.5" />
                        已交付
                      </Badge>
                      <Badge
                        v-else
                        variant="destructive"
                        class="h-4 px-1 text-[9px]"
                      >
                        异常
                      </Badge>

                      <span class="text-xs font-semibold">{{ node.nickname }}</span>
                      <Badge variant="outline" class="h-4 px-1 text-[9px] font-mono opacity-80">
                        {{ node.role }}
                      </Badge>
                    </div>

                    <div class="flex items-center gap-2 text-[10px] text-muted-foreground">
                      <span v-if="formatElapsed(node.elapsedMs)">{{ formatElapsed(node.elapsedMs) }}</span>
                      <span>{{ formatTime(node.createdAt) }}</span>
                    </div>
                  </div>

                  <p class="text-[11px] leading-snug text-muted-foreground line-clamp-2">
                    <span class="font-medium text-foreground/80">任务:</span> {{ node.task }}
                  </p>

                  <!-- 实时动作或成果展开 -->
                  <div class="flex items-center justify-between pt-1 border-t border-muted/50">
                    <div class="flex items-center gap-1 text-[10px] text-muted-foreground">
                      <Terminal class="h-3 w-3 opacity-60" />
                      <span v-if="node.subagentEntry?.lines.length">
                        已执行 {{ node.subagentEntry.lines.length }} 条实时记录
                      </span>
                      <span v-else>等待响应</span>
                    </div>

                    <div class="flex items-center gap-1.5">
                      <Button
                        v-if="node.reportPreview"
                        variant="ghost"
                        size="sm"
                        class="h-5 px-1.5 text-[10px] text-primary"
                        @click.stop="toggleNodeExpand(node.id)"
                      >
                        <component :is="expandedNodeIds.has(node.id) ? ChevronDown : ChevronRight" class="mr-0.5 h-3 w-3" />
                        成果快照
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        class="h-5 px-1.5 text-[10px]"
                        @click.stop="selectNode(node.id); viewMode = 'graph'"
                      >
                        查看轨迹流 ›
                      </Button>
                    </div>
                  </div>

                  <!-- 展开成果报告 -->
                  <div
                    v-if="expandedNodeIds.has(node.id) && node.reportPreview"
                    class="mt-1 max-h-28 overflow-y-auto rounded bg-muted/70 p-2 font-mono text-[10px] whitespace-pre-wrap"
                  >
                    {{ node.reportPreview }}
                  </div>
                </Card>
              </div>
            </ScrollArea>
          </div>

          <!-- 下半区：协同消息看板 -->
          <div class="flex min-h-0 flex-1 flex-col">
            <!-- 频道选择条 -->
            <div class="flex h-8 shrink-0 items-center gap-1 border-b bg-muted/20 px-2">
              <Button
                v-for="ch in channels"
                :key="ch.name"
                variant="ghost"
                size="sm"
                class="h-6 gap-1 px-2 text-[11px]"
                :class="activeChannel === ch.name ? 'bg-background font-medium shadow-xs text-foreground' : 'text-muted-foreground hover:text-foreground'"
                @click="switchChannel(ch.name)"
              >
                <Hash class="h-3 w-3 opacity-60" />
                <span>{{ ch.name }}</span>
                <Badge
                  v-if="ch.messageCount > 0"
                  variant="secondary"
                  class="ml-0.5 h-3.5 px-1 text-[9px]"
                >
                  {{ ch.messageCount }}
                </Badge>
              </Button>
            </div>

            <!-- 消息流 -->
            <div ref="messageScrollRef" class="min-h-0 flex-1 overflow-hidden">
              <ScrollArea class="h-full p-3">
                <div v-if="posts.length === 0" class="flex flex-col items-center justify-center py-8 text-center text-xs text-muted-foreground">
                  <MessageSquare class="mb-2 h-7 w-7 opacity-30" />
                  <p>频道 #{{ activeChannel }} 尚无协作消息</p>
                  <p class="mt-0.5 text-[10px] opacity-70">
                    智能体自主调研发现的线索将广播至此，您也可以在下方下发实时指引
                  </p>
                </div>

                <div v-else class="space-y-2.5">
                  <Card
                    v-for="post in posts"
                    :key="post.id"
                    class="gap-1 rounded-lg border p-2.5 shadow-none"
                  >
                    <div class="flex items-center justify-between">
                      <div class="flex items-center gap-2">
                        <Avatar class="h-5 w-5 rounded-md border text-[10px]">
                          <AvatarFallback class="bg-primary/10 text-primary font-medium text-[9px]">
                            {{ getInitials(post.authorNickname) }}
                          </AvatarFallback>
                        </Avatar>
                        <span class="text-xs font-medium">{{ post.authorNickname }}</span>
                        <Badge :variant="getRoleBadgeVariant(post.authorRole)" class="h-4 px-1 text-[9px]">
                          {{ post.authorRole }}
                        </Badge>
                      </div>
                      <span class="text-[10px] text-muted-foreground">{{ formatTime(post.createdAt) }}</span>
                    </div>
                    <div class="text-xs leading-relaxed text-foreground/90 whitespace-pre-wrap pl-7">
                      {{ post.content }}
                    </div>
                  </Card>
                </div>
              </ScrollArea>
            </div>

            <!-- 底部发帖栏 -->
            <div class="border-t p-2">
              <div class="flex items-end gap-2">
                <Textarea
                  v-model="newPostContent"
                  rows="2"
                  class="min-h-[48px] resize-none text-xs"
                  :placeholder="`向 #${activeChannel} 广播协同指引 (Enter 发送)...`"
                  @keydown.enter.exact.prevent="handleSendPost"
                />
                <Button
                  size="sm"
                  class="h-8 shrink-0 px-3 text-xs"
                  :disabled="!newPostContent.trim() || isSubmitting"
                  @click="handleSendPost"
                >
                  <Send class="mr-1 h-3 w-3" />
                  广播
                </Button>
              </div>
            </div>
          </div>
        </template>

        <!-- 模式 2: 智能体图谱与详细执行轨迹 (原 SubagentPanel 的全面优雅升级) -->
        <template v-else-if="viewMode === 'graph'">
          <div class="flex h-full min-h-0 overflow-hidden">
            <!-- 左侧：智能体列表 -->
            <aside class="flex w-[230px] shrink-0 flex-col border-r bg-muted/10">
              <div class="flex h-8 shrink-0 items-center justify-between border-b px-2.5 text-[11px] font-medium text-muted-foreground">
                <span>节点列表 ({{ unifiedNodes.length }})</span>
                <span v-if="activeAgentCount > 0" class="text-emerald-600 dark:text-emerald-400 font-normal">
                  {{ activeAgentCount }} 活跃
                </span>
              </div>

              <ScrollArea class="flex-1 p-2">
                <div v-if="unifiedNodes.length === 0" class="py-8 text-center text-xs text-muted-foreground">
                  暂无节点
                </div>
                <div v-else class="space-y-1.5">
                  <button
                    v-for="node in unifiedNodes"
                    :key="node.id"
                    type="button"
                    class="w-full text-left rounded-lg border p-2 transition-colors cursor-pointer"
                    :class="currentSelectedNode?.id === node.id ? 'border-primary bg-primary/10 shadow-xs' : 'border-transparent bg-card hover:border-muted-foreground/20'"
                    @click="selectNode(node.id)"
                  >
                    <div class="flex items-center justify-between">
                      <span class="text-xs font-semibold truncate">{{ node.nickname }}</span>
                      <Badge
                        v-if="node.status === 'running'"
                        variant="outline"
                        class="h-3.5 border-emerald-500/30 bg-emerald-500/10 px-1 text-[8px] text-emerald-600 dark:text-emerald-400"
                      >
                        运行中
                      </Badge>
                      <span v-else class="text-[9px] text-muted-foreground">已完成</span>
                    </div>
                    <p class="mt-1 text-[10px] text-muted-foreground line-clamp-1">
                      {{ node.task }}
                    </p>
                    <div class="mt-1 flex items-center justify-between text-[9px] text-muted-foreground/80">
                      <span>{{ node.subagentEntry?.lines.length ?? 0 }} 动作</span>
                      <span>{{ formatElapsed(node.elapsedMs) }}</span>
                    </div>
                  </button>
                </div>
              </ScrollArea>
            </aside>

            <!-- 右侧：选中智能体的任务指令、实时轨迹与交付报告 -->
            <main class="flex min-w-0 flex-1 flex-col overflow-hidden bg-background">
              <template v-if="currentSelectedNode">
                <!-- 节点头部 -->
                <div class="flex h-10 shrink-0 items-center justify-between border-b px-3">
                  <div class="flex items-center gap-2">
                    <Avatar class="h-6 w-6 rounded-md border text-xs">
                      <AvatarFallback class="bg-primary/10 text-primary font-semibold text-[10px]">
                        <Bot class="h-3.5 w-3.5" />
                      </AvatarFallback>
                    </Avatar>
                    <span class="text-xs font-semibold">{{ currentSelectedNode.nickname }}</span>
                    <Badge variant="outline" class="h-4 px-1 text-[9px] font-mono">
                      {{ currentSelectedNode.role }}
                    </Badge>
                  </div>

                  <div class="flex items-center gap-2">
                    <Badge
                      v-if="currentSelectedNode.status === 'running'"
                      variant="outline"
                      class="h-5 gap-1 border-emerald-500/30 bg-emerald-500/10 px-1.5 text-[10px] text-emerald-600 dark:text-emerald-400"
                    >
                      <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-500" />
                      运行中
                    </Badge>
                    <Button
                      v-if="currentSelectedNode.status === 'running'"
                      variant="destructive"
                      size="sm"
                      class="h-6 px-2 text-[10px]"
                      :disabled="isInterrupting === currentSelectedNode.id"
                      @click="handleInterrupt(currentSelectedNode.id)"
                    >
                      <Ban class="mr-1 h-3 w-3" />
                      打断
                    </Button>
                    <Badge v-else variant="secondary" class="h-5 gap-1 px-1.5 text-[10px]">
                      <CheckCircle2 class="h-3 w-3" />
                      已完成
                    </Badge>
                    <span v-if="formatElapsed(currentSelectedNode.elapsedMs)" class="text-[10px] font-mono text-muted-foreground">
                      耗时 {{ formatElapsed(currentSelectedNode.elapsedMs) }}
                    </span>
                  </div>
                </div>

                <!-- 节点内容流：任务 + 轨迹 + 报告 -->
                <ScrollArea ref="traceScrollRef" class="flex-1 p-3">
                  <div class="space-y-3">
                    <!-- 1. 任务描述卡片 -->
                    <Card class="p-3 gap-2 shadow-none border bg-card">
                      <div class="flex items-center justify-between">
                        <span class="text-[11px] font-medium text-muted-foreground">调查任务 / 运行提示词</span>
                        <div class="flex items-center gap-1">
                          <Button
                            variant="ghost"
                            size="sm"
                            class="h-5 px-1.5 text-[10px]"
                            @click="isTaskExpanded = !isTaskExpanded"
                          >
                            <span>{{ isTaskExpanded ? '收起' : '展开' }}</span>
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            class="h-5 px-1.5 text-[10px]"
                            @click="copyText(currentSelectedNode.task, 'task', currentSelectedNode.id)"
                          >
                            <Check v-if="copiedTaskId === currentSelectedNode.id" class="mr-1 h-3 w-3 text-emerald-500" />
                            <Copy v-else class="mr-1 h-3 w-3" />
                            <span>{{ copiedTaskId === currentSelectedNode.id ? '已复制' : '复制' }}</span>
                          </Button>
                        </div>
                      </div>
                      <div
                        class="rounded-md bg-muted/40 p-2.5 text-xs leading-relaxed text-foreground whitespace-pre-wrap font-sans transition-all relative"
                        :class="isTaskExpanded ? '' : 'max-h-24 overflow-hidden'"
                      >
                        {{ currentSelectedNode.task }}
                        <div
                          v-if="!isTaskExpanded && currentSelectedNode.task.length > 120"
                          class="absolute inset-x-0 bottom-0 h-8 bg-gradient-to-t from-muted/90 to-transparent pointer-events-none"
                        />
                      </div>
                    </Card>

                    <!-- 2. 异常中断原因（若有） -->
                    <Card v-if="currentSelectedNode.detail" class="p-3 gap-2 shadow-none border-destructive/50 bg-destructive/10 text-destructive">
                      <div class="text-[11px] font-medium">异常中断原因</div>
                      <div class="font-mono text-xs leading-relaxed whitespace-pre-wrap">
                        {{ currentSelectedNode.detail }}
                      </div>
                    </Card>

                    <!-- 3. 最终成果报告（若已生成） -->
                    <Card v-if="currentSelectedNode.reportPreview" class="p-3 gap-2 shadow-none border border-emerald-500/30 bg-emerald-500/5">
                      <div class="flex items-center justify-between">
                        <span class="text-[11px] font-semibold text-emerald-600 dark:text-emerald-400">成果交付报告</span>
                        <Button
                          variant="outline"
                          size="sm"
                          class="h-6 px-2 text-[10px]"
                          @click="copyText(currentSelectedNode.reportPreview!, 'report', currentSelectedNode.id)"
                        >
                          <Check v-if="copiedReportId === currentSelectedNode.id" class="mr-1 h-3 w-3 text-emerald-500" />
                          <Copy v-else class="mr-1 h-3 w-3" />
                          <span>{{ copiedReportId === currentSelectedNode.id ? '已复制报告' : '复制交付报告' }}</span>
                        </Button>
                      </div>
                      <div class="rounded-md bg-background/80 p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap">
                        {{ currentSelectedNode.reportPreview }}
                      </div>
                    </Card>

                    <!-- 4. 实时执行轨迹 (流式动作与推理) -->
                    <Card class="p-3 gap-2 shadow-none border">
                      <div class="flex items-center justify-between">
                        <div class="flex items-center gap-1.5">
                          <Terminal class="h-3.5 w-3.5 text-muted-foreground" />
                          <span class="text-[11px] font-semibold">
                            执行轨迹 ({{ currentSelectedNode.subagentEntry?.lines.length ?? 0 }} 条记录)
                          </span>
                        </div>
                        <span v-if="currentSelectedNode.status === 'running'" class="text-[10px] text-emerald-600 dark:text-emerald-400 font-mono animate-pulse">
                          ● 实时流式执行中...
                        </span>
                      </div>

                      <div v-if="!currentSelectedNode.subagentEntry?.lines.length" class="py-6 text-center text-xs text-muted-foreground">
                        等待子智能体产出第一条工具动作...
                      </div>

                      <div v-else class="space-y-1.5 font-mono text-xs">
                        <div
                          v-for="(line, lIdx) in currentSelectedNode.subagentEntry.lines"
                          :key="lIdx"
                          class="rounded-md p-2"
                          :class="[
                            line.kind === 'tool' ? 'bg-primary/5 border border-primary/20 text-primary' :
                            line.kind === 'tool-result' ? 'bg-muted/70 text-foreground/90' :
                            line.kind === 'reasoning' ? 'bg-muted/30 text-muted-foreground italic' :
                            'bg-background text-foreground'
                          ]"
                        >
                          <div v-if="line.kind === 'tool'" class="flex items-center justify-between font-bold">
                            <span>▶ 调用工具: {{ line.toolName }}</span>
                            <span class="text-[10px] font-normal opacity-70">#{{ lIdx + 1 }}</span>
                          </div>
                          <div v-else-if="line.kind === 'tool-result'" class="text-[11px] whitespace-pre-wrap">
                            <span class="font-semibold text-muted-foreground">◀ 结果 [{{ line.toolName }}]: </span>
                            <span>{{ line.text }}</span>
                          </div>
                          <div v-else-if="line.kind === 'reasoning'" class="text-[11px] leading-relaxed whitespace-pre-wrap">
                            <span class="font-semibold not-italic">[思考]: </span>
                            <span>{{ line.text }}</span>
                          </div>
                          <div v-else class="text-[11px] leading-relaxed whitespace-pre-wrap">
                            {{ line.text }}
                          </div>
                        </div>
                      </div>
                    </Card>
                  </div>
                </ScrollArea>

                <!-- 运行中智能体信箱指令注入栏 (Mailbox Directives) -->
                <div v-if="currentSelectedNode.status === 'running'" class="border-t p-2.5 bg-background">
                  <div class="flex items-center gap-2">
                    <Input
                      v-model="directiveInput"
                      class="h-8 text-xs flex-1"
                      placeholder="向该智能体注入即时指令 (放入其专属 Mailbox，下一步自动响应)..."
                      :disabled="isSendingDirective"
                      @keydown.enter.prevent="handleSendDirective(currentSelectedNode.id)"
                    />
                    <Button
                      size="sm"
                      class="h-8 px-3 text-xs"
                      :disabled="!directiveInput.trim() || isSendingDirective"
                      @click="handleSendDirective(currentSelectedNode.id)"
                    >
                      <Send class="mr-1 h-3 w-3" />
                      注入指令
                    </Button>
                  </div>
                </div>
              </template>

              <div v-else class="flex h-full flex-col items-center justify-center py-16 text-center text-xs text-muted-foreground">
                <Sparkles class="mb-3 h-8 w-8 text-primary/40" />
                <p class="text-sm font-medium text-foreground">暂无活跃的子智能体</p>
                <p class="mt-1 max-w-xs text-[11px] text-muted-foreground">
                  当主模型发起子智能体协作时，所有步骤将完整在此展示。
                </p>
              </div>
            </main>
          </div>
        </template>

        <!-- 模式 3: 独立协同看板视图 (Board Mode) -->
        <template v-else-if="viewMode === 'board'">
          <div class="flex h-full flex-col">
            <!-- 频道选择条 -->
            <div class="flex h-9 shrink-0 items-center justify-between border-b bg-muted/20 px-3">
              <div class="flex items-center gap-1">
                <Button
                  v-for="ch in channels"
                  :key="ch.name"
                  variant="ghost"
                  size="sm"
                  class="h-7 gap-1.5 px-2.5 text-xs"
                  :class="activeChannel === ch.name ? 'bg-background font-medium shadow-xs text-foreground' : 'text-muted-foreground hover:text-foreground'"
                  @click="switchChannel(ch.name)"
                >
                  <Hash class="h-3.5 w-3.5 opacity-60" />
                  <span>{{ ch.name }}</span>
                  <Badge
                    v-if="ch.messageCount > 0"
                    variant="secondary"
                    class="ml-0.5 h-4 px-1.5 text-[10px]"
                  >
                    {{ ch.messageCount }}
                  </Badge>
                </Button>
              </div>

              <span class="text-[11px] text-muted-foreground">
                频道共 {{ posts.length }} 条消息
              </span>
            </div>

            <!-- 消息流 -->
            <div ref="messageScrollRef" class="min-h-0 flex-1 overflow-hidden">
              <ScrollArea class="h-full p-4">
                <div v-if="posts.length === 0" class="flex flex-col items-center justify-center py-20 text-center text-xs text-muted-foreground">
                  <MessageSquare class="mb-2 h-9 w-9 opacity-30" />
                  <p class="text-sm font-medium text-foreground">频道 #{{ activeChannel }} 尚无协作消息</p>
                  <p class="mt-1 text-xs opacity-70">
                    在下方输入框中发布消息，向分布式智能体集群下发指引或同步上下文。
                  </p>
                </div>

                <div v-else class="space-y-3">
                  <Card
                    v-for="post in posts"
                    :key="post.id"
                    class="gap-1.5 rounded-xl border p-3 shadow-none"
                  >
                    <div class="flex items-center justify-between">
                      <div class="flex items-center gap-2">
                        <Avatar class="h-6 w-6 rounded-md border text-xs">
                          <AvatarFallback class="bg-primary/10 text-primary font-medium text-[10px]">
                            <User v-if="post.authorRole === 'user'" class="h-3 w-3" />
                            <Bot v-else class="h-3 w-3" />
                          </AvatarFallback>
                        </Avatar>
                        <div class="flex items-center gap-1.5">
                          <span class="text-xs font-semibold">{{ post.authorNickname }}</span>
                          <Badge :variant="getRoleBadgeVariant(post.authorRole)" class="h-4 px-1.5 text-[9px]">
                            {{ post.authorRole }}
                          </Badge>
                        </div>
                      </div>
                      <span class="text-[10px] text-muted-foreground">{{ formatTime(post.createdAt) }}</span>
                    </div>
                    <div class="text-xs leading-relaxed text-foreground/90 whitespace-pre-wrap pl-8">
                      {{ post.content }}
                    </div>
                  </Card>
                </div>
              </ScrollArea>
            </div>

            <!-- 底部输入栏 -->
            <div class="border-t p-3">
              <div class="flex items-end gap-2">
                <Textarea
                  v-model="newPostContent"
                  rows="3"
                  class="min-h-[64px] resize-none text-xs"
                  :placeholder="`向 #${activeChannel} 广播协同指引 (Enter 发送，Shift+Enter 换行)...`"
                  @keydown.enter.exact.prevent="handleSendPost"
                />
                <Button
                  size="default"
                  class="h-10 shrink-0 px-4 text-xs"
                  :disabled="!newPostContent.trim() || isSubmitting"
                  @click="handleSendPost"
                >
                  <Send class="mr-1.5 h-3.5 w-3.5" />
                  广播
                </Button>
              </div>
            </div>
          </div>
        </template>
      </div>
    </div>
  </TooltipProvider>
</template>
