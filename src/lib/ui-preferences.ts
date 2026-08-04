export type UiLanguage = "zh-CN" | "en-US";
export type UiTheme = "system" | "light" | "dark";

const UI_LANGUAGE_STORAGE_KEY = "nova.ui.language";
const UI_THEME_STORAGE_KEY = "nova.ui.theme";
const SIDEBAR_WIDTH_STORAGE_KEY = "nova.ui.sidebarWidth";
const DRAWER_WIDTH_STORAGE_KEY = "nova.ui.drawerWidth";

/** 侧边栏宽度限制（px） */
export const SIDEBAR_MIN_WIDTH = 140;
export const SIDEBAR_MAX_WIDTH = 180;
export const SIDEBAR_DEFAULT_WIDTH = 160;

/** 工作区抽屉宽度限制（px）；默认取窗口宽度的 48%，限制在合理区间。 */
export const DRAWER_MIN_WIDTH = 360;
export const DRAWER_MAX_RATIO = 0.8;

export function getDrawerDefaultWidth(): number {
  if (typeof window === "undefined") {
    return 720;
  }
  return Math.round(
    Math.min(Math.max(window.innerWidth * 0.48, DRAWER_MIN_WIDTH), window.innerWidth * DRAWER_MAX_RATIO),
  );
}

export function clampDrawerWidth(width: number): number {
  const max = typeof window === "undefined" ? 1400 : Math.max(DRAWER_MIN_WIDTH, window.innerWidth * DRAWER_MAX_RATIO);
  return Math.round(Math.min(Math.max(width, DRAWER_MIN_WIDTH), max));
}

export function getStoredDrawerWidth(): number {
  const fallback = getDrawerDefaultWidth();
  if (typeof window === "undefined") {
    return fallback;
  }
  const raw = window.localStorage.getItem(DRAWER_WIDTH_STORAGE_KEY);
  const parsed = Number.parseInt(raw ?? "", 10);
  if (!Number.isFinite(parsed)) {
    return fallback;
  }
  return clampDrawerWidth(parsed);
}

export function setStoredDrawerWidth(width: number) {
  if (typeof window === "undefined") {
    return;
  }
  window.localStorage.setItem(DRAWER_WIDTH_STORAGE_KEY, String(clampDrawerWidth(width)));
}

export function getStoredSidebarWidth(): number {
  if (typeof window === "undefined") {
    return SIDEBAR_DEFAULT_WIDTH;
  }
  const raw = window.localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY);
  const parsed = Number.parseInt(raw ?? "", 10);
  if (!Number.isFinite(parsed)) {
    return SIDEBAR_DEFAULT_WIDTH;
  }
  return Math.min(Math.max(parsed, SIDEBAR_MIN_WIDTH), SIDEBAR_MAX_WIDTH);
}

export function setStoredSidebarWidth(width: number) {
  if (typeof window === "undefined") {
    return;
  }
  const clamped = Math.min(Math.max(Math.round(width), SIDEBAR_MIN_WIDTH), SIDEBAR_MAX_WIDTH);
  window.localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(clamped));
}

export function normalizeUiLanguage(value: unknown): UiLanguage {
  const raw = typeof value === "string" ? value.trim().toLowerCase() : "";
  if (raw === "en" || raw === "en-us" || raw === "english") {
    return "en-US";
  }
  return "zh-CN";
}

export function normalizeUiTheme(value: unknown): UiTheme {
  const raw = typeof value === "string" ? value.trim().toLowerCase() : "";
  if (raw === "light" || raw === "dark") {
    return raw;
  }
  return "system";
}

export function getStoredUiLanguage(): UiLanguage {
  if (typeof window === "undefined") {
    return "zh-CN";
  }
  return normalizeUiLanguage(window.localStorage.getItem(UI_LANGUAGE_STORAGE_KEY));
}

export function setStoredUiLanguage(language: UiLanguage) {
  if (typeof window === "undefined") {
    return;
  }
  window.localStorage.setItem(UI_LANGUAGE_STORAGE_KEY, language);
}

export function getStoredUiTheme(): UiTheme {
  if (typeof window === "undefined") {
    return "system";
  }
  return normalizeUiTheme(window.localStorage.getItem(UI_THEME_STORAGE_KEY));
}

export function setStoredUiTheme(theme: UiTheme) {
  if (typeof window === "undefined") {
    return;
  }
  window.localStorage.setItem(UI_THEME_STORAGE_KEY, theme);
}

export function applyUiTheme(theme: UiTheme) {
  if (typeof document === "undefined") {
    return;
  }

  const prefersDark =
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches;

  const shouldUseDark = theme === "dark" || (theme === "system" && prefersDark);
  document.documentElement.classList.toggle("dark", shouldUseDark);
}
