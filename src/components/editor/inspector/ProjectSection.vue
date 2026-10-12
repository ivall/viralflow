<script setup lang="ts">
import { computed } from 'vue'
import UiColor from '@/components/ui/UiColor.vue'
import UiField from '@/components/ui/UiField.vue'
import UiSection from '@/components/ui/UiSection.vue'
import UiSelect from '@/components/ui/UiSelect.vue'
import { ASPECT_PRESETS } from '@/lib/project'
import type { Fps } from '@/lib/types'
import { formatDuration } from '@/services/media'
import { useEditor } from '@/stores/editor'

const editor = useEditor()
const project = computed(() => editor.project!)
const aspectOptions = ASPECT_PRESETS.map(preset => ({ value: preset.id, label: `${preset.label} - ${preset.width}×${preset.height}`, hint: preset.hint }))
const aspect = computed(() => ASPECT_PRESETS.find(preset => preset.width === project.value.width && preset.height === project.value.height)?.id ?? 'custom')
const fpsOptions = ([24, 25, 30, 50, 60] as Fps[]).map(value => ({ value, label: `${value} fps` }))
const clips = computed(() => project.value.tracks.reduce((sum, track) => sum + track.items.length, 0))
const stats = computed(() => [
  { label: 'Length', value: formatDuration(editor.playbackEnd) },
  { label: 'Clips', value: String(clips.value) },
  { label: 'Tracks', value: String(project.value.tracks.length) },
  { label: 'Media files', value: String(project.value.assets.length) }
])

function setAspect(id: string) {
  const preset = ASPECT_PRESETS.find(candidate => candidate.id === id)
  if (preset) editor.setCanvas(preset.width, preset.height)
}
</script>

<template>
  <UiSection title="Project">
    <UiField label="Name">
      <input class="field" :value="project.name" aria-label="Project name" maxlength="120" @change="editor.rename(($event.target as HTMLInputElement).value)">
    </UiField>
    <UiField label="Canvas">
      <UiSelect :model-value="aspect" :options="aspect === 'custom' ? [{ value: 'custom', label: `${project.width}×${project.height}` }, ...aspectOptions] : aspectOptions" label="Canvas ratio" @update:model-value="setAspect" />
    </UiField>
    <UiField label="Frame rate">
      <UiSelect :model-value="project.fps" :options="fpsOptions" label="Frame rate" @update:model-value="value => { editor.beginChange(); editor.updateExport({ fps: value }) }" />
    </UiField>
    <UiField label="Background">
      <UiColor :model-value="project.background" label="Background color" @start="editor.beginChange()" @update:model-value="value => editor.setBackground(value, false)" />
    </UiField>
  </UiSection>
  <UiSection title="Overview">
    <dl class="divide-y divide-line text-xs">
      <div v-for="stat in stats" :key="stat.label" class="flex h-8 items-center justify-between">
        <dt class="text-muted">{{ stat.label }}</dt>
        <dd class="tabular-nums text-fg">{{ stat.value }}</dd>
      </div>
    </dl>
  </UiSection>
  <p class="px-4 pb-4 text-2xs leading-4 text-faint">Select a clip on the timeline or in the preview to edit it.</p>
</template>
