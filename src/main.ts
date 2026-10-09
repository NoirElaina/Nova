import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import BrowserWindowShell from "./components/chat/workspace/BrowserWindowShell.vue";
import { router } from "./router";
import "./main.css";
import { installBackendErrorToastListener, installBackendWarningToastListener, installGlobalErrorToastHandlers } from "./lib/toast";
import { applyUiTheme, getStoredUiTheme } from "./lib/ui-preferences";

applyUiTheme(getStoredUiTheme());

const params = new URLSearchParams(window.location.search);

if (params.get("novaBrowserWindow") === "1") {
  createApp(BrowserWindowShell).mount("#app");
} else {
  installGlobalErrorToastHandlers();
  void installBackendErrorToastListener();
  void installBackendWarningToastListener();
  const pinia = createPinia();
  const app = createApp(App);
  app.use(pinia);
  app.use(router);
  app.mount("#app");
}

