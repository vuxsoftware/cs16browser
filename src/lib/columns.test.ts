import { expect, it } from "vitest";
import { BANNED_COLUMNS, COLUMNS, resizeBannedColumns, resolveColumnWidths } from "./columns";

it("keeps the list in its viewport when the Servers column is enlarged", () => {
  const widths = resolveColumnWidths({ name: 1800 }, 1892, "name");
  expect(widths.players).toBe(110);
  expect(widths.game).toBe(40);
  expect(widths.map).toBe(40);
  expect(widths.latency).toBe(80);
  expect(COLUMNS.reduce((sum, col) => sum + widths[col.key], 0)).toBe(1892);
});

it("restores the right columns as Servers is narrowed", () => {
  const widths = resolveColumnWidths({ name: 800 }, 1892, "name");
  expect(widths.game).toBe(224);
  expect(widths.map).toBe(180);
  expect(widths.latency).toBe(126);
});

it("fills unused Banned space with Servers alone", () => {
  const widths = resolveColumnWidths(
    { name: 120, endpoint: 240 },
    1500,
    "endpoint",
    BANNED_COLUMNS,
  );
  expect(widths.name).toBe(724);
  expect(widths.endpoint).toBe(240);
  expect(widths.reason).toBe(500);
  expect(BANNED_COLUMNS.reduce((sum, col) => sum + widths[col.key], 0)).toBe(1500);
});

it("reserves enough Banned space for an IPv4 address and port", () => {
  const widths = resolveColumnWidths({}, 1500, null, BANNED_COLUMNS);
  expect(widths.endpoint).toBe(280);
  expect(widths.reason).toBe(500);
  expect(widths.name).toBe(684);
});

it("resizes both Banned dividers without fixing Reason", () => {
  const nameBoundary = resizeBannedColumns({}, 1500, "name", 650);
  expect(resolveColumnWidths(nameBoundary, 1500, null, BANNED_COLUMNS)).toMatchObject({
    name: 650,
    endpoint: 314,
    reason: 500,
  });

  const ipBoundary = resizeBannedColumns({}, 1500, "endpoint", 330);
  expect(resolveColumnWidths(ipBoundary, 1500, null, BANNED_COLUMNS)).toMatchObject({
    name: 684,
    endpoint: 330,
    reason: 450,
  });
});
