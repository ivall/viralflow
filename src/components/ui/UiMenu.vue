<script setup lang="ts">
import { DropdownMenuContent, DropdownMenuItem, DropdownMenuPortal, DropdownMenuRoot, DropdownMenuSeparator, DropdownMenuTrigger } from 'reka-ui'
import type { MenuItem } from './menu'

withDefaults(defineProps<{ items: MenuItem[]; side?: 'top' | 'bottom' | 'left' | 'right'; align?: 'start' | 'center' | 'end' }>(), { side: 'bottom', align: 'end' })
const emit = defineEmits<{ select: [id: string] }>()
</script>

<template>
  <DropdownMenuRoot :modal="false">
    <DropdownMenuTrigger as-child><slot /></DropdownMenuTrigger>
    <DropdownMenuPortal>
      <DropdownMenuContent :side="side" :align="align" :side-offset="4" class="z-[200] min-w-[180px] rounded-lg border border-line bg-raised p-1 shadow-2xl">
        <template v-for="item in items" :key="item.id">
          <DropdownMenuSeparator v-if="item.separatorBefore" class="my-1 h-px bg-line" />
          <DropdownMenuItem
            :disabled="item.disabled"
            class="flex h-8 items-center gap-2 rounded-md px-2 text-xs outline-none data-[disabled]:opacity-40 data-[highlighted]:bg-hover"
            :class="item.danger ? 'text-danger' : 'text-fg'"
            @select="emit('select', item.id)"
          >
            <component :is="item.icon" v-if="item.icon" class="size-3.5 opacity-80" />
            <span class="flex-1">{{ item.label }}</span>
            <kbd v-if="item.shortcut" class="font-sans text-2xs text-faint">{{ item.shortcut }}</kbd>
          </DropdownMenuItem>
        </template>
      </DropdownMenuContent>
    </DropdownMenuPortal>
  </DropdownMenuRoot>
</template>
