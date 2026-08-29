export const UNLIMITED_CPU_MILLIS = 0;

export interface CpuLimitOption {
  value: number;
  label: string;
}

export function cpuLimitOptions(logicalCores: number): CpuLimitOption[] {
  const cores =
    Number.isFinite(logicalCores) && logicalCores >= 1
      ? Math.min(Math.floor(logicalCores), 128)
      : 8;
  const options: CpuLimitOption[] = [{ value: UNLIMITED_CPU_MILLIS, label: "No extra cap" }];
  for (const n of [0.25, 0.5, 1, 2, 4, 6, 8, 12, 16, 24, 32]) {
    if (n > cores) continue;
    options.push({
      value: Math.round(n * 1000),
      label: n < 1 ? `${n} cores` : n === 1 ? "1 core" : `${n} cores`,
    });
  }
  const allMillis = cores * 1000;
  if (!options.some((option) => option.value === allMillis)) {
    options.push({ value: allMillis, label: `All ${cores} cores` });
  }
  return options;
}

export function cpuLimitOptionsForCurrent(
  logicalCores: number,
  current: number,
): CpuLimitOption[] {
  const options = cpuLimitOptions(logicalCores);
  if (
    Number.isFinite(current) &&
    current > 0 &&
    !options.some((option) => option.value === current)
  ) {
    options.push({ value: current, label: formatCpuLimit(current) });
    options.sort((a, b) => a.value - b.value);
  }
  return options;
}

export function formatCpuLimit(cpuMillis: number): string {
  if (!cpuMillis) return "No extra cap";
  const cores = cpuMillis / 1000;
  if (Number.isInteger(cores)) return cores === 1 ? "1 core" : `${cores} cores`;
  return `${String(cores.toFixed(2)).replace(/0+$/u, "").replace(/\.$/u, "")} cores`;
}

export function cpuMillisFields(cpuMillis: number): { cpu_millis: number } | Record<string, never> {
  return cpuMillis > 0 ? { cpu_millis: cpuMillis } : {};
}
