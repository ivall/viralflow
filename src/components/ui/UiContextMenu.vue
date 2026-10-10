<script setup lang="ts">
import { ContextMenuContent, ContextMenuItem, ContextMenuPortal, ContextMenuRoot, ContextMenuSeparator, ContextMenuTrigger } from 'reka-ui'
import type { MenuItem } from './menu'

defineProps<{ items: MenuItem[] }>()
const emit = defineEmits<{ select: [id: string]; open: [] }>()
</script>

<template>
  <ContextMenuRoot :modal="false" @update:open="open => open && emit('open')">
    <ContextMenuTrigger as-child><slot /></ContextMenuTrigger>
    <ContextMenuPortal>
      <ContextMenuContent class="z-[200] min-w-[200px] rounded-lg border border-line bg-raised p-1 shadow-2xl">
        <template v-for="item in items" :key="item.id">
          <ContextMenuSeparator v-if="item.separatorBefore" class="my-1 h-px bg-line" />
          <ContextMenuItem
            :disabled="item.disabled"
            class="flex h-8 items-center gap-2 rounded-md px-2 text-xs outline-none data-[disabled]:opacity-40 data-[highlighted]:bg-hover"
            :class="item.danger ? 'text-danger' : 'text-fg'"
            @select="emit('select', item.id)"
          >
            <component :is="item.icon" v-if="item.icon" class="size-3.5 opacity-80" />
            <span class="flex-1">{{ item.label }}</span>
            <kbd v-if="item.shortcut" class="font-sans text-2xs text-faint">{{ item.shortcut }}</kbd>
          </ContextMenuItem>
        </template>
      </ContextMenuContent>
    </ContextMenuPortal>
  </ContextMenuRoot>
</template>
