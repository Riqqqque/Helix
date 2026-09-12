import { render } from 'preact';
import { useState } from 'preact/hooks';
import { HostRebootDialog } from '../src/dashboard-settings';
import type { HostIntegration } from '../src/host-api';
import '../src/styles.css';

// No network or host commands: requests in this fixture never leave the page.
let calls = 0;
const mode = new URLSearchParams(location.search).get('mode');
const preflight = {
  schema_version: 1, can_schedule: mode !== 'blocked', active_players: 0, active_server_count: 2,
  active_servers: [], active_servers_truncated: false, active_jobs_total: mode === 'blocked' ? 1 : 0,
  active_jobs: [], blockers: mode === 'blocked' ? [{ code: 'active_jobs', message: 'Wait for the backup to finish.' }] : [], checked_at_unix_ms: Date.now(),
};
window.fetch = async (input, init) => {
  if (input === '/api/v1/host/reboot/preflight' && (init?.method ?? 'GET') === 'GET') return Response.json(preflight);
  if (input === '/api/v1/host/reboot' && init?.method === 'POST') {
    calls++;
    const status = document.getElementById('calls');
    if (status) status.textContent = `Reboot requests: ${calls}`;
    const body = JSON.parse(String(init.body));
    if (body.delay_seconds !== 0 || body.confirmation_hostname !== 'example-host' || !body.disruption_acknowledged) throw new Error('Invalid reboot request');
    if (mode === 'disconnect') throw new TypeError('Connection lost');
    return Response.json({ operation_id: '6b8f95ce-9c58-4c4c-b232-627a29ca1c03', state: 'scheduled', hostname: 'example-host', scheduled_at_unix_ms: Date.now(), execute_at_unix_ms: Date.now(), delay_seconds: 0, cancellable: false, timer_backend: 'systemd_transient_service', preflight });
  }
  throw new Error(`Unexpected fixture request: ${String(input)}`);
};
function App() {
  const [open, setOpen] = useState(false);
  return <main style={{ padding: '32px' }}><h1>Host reboot test</h1><p>Isolated fixture. No host will restart.</p><p id="calls">Reboot requests: 0</p>
    <button class="button button--danger" onClick={() => setOpen(true)}>Reboot host</button>
    {open && <HostRebootDialog integration={{ hostname: 'example-host', scheduledReboot: { state: 'none' } } as HostIntegration} csrfToken="fixture" onClose={() => setOpen(false)} onChanged={async () => undefined} />}
  </main>;
}
render(<App />, document.getElementById('app')!);
