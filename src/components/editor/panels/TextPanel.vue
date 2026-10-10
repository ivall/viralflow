<script setup lang="ts">
import { Plus } from '@lucide/vue'
import UiButton from '@/components/ui/UiButton.vue'
import { beginDrag } from '@/lib/dnd'
import { TEXT_PRESETS, textItemDefaults, type TextPreset } from '@/lib/project'
import { useEditor } from '@/stores/editor'

const emit = defineEmits<{ edit: [] }>()
const editor = useEditor()

function add(preset?: TextPreset) {
  editor.addItem(textItemDefaults(preset ? preset.label : 'Your text', preset))
  emit('edit')
}

function onPointer(event: PointerEvent, preset: TextPreset) {
  const defaults = textItemDefaults(preset.label, preset)
  beginDrag(event, { type: 'item', defaults, duration: defaults.duration ?? 3 }, preset.label, () => add(preset))
}

function previewStyle(preset: TextPreset) {
  const style = preset.style
  return {
    fontFamily: `'${style.font}'`,
    fontWeight: String(style.weight ?? 700),
    fontStyle: style.italic ? 'italic' : 'normal',
    color: style.color ?? '#ffffff',
    textTransform: style.uppercase ? 'uppercase' as const : 'none' as const,
    WebkitTextStroke: style.strokeWidth ? `${Math.min(2, style.strokeWidth / 4)}px ${style.strokeColor}` : undefined,
    paintOrder: 'stroke fill',
    background: style.backgroundOpacity ? style.backgroundColor : undefined,
    padding: style.backgroundOpacity ? '2px 8px' : undefined,
    borderRadius: style.backgroundOpacity ? `${Math.min(8, (style.backgroundRadius ?? 0) / 2)}px` : undefined,
    textShadow: style.shadowOpacity ? `0 ${(style.shadowDistance ?? 0) / 3}px ${(style.shadowBlur ?? 0) / 3}px ${style.shadowColor}` : undefined
  }
}
</script>

<template>
  <div class="h-full overflow-y-auto p-3">
    <UiButton variant="primary" class="w-full" @click="add()"><Plus class="size-3.5" />Add text</UiButton>
    <h3 class="mb-2 mt-5 text-xs font-semibold">Styles</h3>
    <div class="grid grid-cols-2 gap-2">
      <button
        v-for="preset in TEXT_PRESETS" :key="preset.id"
        class="flex h-16 items-center justify-center overflow-hidden rounded-md border border-line bg-app px-2 transition-colors hover:border-line-strong"
        :title="`${preset.label} - click to add, or drag to the timeline`"
        @pointerdown="onPointer($event, preset)"
      >
        <span class="max-w-full truncate text-base leading-tight" :style="previewStyle(preset)">{{ preset.label }}</span>
      </button>
    </div>
  </div>
</template>
