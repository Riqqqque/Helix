import { render } from 'preact';
import { useState } from 'preact/hooks';
import { ValheimPanel } from '../src/valheim-panel';
import { CreateValheimDialog } from '../src/servers';
import type { NativeServerDetail } from '../src/control-api';
import '../src/styles.css';

const detail: NativeServerDetail = {
  id: 'helix:11111111-1111-4111-8111-111111111111', name: 'Valheim test world', instanceName: 'valheim-test', kind: 'valheim',
  software: 'Valheim', minecraftVersion: 'dedicated', build: '896660', javaVersion: 0, runtimeImage: 'helix-valheim-runtime:3',
  artifactSha256: '0'.repeat(64), memoryLimitMb: 4096, cpuLimitMillis: 0, gamePort: 2456, queryPort: 2457,
  startOnBoot: true, createdAtUnixMs: 0, dataPath: '/test/valheim', diskBytes: 0, status: 'stopped', playersOnline: 0,
  maxPlayers: 10, cpuPercent: 0, memoryUsedMb: 0, tps: null, containerState: {}, settings: null, consoleHistory: {
    persistent: true, retentionBytes: 1024, retentionFiles: 5, scope: 'per_server',
  }, capabilities: ['files', 'settings', 'logs', 'backups', 'advanced'], browserListing: null, modpack: null,
};
function App() {
  const [mode, setMode] = useState<'settings' | 'mods'>('settings');
  const [running, setRunning] = useState(false);
  const [creating, setCreating] = useState(false);
  return <main style={{ maxWidth: '1180px', margin: '0 auto', padding: '24px' }}>
    <header class="page-head"><h1>Valheim test world</h1><p>Isolated browser test · no production server actions</p></header>
    <button class="button button--quiet" onClick={() => setCreating(true)}>New Valheim server</button>
    {creating && <CreateValheimDialog csrfToken="disposable-browser-test" servers={[]} canManageNetwork logicalCores={8} onClose={() => setCreating(false)} onComplete={async () => {}} onSessionExpired={() => { throw new Error('Unexpected session expiry'); }} />}
    <nav class="server-tabs"><button onClick={() => setMode('settings')}>Settings</button><button onClick={() => setMode('mods')}>Mods</button><label><input type="checkbox" checked={running} onChange={e => setRunning(e.currentTarget.checked)} /> Simulate running</label></nav>
    <ValheimPanel key={mode} detail={{ ...detail, status: running ? 'online' : 'stopped' }} mode={mode} csrfToken="disposable-browser-test" canManage onSessionExpired={() => { throw new Error('Session unexpectedly expired'); }} onBackups={() => { document.title = 'Backups opened'; }} />
  </main>;
}
render(<App />, document.getElementById('app')!);
