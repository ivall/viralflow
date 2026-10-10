<script setup lang="ts">
import { open, save } from '@tauri-apps/plugin-dialog'
import { Captions, Download, FileDown, FileUp, LoaderCircle } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiField from '@/components/ui/UiField.vue'
import UiSegmented from '@/components/ui/UiSegmented.vue'
import UiSelect from '@/components/ui/UiSelect.vue'
import { uid } from '@/lib/engine/math'
import { CAPTION_PRESET, TEXT_PRESETS } from '@/lib/project'
import { formatSrt, parseSubtitles } from '@/lib/srt'
import { captionCues, generateCaptions, speechSources } from '@/services/captions'
import { errorMessage, ipc, onEvent, type WhisperModel } from '@/services/ipc'
import { formatBytes } from '@/services/media'
import { useEditor } from '@/stores/editor'
import { useSettings } from '@/stores/settings'
import { useToasts } from '@/stores/toast'

const props = defineProps<{ sourceId?: string | null }>()
const editor = useEditor()
const settings = useSettings()
const toasts = useToasts()
const source = ref<string>('')
const language = ref(settings.value.captionLanguage || 'auto')
const maxWords = ref(3)
const preset = ref('caption-default')
const position = ref<'top' | 'middle' | 'bottom'>('bottom')
const busy = ref(false)
const models = ref<WhisperModel[]>([])
const downloading = ref<{ received: number; total: number } | null>(null)
let unlisten: (() => void) | null = null

const sources = computed(() => speechSources().map(item => ({ value: item.id, label: `${item.name} (${item.start.toFixed(1)}s)` })))
const cloud = computed(() => settings.value.transcriptionProvider === 'openrouter')
const model = computed(() => models.value.find(candidate => candidate.id === settings.value.whisperModel))
const ready = computed(() => cloud.value ? settings.key.hasKey : Boolean(model.value?.installed) && settings.value && settings.tools?.whisper !== false)
const languageOptions = [
  { value: 'auto', label: 'Detect automatically' }, { value: 'en', label: 'English' }, { value: 'pl', label: 'Polish' }, { value: 'de', label: 'German' },
  { value: 'es', label: 'Spanish' }, { value: 'fr', label: 'French' }, { value: 'it', label: 'Italian' }, { value: 'pt', label: 'Portuguese' },
  { value: 'uk', label: 'Ukrainian' }, { value: 'ru', label: 'Russian' }, { value: 'nl', label: 'Dutch' }, { value: 'tr', label: 'Turkish' },
  { value: 'ja', label: 'Japanese' }, { value: 'ko', label: 'Korean' }, { value: 'zh', label: 'Chinese' }
]
const styleOptions = [{ value: 'caption-default', label: 'Bold with outline' }, ...TEXT_PRESETS.map(entry => ({ value: entry.id, label: entry.label }))]
const wordOptions = [{ value: 1, label: '1' }, { value: 2, label: '2' }, { value: 3, label: '3' }, { value: 5, label: '5' }, { value: 8, label: '8' }]
const positionOptions = [{ value: 'top' as const, label: 'Top' }, { value: 'middle' as const, label: 'Middle' }, { value: 'bottom' as const, label: 'Bottom' }]

function pickDefaultSource() {
  const ids = sources.value.map(option => option.value)
  if (props.sourceId && ids.includes(props.sourceId)) source.value = props.sourceId
  else if (editor.primaryId && ids.includes(editor.primaryId)) source.value = editor.primaryId
  else if (!ids.includes(source.value)) source.value = ids[0] ?? ''
}

watch(() => [props.sourceId, sources.value.length], pickDefaultSource, { immediate: true })

onMounted(async () => {
  try { models.value = await ipc.whisperModels() } catch { models.value = [] }
  unlisten = await onEvent<{ id: string; received: number; total: number }>('model-download', payload => {
    downloading.value = { received: payload.received, total: payload.total }
  })
})

onBeforeUnmount(() => unlisten?.())

async function downloadModel() {
  if (!model.value) return
  downloading.value = { received: 0, total: model.value.size }
  try {
    await ipc.downloadWhisperModel(model.value.id)
    models.value = await ipc.whisperModels()
    toasts.success('Speech model installed')
  } catch (error) {
    toasts.error('Model download failed', errorMessage(error))
  } finally {
    downloading.value = null
  }
}

function captionStyle() {
  if (preset.value === 'caption-default') return { ...CAPTION_PRESET }
  return { ...TEXT_PRESETS.find(entry => entry.id === preset.value)?.style, maxWidth: 80 }
}

async function generate() {
  if (!source.value) return
  busy.value = true
  try {
    const result = await generateCaptions(source.value, { language: language.value, maxWords: maxWords.value, style: captionStyle(), y: position.value === 'top' ? 20 : position.value === 'middle' ? 50 : 74 })
    if (!result.count) toasts.show('No speech was found in this clip')
    else toasts.success(`Added ${result.count} captions`, result.alignment === 'word' ? 'Timed word by word.' : 'Timed by sentence; adjust any caption on the timeline.')
  } catch (error) {
    toasts.error('Captions could not be created', errorMessage(error))
  } finally {
    busy.value = false
  }
}

async function importSubtitles() {
  const path = await open({ title: 'Import subtitles', filters: [{ name: 'Subtitles', extensions: ['srt', 'vtt'] }] })
  if (!path || Array.isArray(path)) return
  try {
    const cues = parseSubtitles(await ipc.readTextFile(path))
    if (!cues.length) {
      toasts.error('No subtitles found in this file')
      return
    }
    const count = editor.applyCaptions(cues, { sourceId: uid('srt'), offset: 0, speed: 1, style: captionStyle(), y: position.value === 'top' ? 20 : position.value === 'middle' ? 50 : 74 })
    toasts.success(`Imported ${count} subtitles`)
  } catch (error) {
    toasts.error('Subtitles could not be imported', errorMessage(error))
  }
}

async function exportSubtitles() {
  const cues = captionCues()
  if (!cues.length) {
    toasts.show('There are no captions to export')
    return
  }
  const path = await save({ title: 'Export subtitles', defaultPath: `${editor.project?.name ?? 'captions'}.srt`, filters: [{ name: 'SubRip', extensions: ['srt'] }] })
  if (!path) return
  try {
    await ipc.writeTextFile(path, formatSrt(cues))
    toasts.success('Subtitles exported', path)
  } catch (error) {
    toasts.error('Subtitles could not be exported', errorMessage(error))
  }
}
</script>

<template>
  <div class="h-full space-y-3 overflow-y-auto p-3">
    <section class="space-y-2.5 rounded-lg border border-line p-3">
      <h3 class="flex items-center gap-2 text-xs font-semibold"><Captions class="size-4 text-accent" />Auto captions</h3>
      <template v-if="sources.length">
        <UiField label="Speech from"><UiSelect v-model="source" :options="sources" label="Source clip" /></UiField>
        <UiField label="Language"><UiSelect v-model="language" :options="languageOptions" label="Language" /></UiField>
        <UiField label="Words"><UiSegmented v-model="maxWords" :options="wordOptions" label="Words per caption" /></UiField>
        <UiField label="Style"><UiSelect v-model="preset" :options="styleOptions" label="Caption style" /></UiField>
        <UiField label="Position"><UiSegmented v-model="position" :options="positionOptions" label="Caption position" /></UiField>

        <div v-if="!cloud && settings.tools && !settings.tools.whisper" class="rounded-md bg-warn/10 p-2 text-2xs leading-4 text-warn">
          Your FFmpeg build has no speech recognition. Install FFmpeg 8 (full build) or switch captions to the cloud in Settings.
        </div>
        <div v-else-if="!cloud && model && !model.installed" class="space-y-2 rounded-md bg-app p-2.5">
          <p class="text-2xs leading-4 text-muted">Captions run on this computer. The “{{ model.label.split(' - ')[0] }}” speech model ({{ formatBytes(model.size) }}) is downloaded once.</p>
          <UiButton v-if="!downloading" variant="outline" size="sm" class="w-full" @click="downloadModel"><Download class="size-3.5" />Download model</UiButton>
          <div v-else class="h-1.5 overflow-hidden rounded-full bg-line"><div class="h-full bg-accent transition-[width]" :style="{ width: `${Math.round(downloading.received / Math.max(1, downloading.total) * 100)}%` }" /></div>
        </div>
        <p v-else-if="cloud && !settings.key.hasKey" class="text-2xs text-warn">Add an OpenRouter key in Settings to use cloud captions.</p>

        <UiButton variant="primary" class="w-full" :disabled="!ready || busy || !source" @click="generate">
          <LoaderCircle v-if="busy" class="size-3.5 animate-spin" />{{ busy ? 'Listening…' : 'Generate captions' }}
        </UiButton>
      </template>
      <p v-else class="text-2xs leading-4 text-muted">Add a video with sound or an audio clip to the timeline first.</p>
    </section>

    <section class="space-y-2 rounded-lg border border-line p-3">
      <h3 class="text-xs font-semibold">Subtitle files</h3>
      <div class="flex gap-2">
        <UiButton variant="outline" size="sm" class="flex-1" @click="importSubtitles"><FileUp class="size-3.5" />Import .srt</UiButton>
        <UiButton variant="outline" size="sm" class="flex-1" @click="exportSubtitles"><FileDown class="size-3.5" />Export .srt</UiButton>
      </div>
    </section>
  </div>
</template>
