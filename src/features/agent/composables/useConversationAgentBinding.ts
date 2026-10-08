import { computed, onBeforeUnmount, onMounted, ref, watch, type Ref, type ComputedRef } from "vue";
import { agentApi, type ConversationAgentMeta } from "../api/agent-api";

export type ConversationAgent = ConversationAgentMeta | null;

export interface UseConversationAgentOptions {
  activeConversationId: Ref<string> | ComputedRef<string>;
  pendingAgentBundleId: Ref<string | null> | ComputedRef<string | null>;
  clearPendingAgent: () => void;
  handleLaunchAgentConversation: (bundleId: string) => Promise<void>;
}

export function useConversationAgentBinding(options: UseConversationAgentOptions) {
  const {
    activeConversationId,
    pendingAgentBundleId,
    clearPendingAgent,
    handleLaunchAgentConversation,
  } = options;

  const conversationAgent = ref<ConversationAgent>(null);
  const pendingAgentMeta = ref<ConversationAgent>(null);

  const displayAgent = computed<ConversationAgent>(() => {
    if (conversationAgent.value) return conversationAgent.value;
    if (!pendingAgentBundleId.value) return null;
    return pendingAgentMeta.value?.id === pendingAgentBundleId.value ? pendingAgentMeta.value : null;
  });

  const refreshConversationAgent = async () => {
    const convId = activeConversationId.value?.trim();
    if (!convId) {
      conversationAgent.value = null;
      return;
    }
    try {
      conversationAgent.value = await agentApi.getConversationAgent(convId);
    } catch {
      conversationAgent.value = null;
    }
  };

  const removeConversationAgent = async () => {
    const convId = activeConversationId.value?.trim();
    if (convId) {
      try {
        await agentApi.setConversationAgent(convId, null);
        conversationAgent.value = null;
      } catch (err) {
        console.error("Failed to remove conversation agent:", err);
      }
      return;
    }
    clearPendingAgent();
  };

  const handleLaunchAgent = async (bundleId: string) => {
    try {
      pendingAgentMeta.value = await agentApi.loadAgentBundle(bundleId);
    } catch {
      pendingAgentMeta.value = null;
    }
    await handleLaunchAgentConversation(bundleId);
  };

  const onAgentBundleChanged = () => {
    void refreshConversationAgent();
  };

  watch(activeConversationId, () => {
    void refreshConversationAgent();
  });

  onMounted(() => {
    void refreshConversationAgent();
    window.addEventListener("agent-bundle-changed", onAgentBundleChanged);
  });

  onBeforeUnmount(() => {
    window.removeEventListener("agent-bundle-changed", onAgentBundleChanged);
  });

  return {
    displayAgent,
    conversationAgent,
    pendingAgentMeta,
    refreshConversationAgent,
    removeConversationAgent,
    handleLaunchAgent,
  };
}
