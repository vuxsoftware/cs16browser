import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Backend } from "./cs16";

vi.mock("./mock.js", () => ({}));
vi.mock("./bindings", () => ({
  commands: {
    startScan: vi.fn(async () => ({ status: "ok", data: null })),
    stopScan: vi.fn(async () => undefined),
    refreshServer: vi.fn(async () => ({ status: "ok", data: null })),
    serverPlayers: vi.fn(async () => ({ status: "ok", data: [] })),
    clearBans: vi.fn(async () => 2),
    bans: vi.fn(async () => []),
    cachedRows: vi.fn(async () => []),
    connect: vi.fn(async () => ({ status: "ok", data: "steam://connect/1.2.3.4:27015/" })),
    getSettings: vi.fn(async () => ({ api_key_source: "none" })),
    setApiKey: vi.fn(async () => ({ status: "ok", data: { api_key_source: "settings" } })),
    openApiKeyPage: vi.fn(async () => ({ status: "ok", data: null })),
    openRepositoryPage: vi.fn(async () => ({ status: "ok", data: null })),
  },
  events: { scan: { listen: vi.fn(async () => vi.fn()) } },
}));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn(async () => null) }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({
    close: vi.fn(),
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    startResizeDragging: vi.fn(),
  })),
}));

beforeEach(() => {
  vi.resetModules();
  delete (window as Window & { __TAURI__?: object }).__TAURI__;
  delete window.cs16Mock;
});

describe("backend boundary", () => {
  it("adapts synchronous mock commands to promises and skips native updates", async () => {
    const mock = {
      mode: "mock",
      startScan: () => undefined,
      stopScan: () => undefined,
      refreshServer: () => undefined,
      serverPlayers: () => [],
      clearBans: () => 3,
      bans: () => [],
      cachedRows: () => [],
      connect: () => "steam://connect/example/",
      getSettings: () => ({ api_key_source: "none" }),
      setApiKey: () => ({ api_key_source: "settings" }),
      openApiKeyPage: () => undefined,
      openRepositoryPage: () => undefined,
      onEvent: () => () => undefined,
    } as unknown as Backend;
    window.cs16Mock = mock;
    const { cs16, availableUpdate } = await import("./cs16");
    expect(cs16.mode).toBe("mock");
    await expect(cs16.clearBans()).resolves.toBe(3);
    await expect(cs16.connect("example")).resolves.toContain("steam://");
    await expect(availableUpdate()).resolves.toBeNull();
  });

  it("unwraps typed command results and surfaces backend errors", async () => {
    (window as Window & { __TAURI__?: object }).__TAURI__ = {};
    const { commands } = await import("./bindings");
    const { cs16 } = await import("./cs16");
    await cs16.startScan(null);
    await cs16.stopScan();
    await cs16.refreshServer("1.2.3.4:27015", true);
    expect(commands.refreshServer).toHaveBeenCalledWith("1.2.3.4:27015", true);
    await cs16.openApiKeyPage();
    await cs16.openRepositoryPage();
    expect(await cs16.serverPlayers("1.2.3.4:27015")).toEqual([]);
    expect(await cs16.clearBans()).toBe(2);
    expect(await cs16.bans()).toEqual([]);
    expect(await cs16.cachedRows()).toEqual([]);
    expect(await cs16.getSettings()).toHaveProperty("api_key_source", "none");
    expect(await cs16.setApiKey("key")).toHaveProperty("api_key_source", "settings");
    expect(await cs16.connect("1.2.3.4:27015")).toContain("steam://connect/");
    vi.mocked(commands.refreshServer).mockResolvedValueOnce({ status: "error", error: "paced" });
    await expect(cs16.refreshServer("1.2.3.4:27015", false)).rejects.toThrow("paced");
  });

  it("registers a typed event listener and permits cancellation", async () => {
    (window as Window & { __TAURI__?: object }).__TAURI__ = {};
    const { events } = await import("./bindings");
    const { cs16 } = await import("./cs16");
    const sink = vi.fn();
    const off = cs16.onEvent(sink);
    await Promise.resolve();
    const listener = vi.mocked(events.scan.listen).mock.calls[0][0];
    listener({ payload: { kind: "phase", phase: "done" } } as Parameters<typeof listener>[0]);
    expect(sink).toHaveBeenCalledWith({ kind: "phase", phase: "done" });
    off();
  });

  it("checks, installs and relaunches only when an update is available", async () => {
    (window as Window & { __TAURI__?: object }).__TAURI__ = {};
    const { check } = await import("@tauri-apps/plugin-updater");
    const { relaunch } = await import("@tauri-apps/plugin-process");
    const update = { version: "0.2.0", downloadAndInstall: vi.fn(async () => undefined) };
    vi.mocked(check).mockResolvedValueOnce(update as never);
    const { availableUpdate, installUpdate } = await import("./cs16");
    expect(await availableUpdate()).toBe(update);
    await installUpdate(update as never);
    expect(update.downloadAndInstall).toHaveBeenCalledOnce();
    expect(relaunch).toHaveBeenCalledOnce();
  });
});
