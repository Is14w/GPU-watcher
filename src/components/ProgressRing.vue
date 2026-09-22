<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(defineProps<{ value?: number; color?: string; size?: number }>(), {
  value: 0,
  color: "#66dfb2",
  size: 88,
});
const radius = 36;
const circumference = 2 * Math.PI * radius;
const offset = computed(() => circumference * (1 - Math.min(100, Math.max(0, props.value ?? 0)) / 100));
</script>

<template>
  <div class="progress-ring" :style="{ width: `${size}px`, height: `${size}px` }">
    <svg viewBox="0 0 84 84">
      <circle cx="42" cy="42" :r="radius" class="ring-track" />
      <circle
        cx="42"
        cy="42"
        :r="radius"
        class="ring-value"
        :stroke="color"
        :stroke-dasharray="circumference"
        :stroke-dashoffset="offset"
      />
    </svg>
    <strong>{{ Math.round(value ?? 0) }}<small>%</small></strong>
  </div>
</template>
