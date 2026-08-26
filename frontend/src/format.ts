const compactNumber = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 1,
});

const percentNumber = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 0,
  maximumFractionDigits: 1,
});

const timestampFormat = new Intl.DateTimeFormat(undefined, {
  dateStyle: 'medium',
  timeStyle: 'medium',
});

const decimalSeparator =
  new Intl.NumberFormat(undefined, { minimumFractionDigits: 1 })
    .formatToParts(1.1)
    .find((part) => part.type === 'decimal')?.value ?? '.';

const binaryUnits = [
  { label: 'KiB', bytes: 1_024n },
  { label: 'MiB', bytes: 1_048_576n },
  { label: 'GiB', bytes: 1_073_741_824n },
  { label: 'TiB', bytes: 1_099_511_627_776n },
  { label: 'PiB', bytes: 1_125_899_906_842_624n },
  { label: 'EiB', bytes: 1_152_921_504_606_846_976n },
] as const;

function formatBigIntBytes(bytes: bigint): string {
  if (bytes < 1_024n) {
    return `${compactNumber.format(bytes)} B`;
  }

  let unit: (typeof binaryUnits)[number] = binaryUnits[0];
  for (const candidate of binaryUnits) {
    if (bytes < candidate.bytes) {
      break;
    }
    unit = candidate;
  }

  const roundedTenths = (bytes * 10n + unit.bytes / 2n) / unit.bytes;
  const whole = roundedTenths / 10n;
  const fraction = roundedTenths % 10n;
  const value =
    fraction === 0n
      ? compactNumber.format(whole)
      : `${compactNumber.format(whole)}${decimalSeparator}${fraction}`;
  return `${value} ${unit.label}`;
}

export function formatBytes(bytes: number | bigint): string {
  if (typeof bytes === 'bigint') {
    return bytes < 0n ? 'Unavailable' : formatBigIntBytes(bytes);
  }

  if (!Number.isFinite(bytes) || bytes < 0) {
    return 'Unavailable';
  }

  if (bytes < 1_024) {
    return `${compactNumber.format(bytes)} B`;
  }

  const units = ['KiB', 'MiB', 'GiB', 'TiB', 'PiB'] as const;
  let value = bytes / 1_024;
  let unitIndex = 0;

  while (value >= 1_024 && unitIndex < units.length - 1) {
    value /= 1_024;
    unitIndex += 1;
  }

  return `${compactNumber.format(value)} ${units[unitIndex]}`;
}

export function formatPercent(value: number): string {
  return `${percentNumber.format(value)}%`;
}

export function formatDuration(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) {
    return 'Unavailable';
  }

  const seconds = Math.floor(totalSeconds);
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  const remainingSeconds = seconds % 60;

  if (days > 0) {
    return `${days}d ${hours}h`;
  }

  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  }

  if (minutes > 0) {
    return `${minutes}m ${remainingSeconds}s`;
  }

  return `${remainingSeconds}s`;
}

export function formatTimestamp(timestampUnixMs: number): string {
  if (!Number.isFinite(timestampUnixMs) || timestampUnixMs < 0) {
    return 'Unavailable';
  }

  return timestampFormat.format(new Date(timestampUnixMs));
}

export function calculatePercent(
  used: number | bigint,
  total: number | bigint,
): number | null {
  if (typeof used === 'bigint' || typeof total === 'bigint') {
    if (typeof used !== 'bigint' || typeof total !== 'bigint' || used < 0n || total <= 0n) {
      return null;
    }

    const roundedTenths = (used * 1_000n + total / 2n) / total;
    return Number(roundedTenths) / 10;
  }

  if (!Number.isFinite(used) || !Number.isFinite(total) || total <= 0) {
    return null;
  }

  return (used / total) * 100;
}
