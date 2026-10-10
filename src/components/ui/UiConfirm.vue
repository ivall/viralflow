<script setup lang="ts">
import UiButton from './UiButton.vue'
import UiDialog from './UiDialog.vue'

defineProps<{ open: boolean; title: string; description?: string; confirmLabel: string; danger?: boolean }>()
const emit = defineEmits<{ 'update:open': [value: boolean]; confirm: [] }>()
</script>

<template>
  <UiDialog :open="open" :title="title" width="420px" @update:open="value => emit('update:open', value)">
    <p v-if="description" class="px-5 py-4 text-xs leading-5 text-muted">{{ description }}</p>
    <template #footer>
      <UiButton variant="ghost" @click="emit('update:open', false)">Cancel</UiButton>
      <UiButton :variant="danger ? 'danger' : 'primary'" @click="emit('confirm'); emit('update:open', false)">{{ confirmLabel }}</UiButton>
    </template>
  </UiDialog>
</template>
