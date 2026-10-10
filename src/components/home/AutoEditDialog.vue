<script setup lang="ts">
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { AudioLines, CircleAlert, CircleCheck, Film, Image as ImageIcon, LoaderCircle, X } from '@lucide/vue'
import { computed, ref } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiColor from '@/components/ui/UiColor.vue'
import UiDialog from '@/components/ui/UiDialog.vue'
import UiField from '@/components/ui/UiField.vue'
import UiSelect from '@/components/ui/UiSelect.vue'
import { buildAutoEditProject } from '@/lib/generators'
import { ASPECT_PRESETS } from '@/lib/project'
import type { Fps, MediaAsset } from '@/lib/types'
import { errorMessage, ipc } from '@/services/ipc'
import { probeMedia } from '@/services/media'
import { useProjects } from '@/stores/projects'
import { useSettings } from '@/stores/settings'
import { useToasts } from '@/stores/toast'

const open = defineModel<boolean>('open', { required: true })
const emit = defineEmits<{ created: [id: string] }>()
const settings = useSettings()
const projects = useProjects()
const toasts = useToasts()
const voices = ref<MediaAsset[]>([])
const backgrounds = ref<MediaAsset[]>([])
const images = ref<MediaAsset[]>([])
const language = ref(settings.value.captionLanguage || 'auto')
const accent = ref('#ffd43b')
const aspect = ref(settings.value.defaultAspect || '9:16')
const running = ref(false)
const statuses = ref<Record<string, { state: 'waiting' | 'working' | 'done' | 'failed'; detail?: string }>>({})

const aiAvailable = computed(() => settings.key.hasKey)
const canRun = computed(() => voices.value.length > 0 && backgrounds.value.length > 0 && !running.value)
const aspectOptions = ASPECT_PRESETS.map(preset => ({ value: preset.id, label: preset.label, hint: preset.hint }))
const languageOptions = [{ value: 'auto', label: 'Detect automatically' }, { value: 'en', label: 'English' }, { value: 'pl', label: 'Polish' }, { value: 'de', label: 'German' }, { value: 'es', label: 'Spanish' }, { value: 'fr', label: 'French' }]

async function pick(kind: 'voice' | 'background' | 'image') {
  const filters = kind === 'voice'
    ? [{ name: 'Audio', extensions: ['mp3', 'wav', 'm4a', 'aac', 'ogg', 'flac', 'opus'] }]
    : kind === 'background' ? [{ name: 'Video', extensions: ['mp4', 'mov', 'webm', 'mkv', 'm4v'] }] : [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }]
  const selection = await openDialog({ multiple: true, filters })
  if (!selection) return
  const paths = Array.isArray(selection) ? selection : [selection]
  try {
    const probed = await probeMedia(paths)
    const target = kind === 'voice' ? voices : kind === 'background' ? backgrounds : images
    const wanted = kind === 'voice' ? 'audio' : kind === 'background' ? 'video' : 'image'
    const accepted = probed.assets.filter(asset => asset.kind === wanted)
    target.value = [...target.value, ...accepted.filter(asset => !target.value.some(existing => existing.path === asset.path))]
    if (accepted.length < paths.length) toasts.error('Some files were skipped', probed.failures[0] ?? `Only ${wanted} files fit here.`)
  } catch (error) {
    toasts.error('Files could not be read', errorMessage(error))
  }
}

function remove(key: 'voice' | 'background' | 'image', path: string) {
  const list = key === 'voice' ? voices : key === 'background' ? backgrounds : images
  list.value = list.value.filter(asset => asset.path !== path)
}

async function run() {
  running.value = true
  statuses.value = Object.fromEntries(voices.value.map(voice => [voice.path, { state: 'waiting' }]))
  const preset = ASPECT_PRESETS.find(candidate => candidate.id === aspect.value) ?? ASPECT_PRESETS[0]
  const sounds = await ipc.builtinSounds().catch(() => [])
  const whoosh = await probeMedia(sounds.filter(sound => sound.name.startsWith('Whoosh')).map(sound => sound.path)).catch(() => null)
  let lastId = ''
  let created = 0
  for (const [index, voice] of voices.value.entries()) {
    statuses.value[voice.path] = { state: 'working', detail: 'Transcribing' }
    try {
      const transcript = await ipc.transcribe({ path: voice.path, offset: 0, duration: voice.duration ?? 60, language: language.value, options: { maxWords: 3 } })
      let events: Array<{ imageIndex: number; start: number; end: number }> = []
      if (images.value.length && aiAvailable.value && transcript.text.trim()) {
        statuses.value[voice.path] = { state: 'working', detail: 'Matching photos' }
        events = await ipc.aiMatchImages(transcript.text, transcript.cues, voice.duration ?? 60, images.value.map(image => image.path)).catch(() => [])
      }
      const project = buildAutoEditProject({
        voice,
        background: backgrounds.value[index % backgrounds.value.length],
        images: images.value,
        events,
        cues: transcript.cues,
        whooshIn: whoosh?.assets.find(asset => asset.name.includes('in')),
        whooshOut: whoosh?.assets.find(asset => asset.name.includes('out')),
        accent: accent.value,
        fps: settings.value.defaultFps as Fps,
        width: preset.width,
        height: preset.height
      })
      await ipc.saveProject(project)
      lastId = project.id
      created++
      statuses.value[voice.path] = { state: 'done', detail: `${transcript.cues.length} captions` }
    } catch (error) {
      statuses.value[voice.path] = { state: 'failed', detail: errorMessage(error) }
    }
  }
  running.value = false
  await projects.refresh()
  if (created) {
    toasts.success(created === 1 ? 'Video created' : `${created} videos created`, 'Every part stays editable.')
    if (created === 1) {
      open.value = false
      emit('created', lastId)
    }
  }
}
</script>

<template>
  <UiDialog v-model:open="open" title="Auto-edit voiceovers" description="Each voiceover becomes its own video with captions, a background and your photos." width="680px" :dismissible="!running">
    <div class="space-y-4 px-5 py-4">
      <div class="grid grid-cols-3 gap-3">
        <section v-for="group in ([
          { key: 'voice', title: 'Voiceovers', hint: 'One video per file', list: voices, icon: AudioLines },
          { key: 'background', title: 'Backgrounds', hint: 'Used in turn', list: backgrounds, icon: Film },
          { key: 'image', title: 'Photos', hint: aiAvailable ? 'Matched to the speech by AI' : 'Spread evenly (add an AI key to match by speech)', list: images, icon: ImageIcon }
        ] as const)" :key="group.key" class="flex min-h-[180px] flex-col rounded-lg border border-line bg-app p-2.5">
          <header class="mb-2 flex items-center gap-2">
            <component :is="group.icon" class="size-3.5 text-accent" />
            <span class="text-xs font-semibold">{{ group.title }}</span>
            <span class="ml-auto text-2xs tabular-nums text-faint">{{ group.list.length }}</span>
          </header>
          <ul class="min-h-0 flex-1 space-y-1 overflow-y-auto">
            <li v-for="asset in group.list" :key="asset.path" class="group flex items-center gap-1.5 rounded px-1.5 py-1 text-2xs hover:bg-hover">
              <template v-if="group.key === 'voice' && statuses[asset.path]">
                <LoaderCircle v-if="statuses[asset.path].state === 'working'" class="size-3 shrink-0 animate-spin text-accent" />
                <CircleCheck v-else-if="statuses[asset.path].state === 'done'" class="size-3 shrink-0 text-ok" />
                <CircleAlert v-else-if="statuses[asset.path].state === 'failed'" class="size-3 shrink-0 text-danger" />
              </template>
              <span class="min-w-0 flex-1 truncate" :title="statuses[asset.path]?.detail ?? asset.path">{{ asset.name }}</span>
              <button v-if="!running" class="opacity-0 group-hover:opacity-100" aria-label="Remove" @click="remove(group.key, asset.path)"><X class="size-3 text-muted" /></button>
            </li>
          </ul>
          <UiButton class="mt-2" variant="outline" size="sm" :disabled="running" @click="pick(group.key)">Add files</UiButton>
          <p class="mt-1.5 text-[10px] leading-3 text-faint">{{ group.hint }}</p>
        </section>
      </div>
      <div class="grid grid-cols-3 gap-3">
        <UiField label="Language"><UiSelect v-model="language" :options="languageOptions" label="Speech language" /></UiField>
        <UiField label="Format"><UiSelect v-model="aspect" :options="aspectOptions" label="Format" /></UiField>
        <UiField label="Accent"><UiColor v-model="accent" label="Accent color" /></UiField>
      </div>
      <p class="text-2xs leading-4 text-faint">Captions use {{ settings.value.transcriptionProvider === 'openrouter' ? 'cloud speech recognition' : 'the local speech model (Settings → Captions)' }}.</p>
    </div>
    <template #footer>
      <UiButton variant="ghost" :disabled="running" @click="open = false">Close</UiButton>
      <UiButton variant="primary" :disabled="!canRun" @click="run"><LoaderCircle v-if="running" class="size-3.5 animate-spin" />{{ running ? 'Editing…' : voices.length > 1 ? `Create ${voices.length} videos` : 'Create video' }}</UiButton>
    </template>
  </UiDialog>
</template>
