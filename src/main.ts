import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";

if (import.meta.env.DEV) {
  const { installDevTauriMock } = await import("./dev-tauri-mock");
  installDevTauriMock();
}

const app = createApp(App);
app.use(createPinia());
app.mount("#app");
