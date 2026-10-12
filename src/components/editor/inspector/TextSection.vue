<script setup lang="ts">
import { AlignCenter, AlignLeft, AlignRight, CaseUpper, Italic, Sparkles } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import UiButton from '@/components/ui/UiButton.vue'
import UiColor from '@/components/ui/UiColor.vue'
import UiField from '@/components/ui/UiField.vue'
import UiNumber from '@/components/ui/UiNumber.vue'
import UiSection from '@/components/ui/UiSection.vue'
import UiSegmented from '@/components/ui/UiSegmented.vue'
import UiSelect from '@/components/ui/UiSelect.vue'
import UiSlider from '@/components/ui/UiSlider.vue'
import { textStyleOf } from '@/lib/engine/text'
import { BUNDLED_FONTS, systemFonts, WEIGHT_NAMES, type FontEntry } from '@/lib/fonts'
import { TEXT_PRESETS } from '@/lib/project'
import { previewWords, textPreviewStyle } from '@/lib/textPreview'
import type { TextStyle, TimelineItem } from '@/lib/types'
import { errorMessage, ipc } from '@/services/ipc'
import { useEditor } from '@/stores/editor'
import { useSettings } from '@/stores/settings'
import { useToasts } from '@/stores/toast'
import FontPicker from './FontPicker.vue'

const props = defineProps<{ item: TimelineItem }>()
const editor = useEditor()
const settings = useSettings()
const toasts = useToasts()
const style = computed(() => textStyleOf(props.item))
const draft = ref(style.value.content)
const textarea = ref<HTMLTextAreaElement | null>(null)
const rewriting = ref(false)
let editingStarted = false

watch(() => props.item.id, () => { draft.value = style.value.content; editingStarted = false })
watch(() => style.value.content, value => { if (document.activeElement !== textarea.value) draft.value = value })

const installed = ref<FontEntry[]>([])
const loadingFonts = ref(true)
onMounted(async () => {
  installed.value = await systemFonts()
  loadingFonts.value = false
})

/** Weights the chosen font has; the browser would only fake the others. */
const weightOptions = computed(() => {
  const font = [...BUNDLED_FONTS, ...installed.value].find(candidate => candidate.family === style.value.font)
  const weights = new Set(font?.weights ?? [400, 500, 600, 700, 800, 900])
  weights.add(style.value.weight)
  return [...weights].sort((a, b) => a - b).map(value => ({ value, label: WEIGHT_NAMES[value] ?? String(value) }))
})
const alignOptions = [
  { value: 'left' as const, icon: AlignLeft, title: 'Align left' },
  { value: 'center' as const, icon: AlignCenter, title: 'Center' },
  { value: 'right' as const, icon: AlignRight, title: 'Align right' }
]
const isCaption = computed(() => props.item.groupId?.startsWith('captions-'))
const highlightOptions = [{ value: 'none' as const, label: 'None' }, { value: 'color' as const, label: 'Color' }, { value: 'box' as const, label: 'Box' }]
const wordCount = computed(() => Math.max(1, style.value.content.split(/\s+/).filter(Boolean).length))

function set(patch: Partial<TextStyle>, record = true) {
  editor.updateText(patch, record)
}

function onInput() {
  if (!editingStarted) {
    editor.beginChange()
    editingStarted = true
  }
  editor.updateText({ content: draft.value }, false)
}

function applyPreset(id: string) {
  const preset = TEXT_PRESETS.find(candidate => candidate.id === id)
  if (!preset) return
  editor.applyTextPreset(preset)
}

function applyToAll() {
  const count = editor.applyStyleToGroup()
  toasts.success(`Style applied to ${count} captions`)
}

async function rewrite() {
  rewriting.value = true
  try {
    const text = await ipc.aiRewrite(style.value.content, 'Make it punchier for a short video caption. Keep it about the same length.')
    editor.updateText({ content: text })
    draft.value = text
  } catch (error) {
    toasts.error('Rewrite failed', errorMessage(error))
  } finally {
    rewriting.value = false
  }
}

function focus() {
  textarea.value?.focus()
  textarea.value?.select()
}

defineExpose({ focus })
</script>

<template>
  <UiSection title="Text">
    <template #actions>
      <UiButton v-if="settings.key.hasKey" variant="ghost" size="sm" :disabled="rewriting || !style.content.trim()" @click="rewrite"><Sparkles class="size-3" />{{ rewriting ? 'Rewriting…' : 'Rewrite' }}</UiButton>
    </template>
    <textarea ref="textarea" v-model="draft" class="field min-h-[72px]" rows="3" aria-label="Text content" @input="onInput" @blur="editingStarted = false" />
    <UiButton v-if="isCaption" variant="outline" size="sm" class="w-full" @click="applyToAll">Apply this style to all captions</UiButton>
  </UiSection>

  <UiSection title="Style presets" collapsible :default-open="false">
    <div class="grid max-h-[300px] grid-cols-2 gap-1.5 overflow-y-auto pr-0.5">
      <button v-for="preset in TEXT_PRESETS" :key="preset.id" class="flex h-12 items-center justify-center overflow-hidden rounded-lg border border-line bg-[#24262e] px-2 transition-colors hover:border-line-strong" :title="preset.label" @click="applyPreset(preset.id)">
        <span class="truncate" :style="textPreviewStyle(preset.style, 13)"><span v-for="(word, index) in previewWords({ ...preset.style, highlightIndex: 1 }, preset.label)" :key="index" :style="word.style">{{ word.text }}</span></span>
      </button>
    </div>
  </UiSection>

  <UiSection title="Font">
    <FontPicker :model-value="style.font" :installed="installed" :loading="loadingFonts" @update:model-value="value => set({ font: value })" />
    <div class="grid grid-cols-[1fr_88px] gap-2">
      <UiSelect :model-value="style.weight" :options="weightOptions" label="Font weight" @update:model-value="value => set({ weight: value })" />
      <UiNumber :model-value="style.size" label="Size" :min="8" :max="600" :step="1" :scrub="0.5" @start="editor.beginChange()" @update:model-value="value => set({ size: value }, false)" />
    </div>
    <div class="grid grid-cols-[1fr_auto] gap-2">
      <UiColor :model-value="style.color" label="Text color" @start="editor.beginChange()" @update:model-value="value => set({ color: value }, false)" />
      <div class="flex gap-1">
        <UiSegmented :model-value="style.align" :options="alignOptions" label="Alignment" @update:model-value="value => set({ align: value })" />
        <button class="grid size-8 place-items-center rounded-lg border transition-colors" :class="style.italic ? 'border-accent text-fg' : 'border-line text-muted hover:text-fg'" title="Italic" aria-label="Italic" @click="set({ italic: !style.italic })"><Italic class="size-3.5" /></button>
        <button class="grid size-8 place-items-center rounded-lg border transition-colors" :class="style.uppercase ? 'border-accent text-fg' : 'border-line text-muted hover:text-fg'" title="Uppercase" aria-label="Uppercase" @click="set({ uppercase: !style.uppercase })"><CaseUpper class="size-3.5" /></button>
      </div>
    </div>
    <UiSlider :model-value="style.letterSpacing" label="Spacing" :display="style.letterSpacing.toFixed(0)" :min="-20" :max="60" :step="1" :center="0" @start="editor.beginChange()" @update:model-value="value => set({ letterSpacing: value }, false)" />
    <UiSlider :model-value="style.lineHeight" label="Line height" :display="style.lineHeight.toFixed(2)" :min="0.7" :max="2.5" :step="0.01" @start="editor.beginChange()" @update:model-value="value => set({ lineHeight: value }, false)" />
    <UiSlider :model-value="style.maxWidth" label="Box width" :display="`${Math.round(style.maxWidth)}%`" :min="10" :max="100" :step="1" @start="editor.beginChange()" @update:model-value="value => set({ maxWidth: value }, false)" />
  </UiSection>

  <UiSection title="Word highlight" collapsible :default-open="style.highlightMode !== 'none'">
    <UiSegmented :model-value="style.highlightMode" :options="highlightOptions" label="Highlight" @update:model-value="value => set({ highlightMode: value, highlightIndex: style.highlightIndex >= 0 ? style.highlightIndex : 0 })" />
    <template v-if="style.highlightMode !== 'none'">
      <UiField label="Color"><UiColor :model-value="style.highlightColor" label="Highlight color" @start="editor.beginChange()" @update:model-value="value => set({ highlightColor: value }, false)" /></UiField>
      <UiField v-if="style.highlightMode === 'box'" label="Word color"><UiColor :model-value="style.highlightTextColor || style.color" label="Highlighted word color" @start="editor.beginChange()" @update:model-value="value => set({ highlightTextColor: value }, false)" /></UiField>
      <UiField v-if="!isCaption" label="Word"><UiNumber :model-value="style.highlightIndex + 1" :min="1" :max="wordCount" :step="1" @start="editor.beginChange()" @update:model-value="value => set({ highlightIndex: value - 1 }, false)" /></UiField>
      <p v-else class="text-2xs leading-4 text-faint">Captions light up each word as it is spoken.</p>
    </template>
  </UiSection>

  <UiSection title="Stroke" collapsible :default-open="style.strokeWidth > 0">
    <UiField label="Color"><UiColor :model-value="style.strokeColor" label="Stroke color" @start="editor.beginChange()" @update:model-value="value => set({ strokeColor: value }, false)" /></UiField>
    <UiSlider :model-value="style.strokeWidth" label="Width" :display="style.strokeWidth.toFixed(1)" :min="0" :max="30" :step="0.5" @start="editor.beginChange()" @update:model-value="value => set({ strokeWidth: value }, false)" />
  </UiSection>

  <UiSection title="Background" collapsible :default-open="style.backgroundOpacity > 0">
    <UiField label="Color"><UiColor :model-value="style.backgroundColor" label="Background color" @start="editor.beginChange()" @update:model-value="value => set({ backgroundColor: value, backgroundOpacity: style.backgroundOpacity || 1 }, false)" /></UiField>
    <UiSlider :model-value="style.backgroundOpacity * 100" label="Opacity" :display="`${Math.round(style.backgroundOpacity * 100)}%`" :min="0" :max="100" :step="1" @start="editor.beginChange()" @update:model-value="value => set({ backgroundOpacity: value / 100 }, false)" />
    <UiSlider :model-value="style.backgroundPadding" label="Padding" :display="style.backgroundPadding.toFixed(0)" :min="0" :max="80" :step="1" @start="editor.beginChange()" @update:model-value="value => set({ backgroundPadding: value }, false)" />
    <UiSlider :model-value="style.backgroundRadius" label="Corners" :display="style.backgroundRadius.toFixed(0)" :min="0" :max="80" :step="1" @start="editor.beginChange()" @update:model-value="value => set({ backgroundRadius: value }, false)" />
  </UiSection>

  <UiSection title="Shadow" collapsible :default-open="style.shadowOpacity > 0">
    <UiField label="Color"><UiColor :model-value="style.shadowColor" label="Shadow color" @start="editor.beginChange()" @update:model-value="value => set({ shadowColor: value, shadowOpacity: style.shadowOpacity || 0.8 }, false)" /></UiField>
    <UiSlider :model-value="style.shadowOpacity * 100" label="Opacity" :display="`${Math.round(style.shadowOpacity * 100)}%`" :min="0" :max="100" :step="1" @start="editor.beginChange()" @update:model-value="value => set({ shadowOpacity: value / 100 }, false)" />
    <UiSlider :model-value="style.shadowBlur" label="Blur" :display="style.shadowBlur.toFixed(0)" :min="0" :max="80" :step="1" @start="editor.beginChange()" @update:model-value="value => set({ shadowBlur: value }, false)" />
    <UiSlider :model-value="style.shadowDistance" label="Distance" :display="style.shadowDistance.toFixed(0)" :min="0" :max="60" :step="1" @start="editor.beginChange()" @update:model-value="value => set({ shadowDistance: value }, false)" />
  </UiSection>
</template>
