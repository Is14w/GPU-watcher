import { reactive } from "vue";
import type { Snapshot } from "../types";

export interface SessionPoint {
  capturedAt: number;
  utilization: number;
  memory: number;
}

export interface GpuSessionSeries {
  uuid: string;
  index: number;
  name: string;
  capturedAt: number[];
  utilization: number[];
  memory: number[];
}

export interface HostSession {
  firstSampleAt: number;
  lastSampleAt: number;
  points: SessionPoint[];
  gpus: Record<string, GpuSessionSeries>;
}

export const applicationStartedAt = Date.now();
export const sessions = reactive<Record<string, HostSession>>({});

export function recordSessionSnapshot(hostId: string, snapshot: Snapshot) {
  const utilization = snapshot.gpus.length
    ? snapshot.gpus.reduce((sum, gpu) => sum + (gpu.gpuUtilization ?? 0), 0) / snapshot.gpus.length
    : 0;
  const totalMemory = snapshot.gpus.reduce((sum, gpu) => sum + (gpu.memoryTotal ?? 0), 0);
  const usedMemory = snapshot.gpus.reduce((sum, gpu) => sum + (gpu.memoryUsed ?? 0), 0);
  const memory = totalMemory > 0 ? usedMemory / totalMemory * 100 : 0;

  const session = sessions[hostId] ?? (sessions[hostId] = {
    firstSampleAt: snapshot.capturedAt,
    lastSampleAt: snapshot.capturedAt,
    points: [],
    gpus: {},
  });

  session.lastSampleAt = snapshot.capturedAt;
  session.points.push({ capturedAt: snapshot.capturedAt, utilization, memory });

  for (const gpu of snapshot.gpus) {
    const series = session.gpus[gpu.uuid] ?? (session.gpus[gpu.uuid] = {
      uuid: gpu.uuid,
      index: gpu.index,
      name: gpu.name,
      capturedAt: [],
      utilization: [],
      memory: [],
    });
    series.capturedAt.push(snapshot.capturedAt);
    series.utilization.push(gpu.gpuUtilization ?? 0);
    series.memory.push((gpu.memoryTotal ?? 0) > 0 ? (gpu.memoryUsed ?? 0) / (gpu.memoryTotal ?? 1) * 100 : 0);
  }
}
