<script setup lang="ts">
import { save } from '@tauri-apps/plugin-dialog'
import { CircleCheck, FolderOpen, Play } from '@lucide/vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiDialog from '@/components/ui/UiDialog.vue'
import UiField from '@/components/ui/UiField.vue'
import UiSegmented from '@/components/ui/UiSegmented.vue'
import UiSelect from '@/components/ui/UiSelect.vue'
import UiSwitch from '@/components/ui/UiSwitch.vue'
import { uid } from '@/lib/engine/math'
import type { ExportCodec, ExportFormat, ExportQuality, Fps } from '@/lib/types'
import { estimateMegabytes, exportableProject, outputSize, rasterizeTexts } from '@/services/export'
import { errorMessage, ipc, onEvent, type ExportFinished, type ExportProgress } from '@/services/ipc'
import { formatDuration, formatTime } from '@/services/media'
import { useEditor } from '@/stores/editor'
import { useSettings } from '@/stores/settings'

const open = defineModel<boolean>('open', { required: true })
const editor = useEditor()
const settings = useSettings()
const project = computed(() => editor.project!)
const outputPath = ref('')
const phase = ref<'setup' | 'preparing' | 'rendering' | 'done' | 'failed'>('setup')
const progress = ref<ExportProgress | null>(null)
const error = ref('')
const resultPath = ref('')
let jobId = ''
let unlisteners: Array<() => void> = []

const formatOptions: Array<{ value: ExportFormat; label: string }> = [{ value: 'mp4', label: 'MP4' }, { value: 'mov', label: 'MOV' }, { value: 'webm', label: 'WebM' }]
const codecOptions = computed<Array<{ value: ExportCodec; label: string }>>(() => project.value.export.format === 'webm' ? [{ value: 'vp9', label: 'VP9' }] : [{ value: 'h264', label: 'H.264 - plays everywhere' }, { value: 'h265', label: 'H.265 - smaller file' }])
const resolutionOptions = computed(() => {
  const short = Math.min(project.value.width, project.value.height)
  return [480, 720, 1080, 1440, 2160].filter(value => value <= Math.max(short, 1080) * 2).map(value => ({ value, label: value === 2160 ? '4K' : `${value}p`, hint: value === short ? 'Original' : undefined }))
})
const fpsOptions = ([24, 25, 30, 50, 60] as Fps[]).map(value => ({ value, label: `${value} fps` }))
const qualityOptions: Array<{ value: ExportQuality; label: string }> = [{ value: 'draft', label: 'Draft' }, { value: 'standard', label: 'Good' }, { value: 'high', label: 'High' }, { value: 'ultra', label: 'Best' }]
const loudnessOptions = [
  { value: 0, label: 'Keep as mixed' },
  { value: -14, label: '−14 LUFS - TikTok, Reels, YouTube' },
  { value: -16, label: '−16 LUFS - podcasts, Spotify' },
  { value: -23, label: '−23 LUFS - broadcast' }
]
const size = computed(() => outputSize(project.value))
const duration = computed(() => editor.playbackEnd)
const estimate = computed(() => estimateMegabytes(project.value, duration.value))
const hardwareAvailable = computed(() => Boolean(settings.tools?.hardwareEncoders.length))
const percent = computed(() => Math.round((progress.value?.progress ?? 0) * 100))

watch(open, async value => {
  if (!value) return
  phase.value = 'setup'
  error.value = ''
  progress.value = null
  if (!project.value.export.hardware && settings.value.hardwareEncoding && hardwareAvailable.value) editor.updateExport({ hardware: true })
  outputPath.value = await ipc.defaultExportPath(project.value.name, project.value.export.format)
}, { immediate: true })

function setFormat(format: ExportFormat) {
  editor.updateExport({ format, codec: format === 'webm' ? 'vp9' : project.value.export.codec === 'vp9' ? 'h264' : project.value.export.codec })
  outputPath.value = outputPath.value.replace(/\.(mp4|mov|webm)$/i, `.${format}`)
}

async function choosePath() {
  const path = await save({ title: 'Export video', defaultPath: outputPath.value, filters: [{ name: project.value.export.format.toUpperCase(), extensions: [project.value.export.format] }] })
  if (path) outputPath.value = path
}

async function start() {
  if (duration.value < 0.05) {
    error.value = 'The timeline is empty. Add at least one clip.'
    phase.value = 'failed'
    return
  }
  phase.value = 'preparing'
  error.value = ''
  jobId = uid('export').replace(/[^a-zA-Z0-9-_]/g, '')
  try {
    await editor.flushSave()
    const snapshot = exportableProject(project.value)
    const rasters = await rasterizeTexts(snapshot)
    unlisteners.push(await onEvent<ExportProgress>('export-progress', payload => {
      if (payload.jobId !== jobId) return
      phase.value = 'rendering'
      progress.value = payload
    }))
    unlisteners.push(await onEvent<ExportFinished>('export-finished', payload => {
      if (payload.jobId !== jobId) return
      stopListening()
      if (payload.path) {
        resultPath.value = payload.path
        phase.value = 'done'
      } else if (payload.cancelled) phase.value = 'setup'
      else {
        error.value = payload.error ?? 'The export failed.'
        phase.value = 'failed'
      }
    }))
    await ipc.startExport(jobId, { project: snapshot, outputPath: outputPath.value, rasters })
  } catch (reason) {
    stopListening()
    error.value = errorMessage(reason)
    phase.value = 'failed'
  }
}

function cancel() {
  if (jobId) void ipc.cancelExport(jobId)
}

function stopListening() {
  unlisteners.forEach(stop => stop())
  unlisteners = []
}

function close() {
  if (phase.value === 'preparing' || phase.value === 'rendering') return
  open.value = false
}

onBeforeUnmount(() => {
  cancel()
  stopListening()
})
</script>

<template>
  <UiDialog :open="open" title="Export video" :description="`${project.name} · ${formatDuration(duration)}`" width="560px" :dismissible="phase !== 'preparing' && phase !== 'rendering'" @update:open="value => { if (!value) close() }">
    <div v-if="phase === 'setup' || phase === 'failed'" class="space-y-3 px-5 py-4">
      <UiField label="Save to">
        <button class="field flex items-center gap-2 text-left" :title="outputPath" @click="choosePath">
          <span class="min-w-0 flex-1 truncate">{{ outputPath }}</span><FolderOpen class="size-3.5 shrink-0 text-muted" />
        </button>
      </UiField>
      <UiField label="Format"><UiSegmented :model-value="project.export.format" :options="formatOptions" label="Format" @update:model-value="setFormat" /></UiField>
      <UiField label="Codec"><UiSelect :model-value="project.export.codec" :options="codecOptions" label="Codec" @update:model-value="value => editor.updateExport({ codec: value })" /></UiField>
      <UiField label="Resolution"><UiSelect :model-value="project.export.resolution" :options="resolutionOptions" label="Resolution" @update:model-value="value => editor.updateExport({ resolution: value })" /></UiField>
      <UiField label="Frame rate"><UiSelect :model-value="project.export.fps" :options="fpsOptions" label="Frame rate" @update:model-value="value => editor.updateExport({ fps: value })" /></UiField>
      <UiField label="Quality"><UiSegmented :model-value="project.export.quality" :options="qualityOptions" label="Quality" @update:model-value="value => editor.updateExport({ quality: value })" /></UiField>
      <UiField v-if="project.export.codec !== 'vp9'" label="GPU encoding" :hint="hardwareAvailable ? settings.tools?.hardwareEncoders.join(', ') : 'No supported GPU encoder was found'">
        <div class="flex items-center gap-2">
          <UiSwitch :model-value="project.export.hardware && hardwareAvailable" label="GPU encoding" :disabled="!hardwareAvailable" @update:model-value="value => editor.updateExport({ hardware: value })" />
          <span class="text-2xs text-muted">{{ hardwareAvailable ? 'Faster, slightly larger files' : 'Not available on this computer' }}</span>
        </div>
      </UiField>
      <UiField label="Loudness" hint="Evens out the volume of the whole soundtrack to the level the platform plays at">
        <UiSelect :model-value="project.export.loudness ?? 0" :options="loudnessOptions" label="Loudness" @update:model-value="value => editor.updateExport({ loudness: value || undefined })" />
      </UiField>
      <div class="flex items-center justify-between rounded-md bg-app px-3 py-2 text-2xs text-muted">
        <span class="tabular-nums">{{ size.width }}×{{ size.height }} · {{ project.export.fps }} fps</span>
        <span class="tabular-nums">about {{ estimate }} MB</span>
      </div>
      <p v-if="phase === 'failed'" class="whitespace-pre-wrap rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-2xs leading-4 text-danger">{{ error }}</p>
    </div>

    <div v-else-if="phase === 'preparing' || phase === 'rendering'" class="px-5 py-8">
      <div class="flex items-baseline justify-between text-xs">
        <span class="text-fg">{{ progress?.stage ?? 'Preparing text and media' }}</span>
        <span class="tabular-nums text-muted">{{ percent }}%</span>
      </div>
      <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-line"><div class="h-full rounded-full bg-accent transition-[width] duration-200" :style="{ width: `${percent}%` }" /></div>
      <div class="mt-2 flex justify-between text-2xs tabular-nums text-faint">
        <span v-if="progress?.totalFrames">Frame {{ progress.frame }} of {{ progress.totalFrames }}<template v-if="progress.speed"> · {{ progress.speed.toFixed(1) }}× real time</template></span>
        <span v-if="progress?.etaSeconds !== undefined && progress.etaSeconds !== null">{{ formatTime(progress.etaSeconds) }} left</span>
      </div>
    </div>

    <div v-else class="flex flex-col items-center px-5 py-8 text-center">
      <CircleCheck class="size-8 text-ok" />
      <p class="mt-3 text-sm font-medium">Your video is ready</p>
      <p class="mt-1 max-w-full truncate text-2xs text-muted" :title="resultPath">{{ resultPath }}</p>
    </div>

    <template #footer>
      <template v-if="phase === 'setup' || phase === 'failed'">
        <UiButton variant="ghost" @click="open = false">Cancel</UiButton>
        <UiButton variant="primary" :disabled="!outputPath" @click="start">Export</UiButton>
      </template>
      <UiButton v-else-if="phase === 'preparing' || phase === 'rendering'" variant="outline" @click="cancel">Stop export</UiButton>
      <template v-else>
        <UiButton variant="ghost" @click="ipc.revealPath(resultPath)"><FolderOpen class="size-3.5" />Show in folder</UiButton>
        <UiButton variant="outline" @click="ipc.openPath(resultPath)"><Play class="size-3.5" />Play</UiButton>
        <UiButton variant="primary" @click="open = false">Done</UiButton>
      </template>
    </template>
  </UiDialog>
</template>
