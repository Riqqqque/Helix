import { describe, expect, it } from "vitest";
import {
  cpuLimitOptions,
  cpuLimitOptionsForCurrent,
  formatCpuLimit,
  cpuMillisFields,
} from "./container-resources";

describe("container CPU limits", () => {
  it("always offers no extra cap and stays inside the host core count", () => {
    const options = cpuLimitOptions(8);
    expect(options[0]).toEqual({ value: 0, label: "No extra cap" });
    expect(options.map((option) => option.value)).toContain(1000);
    expect(options.map((option) => option.value)).toContain(8000);
    expect(options.every((option) => option.value <= 8000)).toBe(true);
    expect(cpuLimitOptions(1).map((option) => option.value)).toEqual([0, 250, 500, 1000]);
  });

  it("keeps a custom current cap visible", () => {
    expect(cpuLimitOptionsForCurrent(8, 3000).map((option) => option.value)).toContain(3000);
    expect(formatCpuLimit(0)).toBe("No extra cap");
    expect(formatCpuLimit(1000)).toBe("1 core");
    expect(formatCpuLimit(250)).toBe("0.25 cores");
  });

  it("omits unlimited CPU from create bodies", () => {
    expect(cpuMillisFields(0)).toEqual({});
    expect(cpuMillisFields(2000)).toEqual({ cpu_millis: 2000 });
  });
});
