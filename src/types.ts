export type AuthMethod = "password" | "privateKey";

export interface SavedHost {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  privateKeyPath?: string;
  expectedFingerprint?: string;
  source?: "manual" | "ssh-config";
}

export interface RuntimeSecret {
  password?: string;
  passphrase?: string;
}

export interface SshTarget {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  password?: string;
  privateKeyPath?: string;
  passphrase?: string;
  expectedFingerprint?: string;
}

export interface ConnectionReport {
  fingerprint: string;
  nvidiaSmiAvailable: boolean;
  nvitopAvailable: boolean;
  hostname: string;
}

export interface SshConfigHost {
  alias: string;
  host: string;
  port: number;
  username: string;
  identityFile?: string;
}

export interface GpuMetric {
  index: number;
  uuid: string;
  name: string;
  driverVersion: string;
  pstate: string;
  temperature?: number;
  gpuUtilization?: number;
  memoryUtilization?: number;
  memoryUsed?: number;
  memoryTotal?: number;
  powerDraw?: number;
  powerLimit?: number;
  fanSpeed?: number;
  graphicsClock?: number;
  memoryClock?: number;
}

export interface GpuProcess {
  gpuUuid: string;
  pid: number;
  processName: string;
  usedMemory?: number;
}

export interface Snapshot {
  capturedAt: number;
  gpus: GpuMetric[];
  processes: GpuProcess[];
}

export interface MonitorEvent {
  targetId: string;
  runId: string;
  kind: "connecting" | "connected" | "snapshot" | "error" | "stopped";
  snapshot?: Snapshot;
  message?: string;
}

export interface TerminalEvent {
  targetId: string;
  runId: string;
  kind: "connecting" | "connected" | "output" | "error" | "stopped";
  data?: string;
  message?: string;
}
