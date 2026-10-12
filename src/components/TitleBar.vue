<script setup lang="ts">
import { getCurrentWindow } from '@tauri-apps/api/window'
import { onBeforeUnmount, onMounted, ref } from 'vue'

/**
 * Custom window title bar. Views put their own controls into it through
 * `<Teleport defer to="#titlebar-left|center|right">`. Window buttons are only
 * drawn on Windows; other platforms keep their native frame.
 */
const windowsChrome = navigator.userAgent.includes('Windows')
const maximized = ref(false)
let unlisten: (() => void) | null = null

const appWindow = (() => {
  try { return getCurrentWindow() } catch { return null }
})()

async function syncMaximized() {
  maximized.value = await appWindow?.isMaximized().catch(() => false) ?? false
}

onMounted(async () => {
  if (!windowsChrome || !appWindow) return
  await syncMaximized()
  unlisten = await appWindow.onResized(() => void syncMaximized()).catch(() => null)
})

onBeforeUnmount(() => unlisten?.())
</script>

<template>
  <header class="relative z-[90] flex h-11 shrink-0 select-none items-center bg-app" data-tauri-drag-region>
    <div class="flex h-full min-w-0 flex-1 basis-0 items-center gap-2 pl-3" data-tauri-drag-region>
      <img src="/app-icon.png" alt="" class="pointer-events-none size-[18px] rounded" draggable="false">
      <div id="titlebar-left" class="flex min-w-0 items-center gap-1" />
    </div>
    <div id="titlebar-center" class="flex h-full min-w-0 items-center justify-center" data-tauri-drag-region />
    <div class="flex h-full min-w-0 flex-1 basis-0 items-center justify-end" data-tauri-drag-region>
      <div id="titlebar-right" class="flex items-center gap-1.5 pr-2" />
      <div v-if="windowsChrome" class="flex h-full">
        <button class="grid h-full w-[46px] place-items-center text-muted transition-colors hover:bg-white/[.07] hover:text-fg" aria-label="Minimize" @click="appWindow?.minimize()">
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" stroke="currentColor" stroke-width="1" /></svg>
        </button>
        <button class="grid h-full w-[46px] place-items-center text-muted transition-colors hover:bg-white/[.07] hover:text-fg" :aria-label="maximized ? 'Restore' : 'Maximize'" @click="appWindow?.toggleMaximize()">
          <svg v-if="maximized" width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><path d="M2.5 2.5V.5h7v7h-2" /><rect x=".5" y="2.5" width="7" height="7" /></svg>
          <svg v-else width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><rect x=".5" y=".5" width="9" height="9" /></svg>
        </button>
        <button class="grid h-full w-[46px] place-items-center text-muted transition-colors hover:bg-[#c42b1c] hover:text-white" aria-label="Close" @click="appWindow?.close()">
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" stroke-width="1" /></svg>
        </button>
      </div>
    </div>
  </header>
</template>
