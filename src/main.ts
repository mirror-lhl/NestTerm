import { createApp } from 'vue'
import 'element-plus/theme-chalk/base.css'
import 'element-plus/theme-chalk/dark/css-vars.css'
// 程序式调用（ElMessage / ElMessageBox）不会被模板解析器自动注入样式，显式引入。
import 'element-plus/es/components/message/style/css'
import 'element-plus/es/components/message-box/style/css'
import '@xterm/xterm/css/xterm.css'
import './styles.css'
import App from './App.vue'

createApp(App).mount('#app')
