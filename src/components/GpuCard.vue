<script setup lang="ts">
import { Fan, MemoryStick, Thermometer, Zap } from "@lucide/vue";
import type { GpuMetric } from "../types";
import ProgressRing from "./ProgressRing.vue";
import Sparkline from "./Sparkline.vue";

defineProps<{ gpu: GpuMetric; history: number[] }>();

function number(value?: number, digits = 0) {
  return value == null ? "—" : value.toFixed(digits);
}
</script>

<template>
  <article class="gpu-card">
    <header class="gpu-card-header">
      <div>
        <span class="gpu-index">GPU {{ gpu.index }}</span>
        <h3>{{ gpu.name }}</h3>
      </div>
      <span class="pstate">{{ gpu.pstate }}</span>
    </header>

    <div class="gpu-primary">
      <ProgressRing :value="gpu.gpuUtilization" />
      <div class="gpu-chart">
        <div class="chart-label">
          <span>计算负载</span>
          <strong>{{ number(gpu.gpuUtilization) }}%</strong>
        </div>
        <Sparkline :values="history" />
      </div>
    </div>

    <div class="metric-grid">
      <div class="metric-cell">
        <MemoryStick :size="16" />
        <span>显存</span>
        <strong>{{ number(gpu.memoryUsed) }} <small>/ {{ number(gpu.memoryTotal) }} MiB</small></strong>
      </div>
      <div class="metric-cell">
        <Thermometer :size="16" />
        <span>温度</span>
        <strong>{{ number(gpu.temperature) }}<small>°C</small></strong>
      </div>
      <div class="metric-cell">
        <Zap :size="16" />
        <span>功耗</span>
        <strong>{{ number(gpu.powerDraw, 1) }} <small>/ {{ number(gpu.powerLimit, 0) }} W</small></strong>
      </div>
      <div class="metric-cell">
        <Fan :size="16" />
        <span>风扇</span>
        <strong>{{ number(gpu.fanSpeed) }}<small>%</small></strong>
      </div>
    </div>

    <footer class="gpu-card-footer">
      <span>核心 {{ number(gpu.graphicsClock) }} MHz</span>
      <span>显存 {{ number(gpu.memoryClock) }} MHz</span>
      <span>驱动 {{ gpu.driverVersion }}</span>
    </footer>
  </article>
</template>
