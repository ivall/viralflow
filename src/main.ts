import { createPinia } from 'pinia'
import { createApp } from 'vue'
import { createRouter, createWebHashHistory } from 'vue-router'
import App from './App.vue'
import './styles/main.css'

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', name: 'home', component: () => import('./views/HomeView.vue') },
    { path: '/editor/:id', name: 'editor', component: () => import('./views/EditorView.vue'), props: true },
    { path: '/settings', name: 'settings', component: () => import('./views/SettingsView.vue') }
  ]
})

// The webview's own context menu (Reload, Inspect…) makes no sense in a desktop editor.
window.addEventListener('contextmenu', event => {
  const target = event.target as HTMLElement | null
  if (!target?.closest('input, textarea, [contenteditable="true"]')) event.preventDefault()
})

createApp(App).use(createPinia()).use(router).mount('#app')
