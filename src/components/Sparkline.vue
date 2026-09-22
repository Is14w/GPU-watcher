<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{ values: number[]; color?: string; max?: number; height?: number }>(),
  { color: "#65d9ad", max: 100, height: 64 },
);

const points = computed(() => {
  if (!props.values.length) return "";
  const width = 300;
  const step = props.values.length === 1 ? 0 : width / (props.values.length - 1);
  return props.values
    .map((value, index) => {
      const y = props.height - Math.max(0, Math.min(props.max, value)) / props.max * (props.height - 4) - 2;
      return `${(index * step).toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
});

const fillPoints = computed(() => points.value ? `0,${props.height} ${points.value} 300,${props.height}` : "");
</script>

<template>
  <svg class="sparkline" viewBox="0 0 300 64" preserveAspectRatio="none" aria-hidden="true">
    <defs>
      <linearGradient id="spark-fill" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" :stop-color="color" stop-opacity="0.2" />
        <stop offset="100%" :stop-color="color" stop-opacity="0" />
      </linearGradient>
    </defs>
    <polygon v-if="fillPoints" :points="fillPoints" fill="url(#spark-fill)" />
    <polyline v-if="points" :points="points" fill="none" :stroke="color" stroke-width="2" vector-effect="non-scaling-stroke" />
  </svg>
</template>
