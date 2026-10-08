import { defineStore } from "pinia";
import { ref } from "vue";
import {
  getStoredSidebarWidth,
  setStoredSidebarWidth,
  SIDEBAR_MIN_WIDTH,
  SIDEBAR_MAX_WIDTH,
  getStoredDrawerWidth,
  setStoredDrawerWidth,
  clampDrawerWidth,
} from "@/lib/ui-preferences";

export type WorkspaceTabId = "files" | "diff" | "terminal" | "browser" | "trace";

export const useLayoutStore = defineStore("layout", () => {
  const isDrawerOpen = ref(false);
  const activeWorkspaceTab = ref<WorkspaceTabId>("files");
  const isPlanPanelOpen = ref(false);
  const isBgJobsPanelOpen = ref(false);

  const sidebarWidth = ref(getStoredSidebarWidth());
  const drawerWidth = ref(getStoredDrawerWidth());

  const handleSidebarResize = (width: number) => {
    sidebarWidth.value = Math.min(Math.max(width, SIDEBAR_MIN_WIDTH), SIDEBAR_MAX_WIDTH);
  };

  const handleSidebarResizeEnd = () => {
    setStoredSidebarWidth(sidebarWidth.value);
  };

  const handleDrawerResize = (width: number) => {
    drawerWidth.value = clampDrawerWidth(width);
  };

  const handleDrawerResizeEnd = () => {
    setStoredDrawerWidth(drawerWidth.value);
  };

  return {
    isDrawerOpen,
    activeWorkspaceTab,
    isPlanPanelOpen,
    isBgJobsPanelOpen,
    sidebarWidth,
    drawerWidth,
    handleSidebarResize,
    handleSidebarResizeEnd,
    handleDrawerResize,
    handleDrawerResizeEnd,
  };
});
