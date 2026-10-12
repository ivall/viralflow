<script setup lang="ts">
import { TooltipProvider } from 'reka-ui'
import { onMounted } from 'vue'
import TitleBar from '@/components/TitleBar.vue'
import UiToaster from '@/components/ui/UiToaster.vue'
import { useSettings } from '@/stores/settings'

const settings = useSettings()
onMounted(() => settings.load())
</script>

<template>
  <TooltipProvider :delay-duration="450">
    <div class="flex h-full flex-col">
      <TitleBar />
      <div class="min-h-0 flex-1"><RouterView v-if="settings.loaded" /></div>
    </div>
    <UiToaster />
  </TooltipProvider>
</template>
