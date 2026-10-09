import { createApp } from "vue";
import { createPinia } from "pinia";
// KaTeX 样式（字体路径由 scripts/patch-katex-fonts.cjs 在 postinstall 阶段改写）
import "katex/dist/katex.min.css";
import App from "./App.vue";

if (import.meta.env.DEV) {
  const { installDevTauriMock } = await import("./dev-tauri-mock");
  installDevTauriMock();
}

const app = createApp(App);
app.use(createPinia());
app.mount("#app");
