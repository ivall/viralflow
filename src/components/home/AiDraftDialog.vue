<script setup lang="ts">
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { LoaderCircle, Sparkles } from '@lucide/vue'
import { computed, reactive, ref } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiColor from '@/components/ui/UiColor.vue'
import UiDialog from '@/components/ui/UiDialog.vue'
import UiField from '@/components/ui/UiField.vue'
import UiSegmented from '@/components/ui/UiSegmented.vue'
import UiSelect from '@/components/ui/UiSelect.vue'
import { buildDraftProject } from '@/lib/generators'
import { ASPECT_PRESETS } from '@/lib/project'
import type { Fps, MediaAsset } from '@/lib/types'
import { errorMessage, ipc, type ScriptPlan } from '@/services/ipc'
import { probeMedia } from '@/services/media'
import { useProjects } from '@/stores/projects'
import { useSettings } from '@/stores/settings'
import { useToasts } from '@/stores/toast'

const open = defineModel<boolean>('open', { required: true })
const emit = defineEmits<{ created: [id: string] }>()
const settings = useSettings()
const projects = useProjects()
const toasts = useToasts()
const brief = reactive({ topic: '', goal: 'Keep viewers watching to the end', platform: 'TikTok', language: 'English', duration: 30, tone: 'Energetic', cta: 'Follow for more', script: '' })
const accent = ref('#4cc9f0')
const aspect = ref(settings.value.defaultAspect || '9:16')
const background = ref<MediaAsset | null>(null)
const plan = ref<ScriptPlan | null>(null)
const busy = ref(false)

const hasKey = computed(() => settings.key.hasKey)
const platformOptions = ['TikTok', 'YouTube Shorts', 'Instagram Reels', 'YouTube'].map(value => ({ value, label: value }))
const toneOptions = ['Energetic', 'Calm', 'Funny', 'Expert', 'Mysterious', 'Inspiring'].map(value => ({ value, label: value }))
const languageOptions = ['English', 'Polish', 'German', 'Spanish', 'French', 'Italian', 'Portuguese', 'Ukrainian'].map(value => ({ value, label: value }))
const durationOptions = [15, 30, 45, 60, 90].map(value => ({ value, label: `${value}s` }))
const aspectOptions = ASPECT_PRESETS.map(preset => ({ value: preset.id, label: preset.label, hint: preset.hint }))

async function write() {
  if (!brief.topic.trim()) return
  busy.value = true
  try {
    plan.value = await ipc.aiPlan({ ...brief })
  } catch (error) {
    toasts.error('The script could not be written', errorMessage(error))
  } finally {
    busy.value = false
  }
}

async function pickBackground() {
  const selection = await openDialog({ filters: [{ name: 'Video or image', extensions: ['mp4', 'mov', 'webm', 'mkv', 'png', 'jpg', 'jpeg', 'webp'] }] })
  if (!selection || Array.isArray(selection)) return
  const probed = await probeMedia([selection]).catch(() => null)
  background.value = probed?.assets.find(asset => asset.kind !== 'audio') ?? null
  if (!background.value) toasts.error('This file cannot be used as a background')
}

async function create() {
  if (!plan.value) return
  busy.value = true
  try {
    const preset = ASPECT_PRESETS.find(candidate => candidate.id === aspect.value) ?? ASPECT_PRESETS[0]
    const project = buildDraftProject({
      title: plan.value.title || brief.topic,
      hook: plan.value.hook,
      cta: plan.value.cta || brief.cta,
      cues: plan.value.textCues,
      duration: brief.duration,
      accent: accent.value,
      background: background.value ?? undefined,
      width: preset.width,
      height: preset.height,
      fps: settings.value.defaultFps as Fps
    })
    await ipc.saveProject(project)
    await projects.refresh()
    open.value = false
    plan.value = null
    emit('created', project.id)
  } catch (error) {
    toasts.error('The project could not be created', errorMessage(error))
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <UiDialog v-model:open="open" title="Draft from a topic" description="AI writes a hook, captions and a call to action; you add footage and polish." width="640px" :dismissible="!busy">
    <div v-if="!hasKey" class="px-5 py-6 text-center">
      <p class="text-xs text-fg">This feature needs an OpenRouter API key.</p>
      <p class="mt-1 text-2xs text-muted">Add it in Settings → AI assistant. Everything else in the editor works without it.</p>
    </div>
    <div v-else-if="!plan" class="space-y-3 px-5 py-4">
      <label class="block">
        <span class="mb-1 block text-xs font-medium">What is the video about?</span>
        <textarea v-model="brief.topic" class="field" rows="3" maxlength="500" placeholder="e.g. Three habits that make mornings easier" />
      </label>
      <div class="grid grid-cols-2 gap-x-4 gap-y-2.5">
        <UiField label="Platform"><UiSelect v-model="brief.platform" :options="platformOptions" label="Platform" /></UiField>
        <UiField label="Language"><UiSelect v-model="brief.language" :options="languageOptions" label="Language" /></UiField>
        <UiField label="Tone"><UiSelect v-model="brief.tone" :options="toneOptions" label="Tone" /></UiField>
        <UiField label="Format"><UiSelect v-model="aspect" :options="aspectOptions" label="Format" /></UiField>
      </div>
      <UiField label="Length"><UiSegmented v-model="brief.duration" :options="durationOptions" label="Length" /></UiField>
      <UiField label="Goal"><input v-model="brief.goal" class="field" maxlength="200"></UiField>
      <UiField label="Call to action"><input v-model="brief.cta" class="field" maxlength="120"></UiField>
    </div>
    <div v-else class="space-y-3 px-5 py-4">
      <UiField label="Title"><input v-model="plan.title" class="field"></UiField>
      <UiField label="Hook"><input v-model="plan.hook" class="field"></UiField>
      <UiField label="Ending"><input v-model="plan.cta" class="field"></UiField>
      <div>
        <p class="mb-1 text-xs text-muted">Script</p>
        <textarea v-model="plan.script" class="field" rows="5" />
      </div>
      <p class="text-2xs text-faint">{{ plan.textCues.length }} timed captions will be created from the script.</p>
      <div class="grid grid-cols-2 gap-4">
        <UiField label="Accent"><UiColor v-model="accent" label="Accent color" /></UiField>
        <UiField label="Background">
          <UiButton variant="outline" size="sm" class="w-full justify-start" @click="pickBackground"><span class="truncate">{{ background?.name ?? 'Choose video or photo…' }}</span></UiButton>
        </UiField>
      </div>
    </div>
    <template #footer>
      <UiButton v-if="plan" variant="ghost" :disabled="busy" @click="plan = null">Back</UiButton>
      <UiButton v-else variant="ghost" @click="open = false">Cancel</UiButton>
      <UiButton v-if="hasKey && !plan" variant="primary" :disabled="busy || !brief.topic.trim()" @click="write"><LoaderCircle v-if="busy" class="size-3.5 animate-spin" /><Sparkles v-else class="size-3.5" />{{ busy ? 'Writing…' : 'Write script' }}</UiButton>
      <UiButton v-else-if="plan" variant="primary" :disabled="busy" @click="create">Create project</UiButton>
    </template>
  </UiDialog>
</template>
