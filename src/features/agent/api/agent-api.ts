import { invoke } from "@tauri-apps/api/core";

export interface ConversationAgentMeta {
  id: string;
  name: string;
  description?: string;
}

export const agentApi = {
  getConversationAgent(conversationId: string): Promise<ConversationAgentMeta | null> {
    return invoke<ConversationAgentMeta | null>("get_conversation_agent", {
      conversationId,
    });
  },

  setConversationAgent(
    conversationId: string,
    bundleId: string | null,
  ): Promise<void> {
    return invoke<void>("set_conversation_agent", {
      conversationId,
      bundleId,
    });
  },

  loadAgentBundle(bundleId: string): Promise<ConversationAgentMeta | null> {
    return invoke<ConversationAgentMeta | null>("load_agent_bundle", {
      bundleId,
    });
  },
};
