<script setup lang="ts">
import { TooltipContent, TooltipPortal, TooltipRoot, TooltipTrigger } from 'reka-ui'

withDefaults(defineProps<{ label: string; shortcut?: string; side?: 'top' | 'bottom' | 'left' | 'right' }>(), { side: 'top' })
</script>

<template>
  <TooltipRoot :delay-duration="450">
    <TooltipTrigger as-child><slot /></TooltipTrigger>
    <TooltipPortal>
      <TooltipContent :side="side" :side-offset="6" class="z-[200] flex items-center gap-2 rounded-md border border-line bg-raised px-2 py-1 text-2xs text-fg shadow-lg">
        {{ label }}<kbd v-if="shortcut" class="rounded border border-line px-1 font-sans text-[11px] text-muted">{{ shortcut }}</kbd>
      </TooltipContent>
    </TooltipPortal>
  </TooltipRoot>
</template>
