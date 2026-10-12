<script setup lang="ts">
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { save as saveDialog } from '@tauri-apps/plugin-dialog'
import { ArrowLeft, Check, CloudAlert, Download, FileDown, Keyboard, LoaderCircle, MoreHorizontal, Settings, Upload } from '@lucide/vue'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { onBeforeRouteLeave, useRouter } from 'vue-router'
import AssetPanel from '@/components/editor/AssetPanel.vue'
import ExportDialog from '@/components/editor/ExportDialog.vue'
import Inspector from '@/components/editor/Inspector.vue'
import Player from '@/components/editor/Player.vue'
import TimelineView from '@/components/editor/timeline/TimelineView.vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiDialog from '@/components/ui/UiDialog.vue'
import UiMenu from '@/components/ui/UiMenu.vue'
import type { MenuItem } from '@/components/ui/menu'
import { drag, dropAt, setExternalDrag } from '@/lib/dnd'
import { SHORTCUTS } from '@/lib/shortcuts'
import { errorMessage, ipc } from '@/services/ipc'
import { importIntoProject, pickMediaFiles } from '@/services/media'
import { useEditor } from '@/stores/editor'
import { useToasts } from '@/stores/toast'

const props = defineProps<{ id: string }>()
const router = useRouter()
const editor = useEditor()
const toasts = useToasts()
const loading = ref(true)
const loadError = ref('')
const panel = ref('media')
const captionSource = ref<string | null>(null)
const showExport = ref(false)
const showShortcuts = ref(false)
const timelineHeight = ref(Math.round(window.innerHeight * 0.36))
const editingName = ref(false)
const nameDraft = ref('')
const inspector = ref<InstanceType<typeof Inspector> | null>(null)
const timeline = ref<InstanceType<typeof TimelineView> | null>(null)
let unlistenDrop: (() => void) | null = null
let unlistenClose: (() => void) | null = null

const moreItems: MenuItem[] = [
  { id: 'import', label: 'Import media…', icon: Upload, shortcut: 'Ctrl+I' },
  { id: 'save-file', label: 'Save project file…', icon: FileDown },
  { id: 'shortcuts', label: 'Keyboard shortcuts', icon: Keyboard },
  { id: 'settings', label: 'Settings', icon: Settings, separatorBefore: true }
]

const saveLabel = computed(() => ({ saved: 'Saved', saving: 'Saving…', dirty: 'Unsaved changes', error: 'Not saved' })[editor.saveState])

async function load() {
  loading.value = true
  loadError.value = ''
  try {
    await editor.open(props.id)
    await getCurrentWindow().setTitle(`${editor.project?.name} - ViralFlow`).catch(() => undefined)
    await nextTick()
    timeline.value?.zoomToFit()
  } catch (error) {
    loadError.value = errorMessage(error)
  } finally {
    loading.value = false
  }
}

watch(() => props.id, load)
watch(() => editor.project?.name, name => { if (name) void getCurrentWindow().setTitle(`${name} - ViralFlow`).catch(() => undefined) })

function startRename() {
  nameDraft.value = editor.project?.name ?? ''
  editingName.value = true
  void nextTick(() => document.querySelector<HTMLInputElement>('[data-project-name]')?.select())
}

function finishRename(save: boolean) {
  if (save) editor.rename(nameDraft.value)
  editingName.value = false
}

async function importMedia() {
  const assets = await importIntoProject(await pickMediaFiles())
  if (assets.length) toasts.success(assets.length === 1 ? `${assets[0].name} imported` : `${assets.length} files imported`)
}

async function onMore(id: string) {
  if (id === 'import') await importMedia()
  else if (id === 'shortcuts') showShortcuts.value = true
  else if (id === 'settings') await router.push({ name: 'settings' })
  else if (id === 'save-file' && editor.project) {
    const path = await saveDialog({ title: 'Save project file', defaultPath: `${editor.project.name}.vfproj`, filters: [{ name: 'ViralFlow project', extensions: ['vfproj'] }] })
    if (!path) return
    try {
      await ipc.writeProjectFile(path, JSON.parse(JSON.stringify(editor.project)))
      toasts.success('Project file saved')
    } catch (error) {
      toasts.error('Project file could not be saved', errorMessage(error))
    }
  }
}

function openCaptions(itemId: string) {
  captionSource.value = itemId
  panel.value = 'captions'
}

function focusText() {
  void nextTick(() => inspector.value?.focusText())
}

// ---------------------------------------------------------------- keyboard

function onKeydown(event: KeyboardEvent) {
  if (!editor.project || showExport.value || showShortcuts.value) return
  const target = event.target as HTMLElement
  if (target.closest('input, textarea, select, [contenteditable="true"], [role="slider"], [role="combobox"], [role="listbox"], [role="menu"], [role="dialog"]')) return
  const mod = event.ctrlKey || event.metaKey
  const key = event.key.toLowerCase()
  const handled = () => event.preventDefault()

  if (mod) {
    if (key === 'z') { handled(); event.shiftKey ? editor.redo() : editor.undo() }
    else if (key === 'y') { handled(); editor.redo() }
    else if (key === 'c') { handled(); editor.copySelected() }
    else if (key === 'x') { handled(); editor.cutSelected() }
    else if (key === 'v') { handled(); editor.paste() }
    else if (key === 'd') { handled(); editor.duplicateSelected() }
    else if (key === 'a') { handled(); editor.selectAll() }
    else if (key === 'g') { handled(); event.shiftKey ? editor.ungroupSelected() : editor.groupSelected() }
    else if (key === 'b') { handled(); editor.splitAt() }
    else if (key === 's') { handled(); void editor.flushSave().then(() => editor.saveState === 'saved' && toasts.success('Project saved')) }
    else if (key === 'e') { handled(); showExport.value = true }
    else if (key === 'i') { handled(); void importMedia() }
    return
  }
  if (event.code === 'Space') { handled(); editor.togglePlay() }
  else if (key === 's') { handled(); editor.splitAt() }
  else if (key === 'q') { handled(); editor.trimToPlayhead('start') }
  else if (key === 'w') { handled(); editor.trimToPlayhead('end') }
  else if (key === 'k') { handled(); if (event.shiftKey) editor.jumpKeyframe(-1); else if (event.altKey) editor.jumpKeyframe(1); else editor.toggleKeyframeAtPlayhead() }
  else if (event.key === 'Delete' || event.key === 'Backspace') { handled(); editor.deleteSelected(event.shiftKey) }
  else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
    handled()
    const direction = event.key === 'ArrowLeft' ? -1 : 1
    if (event.altKey) editor.nudgeSelected(direction * editor.frame)
    else if (event.shiftKey) { if (editor.playing) editor.stop(); editor.seek(editor.currentTime + direction) }
    else editor.stepFrames(direction)
  }
  else if (event.key === 'Home') { handled(); editor.seek(0) }
  else if (event.key === 'End') { handled(); editor.seek(editor.playbackEnd) }
  else if (key === '=' || key === '+') { handled(); timeline.value?.setZoom(editor.zoom * 1.4) }
  else if (key === '-') { handled(); timeline.value?.setZoom(editor.zoom / 1.4) }
  else if (key === 'z' && event.shiftKey) { handled(); timeline.value?.zoomToFit() }
  else if (event.key === 'Escape') editor.select(null)
}

// ---------------------------------------------------------------- native file drops

async function listenToFileDrops() {
  unlistenDrop = await getCurrentWebview().onDragDropEvent(event => {
    const payload = event.payload
    const ratio = window.devicePixelRatio || 1
    if (payload.type === 'enter') setExternalDrag(payload.paths, payload.position.x / ratio, payload.position.y / ratio)
    else if (payload.type === 'over') {
      if (drag.payload?.type === 'files') {
        drag.x = payload.position.x / ratio
        drag.y = payload.position.y / ratio
      }
    } else if (payload.type === 'drop') {
      const x = payload.position.x / ratio
      const y = payload.position.y / ratio
      setExternalDrag(null)
      if (!dropAt({ type: 'files', paths: payload.paths }, x, y)) {
        void importIntoProject(payload.paths).then(assets => {
          if (assets.length) toasts.success(assets.length === 1 ? `${assets[0].name} added to Media` : `${assets.length} files added to Media`)
        })
        panel.value = 'media'
      }
    } else setExternalDrag(null)
  })
}

onMounted(async () => {
  window.addEventListener('keydown', onKeydown)
  window.addEventListener('focus', editor.checkMedia)
  await load()
  await listenToFileDrops()
  unlistenClose = await getCurrentWindow().onCloseRequested(async () => {
    await editor.flushSave()
  })
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown)
  window.removeEventListener('focus', editor.checkMedia)
  unlistenDrop?.()
  unlistenClose?.()
  void getCurrentWindow().setTitle('ViralFlow').catch(() => undefined)
})

onBeforeRouteLeave(async () => {
  await editor.close()
})
</script>

<template>
  <div class="flex h-full flex-col bg-app">
    <Teleport defer to="#titlebar-left">
      <UiButton variant="ghost" size="sm" @click="router.push({ name: 'home' })"><ArrowLeft class="size-4" />Projects</UiButton>
    </Teleport>
    <Teleport v-if="editor.project" defer to="#titlebar-center">
      <div class="flex min-w-0 items-center gap-2">
        <input
          v-if="editingName" v-model="nameDraft" data-project-name maxlength="120"
          class="h-7 w-72 rounded-lg border border-accent bg-raised px-2.5 text-center text-xs font-medium outline-none"
          @blur="finishRename(true)" @keydown.enter.prevent="finishRename(true)" @keydown.esc.prevent="finishRename(false)"
        >
        <button v-else class="max-w-[420px] truncate rounded-md px-2 py-1 text-xs font-semibold hover:bg-hover" title="Rename project" @click="startRename">{{ editor.project.name }}</button>
        <span class="pointer-events-none flex items-center gap-1 text-2xs" :class="editor.saveState === 'error' ? 'text-danger' : 'text-faint'" aria-live="polite">
          <LoaderCircle v-if="editor.saveState === 'saving'" class="size-3 animate-spin" />
          <CloudAlert v-else-if="editor.saveState === 'error'" class="size-3" />
          <Check v-else-if="editor.saveState === 'saved'" class="size-3" />
          {{ saveLabel }}
        </span>
      </div>
    </Teleport>
    <Teleport defer to="#titlebar-right">
      <UiMenu :items="moreItems" @select="onMore">
        <UiButton variant="ghost" size="sm" icon aria-label="More"><MoreHorizontal class="size-4" /></UiButton>
      </UiMenu>
      <UiButton variant="primary" size="sm" :disabled="!editor.project" @click="showExport = true"><Download class="size-3.5" />Export</UiButton>
    </Teleport>

    <div v-if="loading" class="grid flex-1 place-items-center text-xs text-muted"><LoaderCircle class="size-5 animate-spin" /></div>
    <div v-else-if="loadError" class="grid flex-1 place-items-center">
      <div class="max-w-sm text-center">
        <p class="text-sm font-medium">This project could not be opened</p>
        <p class="mt-1 text-xs text-muted">{{ loadError }}</p>
        <UiButton class="mt-4" variant="outline" @click="router.push({ name: 'home' })">Back to projects</UiButton>
      </div>
    </div>
    <template v-else-if="editor.project">
      <div class="flex min-h-0 flex-1 gap-2 px-2 pt-0.5">
        <AssetPanel v-model="panel" :caption-source="captionSource" @edit-text="focusText" />
        <Player @edit-text="focusText" />
        <Inspector ref="inspector" @captions="openCaptions" />
      </div>
      <TimelineView ref="timeline" class="m-2" :height="timelineHeight" @open-panel="value => panel = value" @resize="value => timelineHeight = value" />
    </template>

    <div v-if="drag.active" class="pointer-events-none fixed z-[400] rounded-md border border-accent bg-panel/95 px-2.5 py-1.5 text-2xs text-fg shadow-xl" :style="{ left: `${drag.x + 14}px`, top: `${drag.y + 10}px` }">{{ drag.label }}</div>

    <ExportDialog v-if="editor.project" v-model:open="showExport" />
    <UiDialog v-model:open="showShortcuts" title="Keyboard shortcuts" width="620px">
      <div class="grid grid-cols-2 gap-x-8 gap-y-5 px-5 py-4">
        <section v-for="group in SHORTCUTS" :key="group.group">
          <h3 class="mb-2 text-xs font-semibold">{{ group.group }}</h3>
          <dl class="space-y-1.5">
            <div v-for="[keys, action] in group.items" :key="keys" class="flex items-baseline justify-between gap-3 text-2xs">
              <dt class="text-muted">{{ action }}</dt>
              <dd><kbd class="whitespace-nowrap rounded border border-line bg-app px-1.5 py-0.5 font-sans text-fg">{{ keys }}</kbd></dd>
            </div>
          </dl>
        </section>
      </div>
    </UiDialog>
  </div>
</template>
