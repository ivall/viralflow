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
    <dl class="grid grid-cols-2 gap-y-1.5 text-xs">
      <dt class="text-muted">Length</dt><dd class="text-right tabular-nums">{{ formatDuration(editor.playbackEnd) }}</dd>
      <dt class="text-muted">Clips</dt><dd class="text-right tabular-nums">{{ clips }}</dd>
      <dt class="text-muted">Tracks</dt><dd class="text-right tabular-nums">{{ project.tracks.length }}</dd>
      <dt class="text-muted">Media files</dt><dd class="text-right tabular-nums">{{ project.assets.length }}</dd>
    </dl>
    <p class="pt-1 text-2xs leading-4 text-faint">Select a clip on the timeline or in the preview to edit it.</p>
  </UiSection>
</template>
