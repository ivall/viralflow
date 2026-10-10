<script setup lang="ts">
import { SwitchRoot, SwitchThumb } from 'reka-ui'

defineProps<{ modelValue: boolean; label?: string }>()
const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>()
</script>

<template>
  <SwitchRoot
    :model-value="modelValue" :aria-label="label"
    class="relative h-[18px] w-8 shrink-0 rounded-full bg-line transition-colors data-[state=checked]:bg-accent"
    @update:model-value="value => emit('update:modelValue', value)"
  >
    <SwitchThumb class="block size-3.5 translate-x-0.5 rounded-full bg-fg shadow transition-transform data-[state=checked]:translate-x-[16px] data-[state=checked]:bg-accent-ink" />
  </SwitchRoot>
</template>
