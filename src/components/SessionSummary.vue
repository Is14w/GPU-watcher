<script setup lang="ts">
import { Activity, Clock3, Gauge, MemoryStick, Timer } from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { applicationStartedAt, sessions } from "../lib/session";

const props = defineProps<{ hostId: string }>();
const now = ref(Date.now());
let timer: number | undefined;

const GPU_COLORS = [
  "#d8799a",
  "#9b87dc",
  "#6597d8",
  "#55a8aa",
  "#77a56a",
  "#d0a14f",
  "#d77d67",
  "#a96ca6",
];

const session = computed(() => sessions[props.hostId]);
const points = computed(() => session.value?.points ?? []);

function average(values: number[]) {
  return values.length ? values.reduce((sum, value) => sum + value, 0) / values.length : 0;
}

const averageUtilization = computed(() => average(points.value.map((point) => point.utilization)));
const peakUtilization = computed(() => points.value.reduce((peak, point) => Math.max(peak, point.utilization), 0));
const activeTime = computed(() => points.value.length
  ? points.value.filter((point) => point.utilization >= 5).length / points.value.length * 100
  : 0);
const averageMemory = computed(() => average(points.value.map((point) => point.memory)));

const gpuRows = computed(() => Object.values(session.value?.gpus ?? {})
  .sort((left, right) => left.index - right.index)
  .map((gpu) => ({
    ...gpu,
    average: average(gpu.utilization),
    peak: gpu.utilization.reduce((value, item) => Math.max(value, item), 0),
    active: gpu.utilization.length
      ? gpu.utilization.filter((value) => value >= 5).length / gpu.utilization.length * 100
      : 0,
    averageMemory: average(gpu.memory),
  })));

// Every sample stays in memory. Only the SVG paths are reduced for long sessions.
const gpuChartSeries = computed(() => {
  const first = session.value?.firstSampleAt ?? 0;
  const last = session.value?.lastSampleAt ?? first;
  const duration = Math.max(1, last - first);

  return gpuRows.value.map((gpu, seriesIndex) => {
    const source = gpu.utilization.map((utilization, index) => ({
      capturedAt: gpu.capturedAt[index] ?? first,
      utilization,
    }));
    const step = Math.max(1, Math.ceil(source.length / 720));
    const rendered = step === 1
      ? source
      : Array.from({ length: Math.ceil(source.length / step) }, (_, index) => {
        const bucket = source.slice(index * step, (index + 1) * step);
        return bucket.reduce((peak, point) => point.utilization > peak.utilization ? point : peak, bucket[0]);
      });
    const path = rendered.map((point, index) => {
      const x = (point.capturedAt - first) / duration * 1000;
      const y = 196 - Math.min(100, Math.max(0, point.utilization)) * 1.72;
      return `${index ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`;
    }).join(" ");

    return {
      uuid: gpu.uuid,
      index: gpu.index,
      name: gpu.name,
      color: GPU_COLORS[seriesIndex % GPU_COLORS.length],
      current: gpu.utilization.length ? gpu.utilization[gpu.utilization.length - 1] : 0,
      path,
    };
  });
});
const firstLabel = computed(() => session.value
  ? new Date(session.value.firstSampleAt).toLocaleTimeString("zh-CN", { hour12: false })
  : "--:--:--");
const lastLabel = computed(() => session.value
  ? new Date(session.value.lastSampleAt).toLocaleTimeString("zh-CN", { hour12: false })
  : "--:--:--");

function formatDuration(milliseconds: number) {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor(seconds % 3600 / 60);
  const remainder = seconds % 60;
  return [hours, minutes, remainder].map((value) => String(value).padStart(2, "0")).join(":");
}

onMounted(() => {
  timer = window.setInterval(() => { now.value = Date.now(); }, 1000);
});
onBeforeUnmount(() => window.clearInterval(timer));
</script>

<template>
  <section class="session-summary">
    <header class="session-heading">
      <div>
        <span>SESSION SUMMARY</span>
        <h2>本次运行总结</h2>
      </div>
      <div class="session-duration"><Timer :size="15" />{{ formatDuration(now - applicationStartedAt) }}</div>
    </header>

    <div v-if="points.length" class="session-stat-grid">
      <div><Activity :size="17" /><span>平均利用率</span><strong>{{ averageUtilization.toFixed(1) }}%</strong></div>
      <div><Gauge :size="17" /><span>峰值利用率</span><strong>{{ peakUtilization.toFixed(0) }}%</strong></div>
      <div><Clock3 :size="17" /><span>活跃时间</span><strong>{{ activeTime.toFixed(1) }}%</strong></div>
      <div><MemoryStick :size="17" /><span>平均显存占用</span><strong>{{ averageMemory.toFixed(1) }}%</strong></div>
    </div>

    <article v-if="points.length" class="session-chart-card">
      <header>
        <div><span>各 GPU 利用率</span></div>
        <small>{{ points.length }} 个采样点</small>
      </header>
      <div class="session-chart-legend">
        <div v-for="series in gpuChartSeries" :key="series.uuid" :title="series.name">
          <i :style="{ backgroundColor: series.color }" />
          <span>GPU {{ series.index }}</span>
          <strong>{{ series.current.toFixed(0) }}%</strong>
        </div>
      </div>
      <svg viewBox="0 0 1000 220" preserveAspectRatio="none" role="img" aria-label="本次运行 GPU 利用率曲线">
        <line
          v-for="level in [25, 50, 75, 100]"
          :key="level"
          x1="0"
          x2="1000"
          :y1="196 - level * 1.72"
          :y2="196 - level * 1.72"
          class="session-grid-line"
        />
        <path
          v-for="series in gpuChartSeries"
          :key="series.uuid"
          :d="series.path"
          :style="{ stroke: series.color }"
          class="session-line"
        />
      </svg>
      <footer><span>{{ firstLabel }}</span><span>{{ lastLabel }}</span></footer>
    </article>

    <div v-if="points.length" class="session-gpu-grid">
      <article v-for="gpu in gpuRows" :key="gpu.uuid" class="session-gpu-card">
        <header><span>GPU {{ gpu.index }}</span><strong>{{ gpu.name }}</strong></header>
        <div><span>平均</span><strong>{{ gpu.average.toFixed(1) }}%</strong></div>
        <div><span>峰值</span><strong>{{ gpu.peak.toFixed(0) }}%</strong></div>
        <div><span>活跃</span><strong>{{ gpu.active.toFixed(1) }}%</strong></div>
        <div><span>显存</span><strong>{{ gpu.averageMemory.toFixed(1) }}%</strong></div>
      </article>
    </div>

    <div v-else class="session-empty">
      <Activity :size="25" />
      <h3>等待第一条 GPU 数据</h3>
    </div>
  </section>
</template>
