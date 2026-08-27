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
