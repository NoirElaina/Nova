import { createRouter, createWebHashHistory } from "vue-router";
import MainLayout from "@/views/MainLayout.vue";
import ChatView from "@/views/ChatView.vue";
import { useConversationStore, type MainView } from "@/stores/conversation";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    {
      path: "/",
      component: MainLayout,
      redirect: "/chat",
      children: [
        {
          path: "chat/:conversationId?",
          name: "chat",
          component: ChatView,
        },
        {
          path: "agent",
          name: "agent",
          component: () => import("@/views/AgentView.vue"),
        },
        {
          path: "schedule",
          name: "schedule",
          component: () => import("@/views/ScheduleView.vue"),
        },
        {
          path: "hooks",
          name: "hooks",
          component: () => import("@/views/HooksView.vue"),
        },
      ],
    },
    {
      path: "/settings",
      name: "settings",
      component: () => import("@/views/SettingsView.vue"),
    },
  ],
});

router.afterEach((to) => {
  try {
    const conversationStore = useConversationStore();
    const routeName = (to.name as string) || "chat";
    if (["chat", "hooks", "agent", "schedule", "settings"].includes(routeName)) {
      conversationStore.setMainView(routeName as MainView);
    }
  } catch {
    // 忽略在 pinia 未初始化前测试运行的边界情况
  }
});

