import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import AutoImport from 'unplugin-auto-import/vite'
import Components from 'unplugin-vue-components/vite'
import { ElementPlusResolver } from 'unplugin-vue-components/resolvers'

// Tauri 开发建议：固定端口（与 tauri.conf.json 的 devUrl 保持一致），
// 并让 Vite 忽略 src-tauri，避免 Rust 编译产物触发前端热更新。
export default defineConfig({
  plugins: [
    vue(),
    AutoImport({
      resolvers: [ElementPlusResolver()],
      dts: 'src/auto-imports.d.ts',
    }),
    Components({
      resolvers: [ElementPlusResolver()],
      dts: 'src/components.d.ts',
    }),
  ],
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    // macOS 11+ 的 WKWebView 对 ES2021 支持完整；Windows 侧 WebView2 为 Chromium，无需降级到 ES5。
    target: 'es2021',
    // Vite 8 默认使用 rolldown/oxc 压缩，勿显式设成 'esbuild'（会额外要求安装 esbuild）。
    sourcemap: false,
  },
})
