import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface CameraConfig {
  name: string;
  cameraEntity: string;
  motionEntities: string[];
}

export interface Config {
  haUrl: string;
  cloudUrl: string;
  token: string;
  cameras: CameraConfig[];
  corner: string;
  monitor: string;
  margin: number;
  width: number;
  height: number;
  dismissSeconds: number;
}

export interface ConnConfig {
  haUrl: string;
  cloudUrl: string;
  token: string;
}

export interface EntityOption {
  entityId: string;
  name: string;
  deviceClass?: string | null;
}

export interface MonitorInfo {
  name: string;
  width: number;
  height: number;
  primary: boolean;
}

export interface TestResult {
  ok: boolean;
  error?: string;
  results?: { label: string; url: string; ok: boolean; error?: string }[];
}

export interface EntitiesResult {
  ok: boolean;
  error?: string;
  cameras?: EntityOption[];
  motion?: EntityOption[];
}

export interface SaveResult {
  ok: boolean;
  error?: string;
}

export interface OverlayShow {
  cameraEntity: string;
  name: string;
  label: string;
  detail: string;
  deviceClass: string | null;
  sound: boolean;
  showLabels: boolean;
  draggable: boolean;
}

export const api = {
  setupLoad: () => invoke<Config | null>("setup_load"),
  listMonitors: () => invoke<MonitorInfo[]>("list_monitors"),
  setupTest: (config: ConnConfig) => invoke<TestResult>("setup_test", { config }),
  setupEntities: (config: ConnConfig) => invoke<EntitiesResult>("setup_entities", { config }),
  setupSave: (config: Config) => invoke<SaveResult>("setup_save", { config }),
  setupCancel: () => invoke("setup_cancel"),
  overlayReady: (label: string) => invoke("overlay_ready", { label }),
  overlayPresent: (label: string) => invoke("overlay_present", { label }),
  overlayClose: (label: string) => invoke("overlay_close", { label }),
  overlayHide: (label: string) => invoke("overlay_hide", { label }),
  webrtcOffer: (label: string, cameraEntity: string, sdp: string) =>
    invoke("webrtc_offer", { label, cameraEntity, sdp }),
  webrtcCandidate: (label: string, candidate: unknown) =>
    invoke("webrtc_candidate", { label, candidate }),
  webrtcStop: (label: string) => invoke("webrtc_stop", { label }),
};

export { listen };
export type { UnlistenFn };
