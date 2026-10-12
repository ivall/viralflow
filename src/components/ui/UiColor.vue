<script setup lang="ts">
import { ref, watch } from 'vue'

const props = defineProps<{ modelValue: string; label?: string }>()
const emit = defineEmits<{ 'update:modelValue': [value: string]; start: [] }>()
const draft = ref(props.modelValue)
watch(() => props.modelValue, value => { draft.value = value })

function commitHex() {
  const raw = draft.value.trim()
  const value = raw.startsWith('#') ? raw : `#${raw}`
  if (/^#[\da-f]{6}$/i.test(value)) {
    if (value.toLowerCase() !== props.modelValue.toLowerCase()) {
      emit('start')
      emit('update:modelValue', value.toLowerCase())
    }
  } else draft.value = props.modelValue
}
</script>

<template>
  <div class="flex h-8 min-w-0 items-center gap-2 rounded-lg border border-line bg-raised pl-1 pr-2 transition-colors focus-within:border-accent hover:border-line-strong">
    <label class="relative size-[22px] shrink-0 overflow-hidden rounded border border-white/10" :style="{ background: modelValue }">
      <input type="color" class="absolute inset-0 cursor-pointer opacity-0" :value="modelValue" :aria-label="label" @focus="emit('start')" @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)">
    </label>
    <input v-model="draft" class="min-w-0 flex-1 bg-transparent text-xs uppercase tabular-nums text-fg outline-none" maxlength="7" :aria-label="label" @blur="commitHex" @keydown.enter="($event.target as HTMLInputElement).blur()">
  </div>
</template>
