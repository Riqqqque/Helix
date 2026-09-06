import type { ComponentChildren } from 'preact';
import { Icon, type IconName } from './icons';
import { InfoTip } from './info-tip';

export function PageHead({ title, detail, actions }: { title: string; detail: string; actions?: ComponentChildren }) {
  return <header class="page-head"><div><span class="eyebrow">HELIX</span><h1>{title}</h1><p>{detail}</p></div>{actions !== undefined && <div class="page-head-actions">{actions}</div>}</header>;
}

export function InlineError({ message }: { message: string | null }) {
  if (message === null) return null;
  return <div class="inline-error" role="alert"><Icon name="warning" size={15} />{message}</div>;
}

export function ProgressBar({ value, tone = 'normal' }: { value: number; tone?: 'normal' | 'warning' | 'danger' }) {
  const safe = Math.max(0, Math.min(100, value));
  return <span class={`progress progress--${tone}`}><i style={{ width: `${safe}%` }} /></span>;
}

export function toneForPercent(value: number): 'normal' | 'warning' | 'danger' {
  if (value >= 90) return 'danger';
  if (value >= 75) return 'warning';
  return 'normal';
}

export function Metric({ icon, label, value, detail, percent, help }: { icon: IconName; label: string; value: string; detail: string; percent?: number | undefined; help?: string | undefined }) {
  return (
    <div class="metric-block">
      <span class="metric-icon"><Icon name={icon} size={16} /></span>
      <div class="metric-copy">
        <span>{label}{help !== undefined && <InfoTip text={help} />}</span>
        <strong>{value}</strong><small>{detail}</small>
        {percent !== undefined && <ProgressBar value={percent} tone={toneForPercent(percent)} />}
      </div>
    </div>
  );
}

export function Sparkline({ values, label }: { values: Array<number | null>; label: string }) {
  const points = values.map((value, index) => ({ index, value })).filter((item): item is { index: number; value: number } => item.value !== null && Number.isFinite(item.value));
  if (points.length < 2) {
    return <div class="sparkline sparkline--empty" aria-label={`${label} graph unavailable`}>Waiting for samples</div>;
  }
  const width = 240;
  const height = 56;
  const min = Math.min(...points.map((item) => item.value));
  const max = Math.max(...points.map((item) => item.value));
  const span = Math.max(0.1, max - min);
  const last = values.length - 1;
  const d = points.map((item, index) => {
    const x = last <= 0 ? 0 : (item.index / last) * width;
    const y = height - ((item.value - min) / span) * (height - 6) - 3;
    return `${index === 0 ? 'M' : 'L'}${x.toFixed(1)} ${y.toFixed(1)}`;
  }).join(' ');
  return (
    <svg class="sparkline" viewBox={`0 0 ${width} ${height}`} role="img" aria-label={label}>
      <path d={d} fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  );
}
