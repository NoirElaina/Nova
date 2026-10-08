import { IpcGateway } from "@/core/ipc/gateway";

export interface ConversationAgentMeta {
  id: string;
  name: string;
  description?: string;
}

export const agentApi = {
  getConversationAgent(conversationId: string): Promise<ConversationAgentMeta | null> {
    return IpcGateway.call<ConversationAgentMeta | null>("get_conversation_agent", {
      conversationId,
    });
  },

  setConversationAgent(
    conversationId: string,
    bundleId: string | null,
  ): Promise<void> {
    return IpcGateway.call<void>("set_conversation_agent", {
      conversationId,
      bundleId,
    });
  },

  loadAgentBundle(bundleId: string): Promise<ConversationAgentMeta | null> {
    return IpcGateway.call<ConversationAgentMeta | null>("load_agent_bundle", {
      bundleId,
    });
  },
};
