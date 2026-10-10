<script setup lang="ts">
import { ref, watch } from 'vue'

const props = withDefaults(defineProps<{
  modelValue: number
  min?: number
  max?: number
  step?: number
  precision?: number
  suffix?: string
  label?: string
  /** Value change per dragged pixel when scrubbing the label. */
  scrub?: number
}>(), { min: -Infinity, max: Infinity, step: 1, precision: 0, scrub: 0 })
const emit = defineEmits<{ 'update:modelValue': [value: number]; start: [] }>()
const draft = ref('')
const editing = ref(false)

function format(value: number) {
  return Number.isFinite(value) ? value.toFixed(props.precision) : ''
}

watch(() => props.modelValue, value => { if (!editing.value) draft.value = format(value) }, { immediate: true })

function clampValue(value: number) {
  return Math.min(props.max, Math.max(props.min, value))
}

function commit() {
  editing.value = false
  const parsed = Number.parseFloat(draft.value.replace(',', '.'))
  if (Number.isFinite(parsed)) {
    const next = clampValue(parsed)
    if (Math.abs(next - props.modelValue) > 1e-9) {
      emit('start')
      emit('update:modelValue', next)
    }
  }
  draft.value = format(props.modelValue)
}

function cancel(event: Event) {
  editing.value = false
  draft.value = format(props.modelValue)
  ;(event.target as HTMLInputElement).blur()
}

function step(direction: number, event: KeyboardEvent) {
  event.preventDefault()
  emit('start')
  emit('update:modelValue', clampValue(props.modelValue + direction * props.step * (event.shiftKey ? 10 : 1)))
}

/** Dragging the label scrubs the value, like in desktop editors. */
function startScrub(event: PointerEvent) {
  event.preventDefault()
  const startX = event.clientX
  const startValue = props.modelValue
  const perPixel = props.scrub || props.step
  let started = false
  const move = (moveEvent: PointerEvent) => {
    const delta = moveEvent.clientX - startX
    if (!started && Math.abs(delta) < 2) return
    if (!started) { started = true; emit('start') }
    const raw = startValue + delta * perPixel * (moveEvent.shiftKey ? 10 : 1)
    emit('update:modelValue', clampValue(Math.round(raw / props.step) * props.step))
  }
  const up = () => {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', up)
    document.body.style.cursor = ''
  }
  document.body.style.cursor = 'ew-resize'
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', up)
}
</script>

<template>
  <label class="flex h-[30px] min-w-0 items-center rounded-md border border-line bg-app px-2 transition-colors focus-within:border-accent hover:border-line-strong">
    <span v-if="label" class="mr-1.5 shrink-0 cursor-ew-resize text-2xs text-faint" @pointerdown="startScrub">{{ label }}</span>
    <input
      v-model="draft"
      class="h-full min-w-0 flex-1 bg-transparent text-xs tabular-nums text-fg outline-none"
      inputmode="decimal"
      @focus="editing = true; ($event.target as HTMLInputElement).select()"
      @blur="commit"
      @keydown.enter="($event.target as HTMLInputElement).blur()"
      @keydown.esc="cancel"
      @keydown.up="step(1, $event)"
      @keydown.down="step(-1, $event)"
    >
    <span v-if="suffix" class="ml-1 shrink-0 text-2xs text-faint">{{ suffix }}</span>
  </label>
</template>
