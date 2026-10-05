// src/__tests__/setup.ts — Vitest global setup: jest-dom matchers, a matchMedia stub
// (jsdom has none) and an instant mock backend. Outside Tauri the API layer already
// resolves every command from src/lib/mock.ts, so no Tauri mock.
import "@testing-library/jest-dom/vitest";
import { mockTiming } from "../lib/mock";

mockTiming.scale = 0.01;

Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  }),
});
