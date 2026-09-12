import { useState } from 'preact/hooks';
import { ApiError } from './api';
import { InlineError } from './dashboard-ui';
import { HostRebootDialog } from './dashboard-settings';
import { getHostIntegration, type HostIntegration } from './host-api';
import { Icon } from './icons';

export function HostRebootButton({ csrfToken, disabled = false, onSessionExpired }: {
  csrfToken: string;
  disabled?: boolean;
  onSessionExpired: () => void;
}) {
  const [integration, setIntegration] = useState<HostIntegration | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const open = async (): Promise<void> => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      setIntegration(await getHostIntegration(csrfToken));
    } catch (cause) {
      if (cause instanceof ApiError && cause.status === 401) onSessionExpired();
      else setError(cause instanceof Error ? cause.message : 'Could not read the host. Try again.');
    } finally {
      setBusy(false);
    }
  };
  return <div>
    <button type="button" class="button button--danger" disabled={disabled || busy} onClick={() => void open()}>
      <Icon name="refresh" size={15} />{busy ? 'Checking host…' : 'Reboot host'}
    </button>
    <InlineError message={error} />
    {integration !== null && <HostRebootDialog integration={integration} csrfToken={csrfToken} onClose={() => setIntegration(null)} onChanged={async () => setIntegration(await getHostIntegration(csrfToken))} />}
  </div>;
}
