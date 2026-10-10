<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { ArrowLeft, CircleCheck, CircleX, Download, Eye, EyeOff, FolderOpen, RefreshCw } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import ModelPicker from '@/components/settings/ModelPicker.vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiSegmented from '@/components/ui/UiSegmented.vue'
import UiSelect from '@/components/ui/UiSelect.vue'
import UiSwitch from '@/components/ui/UiSwitch.vue'
import { ASPECT_PRESETS } from '@/lib/project'
import { errorMessage, ipc, onEvent, type WhisperModel } from '@/services/ipc'
import { formatBytes } from '@/services/media'
import { useSettings } from '@/stores/settings'
import { useToasts } from '@/stores/toast'

const router = useRouter()
const settings = useSettings()
const toasts = useToasts()
const models = ref<WhisperModel[]>([])
const downloading = ref<{ id: string; received: number; total: number } | null>(null)
const showKey = ref(false)
const checking = ref(false)
const keyDraft = ref(settings.value.openrouterApiKey)
let unlisten: (() => void) | null = null

const fpsOptions = [24, 25, 30, 50, 60].map(value => ({ value, label: `${value} fps` }))
const aspectOptions = ASPECT_PRESETS.map(preset => ({ value: preset.id, label: preset.label, hint: preset.hint }))
const providerOptions = [{ value: 'local' as const, label: 'On this computer' }, { value: 'openrouter' as const, label: 'Cloud (OpenRouter)' }]
const languageOptions = [
  { value: 'auto', label: 'Detect automatically' }, { value: 'en', label: 'English' }, { value: 'pl', label: 'Polish' }, { value: 'de', label: 'German' },
  { value: 'es', label: 'Spanish' }, { value: 'fr', label: 'French' }, { value: 'it', label: 'Italian' }, { value: 'pt', label: 'Portuguese' }, { value: 'uk', label: 'Ukrainian' }
]

onMounted(async () => {
  await refreshModels()
  unlisten = await onEvent<{ id: string; received: number; total: number }>('model-download', payload => { downloading.value = payload })
})
onBeforeUnmount(() => unlisten?.())

async function refreshModels() {
  try { models.value = await ipc.whisperModels() } catch { models.value = [] }
}

async function chooseFfmpeg() {
  const path = await open({ title: 'Choose the FFmpeg folder', directory: true })
  if (!path || Array.isArray(path)) return
  await settings.update({ ffmpegPath: path })
  toasts.show(settings.tools?.found ? 'FFmpeg found' : 'FFmpeg was not found in that folder', { tone: settings.tools?.found ? 'success' : 'error' })
}

async function recheck() {
  checking.value = true
  await settings.refreshTools()
  checking.value = false
}

async function chooseExportFolder() {
  const path = await open({ title: 'Default export folder', directory: true })
  if (path && !Array.isArray(path)) await settings.update({ exportFolder: path })
}

async function download(model: WhisperModel) {
  downloading.value = { id: model.id, received: 0, total: model.size }
  try {
    await ipc.downloadWhisperModel(model.id)
    await settings.update({ whisperModel: model.id })
    toasts.success(`${model.label.split(' - ')[0]} model installed`)
  } catch (error) {
    toasts.error('Download failed', errorMessage(error))
  } finally {
    downloading.value = null
    await refreshModels()
  }
}

const transcriptionModel = computed({
  get: () => settings.value.openrouterTranscriptionModel,
  set: (value: string) => { void settings.update({ openrouterTranscriptionModel: value }) }
})

const aiModel = computed({
  get: () => settings.value.openrouterModel,
  set: (value: string) => { void settings.update({ openrouterModel: value }) }
})

function saveKey() {
  void settings.update({ openrouterApiKey: keyDraft.value.trim() })
  toasts.success(keyDraft.value.trim() ? 'API key saved' : 'API key removed')
}
</script>

<template>
  <div class="flex h-full flex-col bg-app">
    <header class="flex h-12 shrink-0 items-center gap-3 border-b border-line px-3">
      <UiButton variant="ghost" size="sm" @click="router.back()"><ArrowLeft class="size-4" />Back</UiButton>
      <h1 class="text-sm font-semibold">Settings</h1>
    </header>
    <main class="min-h-0 flex-1 overflow-y-auto">
      <div class="mx-auto max-w-[720px] space-y-8 px-8 py-8">
        <section>
          <h2 class="text-sm font-semibold">Editing</h2>
          <div class="mt-3 divide-y divide-line rounded-lg border border-line bg-panel">
            <label class="flex items-center gap-4 px-4 py-3"><span class="flex-1"><span class="block text-xs font-medium">Save automatically</span><span class="block text-2xs text-muted">Every change is written to disk a moment after you make it.</span></span><UiSwitch :model-value="settings.value.autosave" label="Save automatically" @update:model-value="value => settings.update({ autosave: value })" /></label>
            <label class="flex items-center gap-4 px-4 py-3"><span class="flex-1"><span class="block text-xs font-medium">Snap on the timeline</span><span class="block text-2xs text-muted">Clips snap to the playhead and to other clips. Hold Shift to place freely.</span></span><UiSwitch :model-value="settings.value.snap" label="Snap" @update:model-value="value => settings.update({ snap: value })" /></label>
            <label class="flex items-center gap-4 px-4 py-3"><span class="flex-1"><span class="block text-xs font-medium">Show safe zone</span><span class="block text-2xs text-muted">Marks the area that TikTok, Reels and Shorts cover with buttons.</span></span><UiSwitch :model-value="settings.value.safeArea" label="Safe zone" @update:model-value="value => settings.update({ safeArea: value })" /></label>
            <div class="flex items-center gap-4 px-4 py-3"><span class="min-w-0 flex-1 text-xs font-medium">Frame rate of new projects</span><div class="w-40 shrink-0"><UiSelect :model-value="settings.value.defaultFps" :options="fpsOptions" label="Default frame rate" @update:model-value="value => settings.update({ defaultFps: value })" /></div></div>
            <div class="flex items-center gap-4 px-4 py-3"><span class="min-w-0 flex-1 text-xs font-medium">Default format</span><div class="w-40 shrink-0"><UiSelect :model-value="settings.value.defaultAspect" :options="aspectOptions" label="Default format" @update:model-value="value => settings.update({ defaultAspect: value })" /></div></div>
          </div>
        </section>

        <section>
          <h2 class="text-sm font-semibold">Tools</h2>
          <div class="mt-3 rounded-lg border border-line bg-panel px-4 py-3">
            <div class="flex items-start gap-3">
              <CircleCheck v-if="settings.tools?.found" class="mt-0.5 size-4 shrink-0 text-ok" />
              <CircleX v-else class="mt-0.5 size-4 shrink-0 text-danger" />
              <div class="min-w-0 flex-1">
                <p class="text-xs font-medium">{{ settings.tools?.found ? 'FFmpeg is ready' : 'FFmpeg was not found' }}</p>
                <p class="mt-0.5 truncate text-2xs text-muted" :title="settings.tools?.ffmpeg">{{ settings.tools?.found ? `${settings.tools.version ?? ''}` : 'It is needed to read, preview and export media.' }}</p>
                <p v-if="settings.tools?.found" class="mt-0.5 truncate text-2xs text-faint">{{ settings.tools.ffmpeg }}</p>
                <p v-if="settings.tools?.found" class="mt-1 text-2xs text-muted">
                  Speech recognition: {{ settings.tools.whisper ? 'available' : 'not in this build' }} · GPU encoders: {{ settings.tools.hardwareEncoders.length ? settings.tools.hardwareEncoders.join(', ') : 'none' }}
                </p>
              </div>
            </div>
            <div class="mt-3 flex gap-2">
              <UiButton variant="outline" size="sm" @click="chooseFfmpeg"><FolderOpen class="size-3.5" />Choose folder</UiButton>
              <UiButton v-if="settings.value.ffmpegPath" variant="ghost" size="sm" @click="settings.update({ ffmpegPath: '' })">Use automatic detection</UiButton>
              <UiButton variant="ghost" size="sm" :disabled="checking" @click="recheck"><RefreshCw class="size-3.5" :class="checking && 'animate-spin'" />Check again</UiButton>
            </div>
          </div>
        </section>

        <section>
          <h2 class="text-sm font-semibold">Captions</h2>
          <div class="mt-3 space-y-3 rounded-lg border border-line bg-panel px-4 py-3">
            <div class="flex items-center gap-4"><span class="min-w-0 flex-1 text-xs font-medium">Speech recognition</span><UiSegmented class="w-72" :model-value="settings.value.transcriptionProvider" :options="providerOptions" label="Speech recognition" @update:model-value="value => settings.update({ transcriptionProvider: value })" /></div>
            <div class="flex items-center gap-4"><span class="min-w-0 flex-1 text-xs font-medium">Default language</span><div class="w-48 shrink-0"><UiSelect :model-value="settings.value.captionLanguage" :options="languageOptions" label="Caption language" @update:model-value="value => settings.update({ captionLanguage: value })" /></div></div>
            <div v-if="settings.value.transcriptionProvider === 'openrouter'">
              <span class="mb-1 block text-xs font-medium">Speech-to-text model</span>
              <ModelPicker v-model="transcriptionModel" kind="transcription" />
              <span class="mt-1 block text-2xs text-faint">{{ settings.value.openrouterApiKey ? 'Audio of the selected clip is sent to OpenRouter.' : 'Add your OpenRouter API key below to use cloud captions.' }}</span>
            </div>
            <div v-if="settings.value.transcriptionProvider === 'local'">
              <p class="mb-2 text-2xs text-muted">Local models run privately with Whisper. Larger models are more accurate but slower.</p>
              <ul class="divide-y divide-line rounded-md border border-line">
                <li v-for="model in models" :key="model.id" class="flex items-center gap-3 px-3 py-2">
                  <input type="radio" name="whisper-model" class="accent-[#4cc9f0]" :checked="settings.value.whisperModel === model.id" :disabled="!model.installed" :aria-label="model.label" @change="settings.update({ whisperModel: model.id })">
                  <span class="min-w-0 flex-1">
                    <span class="block text-xs">{{ model.label }}</span>
                    <span class="block text-2xs text-faint">{{ formatBytes(model.size) }}</span>
                  </span>
                  <span v-if="model.installed" class="text-2xs text-ok">Installed</span>
                  <div v-else-if="downloading?.id === model.id" class="h-1.5 w-28 overflow-hidden rounded-full bg-line"><div class="h-full bg-accent" :style="{ width: `${Math.round(downloading.received / Math.max(1, downloading.total) * 100)}%` }" /></div>
                  <UiButton v-else variant="outline" size="sm" :disabled="Boolean(downloading)" @click="download(model)"><Download class="size-3.5" />Download</UiButton>
                </li>
              </ul>
            </div>
          </div>
        </section>

        <section>
          <h2 class="text-sm font-semibold">AI assistant</h2>
          <div class="mt-3 space-y-3 rounded-lg border border-line bg-panel px-4 py-3">
            <p class="text-2xs leading-4 text-muted">Optional. With an OpenRouter key you can draft videos from a topic, auto-edit voiceovers with your photos, rewrite text and use cloud captions. The key is stored only on this computer.</p>
            <label class="block">
              <span class="mb-1 block text-xs font-medium">OpenRouter API key</span>
              <span class="relative block">
                <input v-model="keyDraft" :type="showKey ? 'text' : 'password'" class="field pr-9" placeholder="sk-or-…" autocomplete="off" spellcheck="false">
                <button class="absolute right-1 top-1/2 grid size-7 -translate-y-1/2 place-items-center rounded text-muted hover:text-fg" :aria-label="showKey ? 'Hide key' : 'Show key'" @click="showKey = !showKey"><EyeOff v-if="showKey" class="size-3.5" /><Eye v-else class="size-3.5" /></button>
              </span>
            </label>
            <div>
              <span class="mb-1 block text-xs font-medium">Model</span>
              <ModelPicker v-model="aiModel" />
              <span class="mt-1 block text-2xs text-faint">Used for drafts, photo matching and rewriting. Changes are saved right away.</span>
            </div>
            <UiButton variant="outline" size="sm" :disabled="keyDraft.trim() === settings.value.openrouterApiKey" @click="saveKey">Save key</UiButton>
          </div>
        </section>

        <section>
          <h2 class="text-sm font-semibold">Export</h2>
          <div class="mt-3 divide-y divide-line rounded-lg border border-line bg-panel">
            <div class="flex items-center gap-4 px-4 py-3">
              <span class="min-w-0 flex-1"><span class="block text-xs font-medium">Default folder</span><span class="block truncate text-2xs text-muted">{{ settings.value.exportFolder || 'Videos folder' }}</span></span>
              <UiButton variant="outline" size="sm" @click="chooseExportFolder"><FolderOpen class="size-3.5" />Change</UiButton>
            </div>
            <label class="flex items-center gap-4 px-4 py-3"><span class="flex-1"><span class="block text-xs font-medium">Prefer GPU encoding</span><span class="block text-2xs text-muted">Uses NVIDIA, Intel or AMD hardware encoders when available.</span></span><UiSwitch :model-value="settings.value.hardwareEncoding" label="GPU encoding" @update:model-value="value => settings.update({ hardwareEncoding: value })" /></label>
          </div>
        </section>
      </div>
    </main>
  </div>
</template>
