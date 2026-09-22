<script setup lang="ts">
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X } from "@lucide/vue";

const appWindow = getCurrentWindow();
</script>

<template>
  <div class="window-controls" aria-label="窗口控制">
    <button aria-label="最小化" title="最小化" @click="appWindow.minimize()"><Minus :size="14" /></button>
    <button aria-label="最大化或还原" title="最大化或还原" @click="appWindow.toggleMaximize()"><Square :size="12" /></button>
    <button class="window-close" aria-label="关闭" title="关闭" @click="appWindow.close()"><X :size="15" /></button>
  </div>
</template>
