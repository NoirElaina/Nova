import { createApp } from "vue";
import App from "./App.vue";
import BrowserWindowShell from "./components/chat/workspace/BrowserWindowShell.vue";
import "./mian.css"
import { installBackendErrorToastListener, installGlobalErrorToastHandlers } from "./lib/toast";
import { applyUiTheme, getStoredUiTheme } from "./lib/ui-preferences";

applyUiTheme(getStoredUiTheme());

const params = new URLSearchParams(window.location.search);

if (params.get("novaBrowserWindow") === "1") {
  createApp(BrowserWindowShell).mount("#app");
} else {
  installGlobalErrorToastHandlers();
  void installBackendErrorToastListener();
  createApp(App).mount("#app");
}
