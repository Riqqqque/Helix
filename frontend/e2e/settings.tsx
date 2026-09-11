import { render } from 'preact';
import { useState } from 'preact/hooks';
import { SettingsPanel } from '../src/servers';
import { ServerConfigNotice } from '../src/server-config-notice';
import { getServerSettings, type MinecraftSettings, type NativeServerDetail } from '../src/control-api';
import '../src/styles.css';

// Isolated fixture: no production requests, files or server actions.
const storageKey = 'helix-settings-browser-fixture';
const initial = {
  expected_revision: 'a'.repeat(64), motd: 'Example world', game_mode: 'survival', difficulty: 'normal',
  max_players: 5, view_distance: 8, simulation_distance: 4, player_idle_timeout: 0,
  online_mode: true, pvp: true, allow_flight: false, white_list: false, enforce_white_list: false,
  spawn_protection: 0, game_port: 25566, memory_mb: 4096,
  restart_behavior: { activation: 'server_restart', restart_required_fields: ['motd', 'game_mode', 'difficulty', 'max_players', 'view_distance', 'simulation_distance', 'player_idle_timeout', 'online_mode', 'allow_flight', 'white_list', 'enforce_white_list', 'spawn_protection'], message: 'Saved settings take effect after a restart.' },
};
let stored = JSON.parse(localStorage.getItem(storageKey) ?? JSON.stringify({ settings: initial, changed: false, restarts: 0 }));
let failSave = false;
const saveFixture = () => localStorage.setItem(storageKey, JSON.stringify(stored));
const id = 'helix:settings-browser-fixture';
window.fetch = async (input, init) => {
  if (typeof input !== 'string' || !input.endsWith('/servers/helix%3Asettings-browser-fixture/settings')) throw new Error('Unexpected fixture request');
  if (init?.method !== 'POST') return Response.json(stored.settings);
  const request = JSON.parse(String(init.body));
  if (failSave || request.expected_revision !== stored.settings.expected_revision) {
    return Response.json({ code: 'conflict', message: 'Settings changed elsewhere; reload before saving.' }, { status: 409 });
  }
  const fields = Object.keys(request).filter(key => key !== 'expected_revision' && request[key] !== stored.settings[key]);
  stored = { ...stored, changed: fields.length > 0 || stored.changed, settings: { ...stored.settings, ...request, expected_revision: String(Number.parseInt(stored.settings.expected_revision.slice(-4), 16) + 1).padStart(64, '0') } };
  saveFixture();
  return Response.json({ settings: stored.settings, changed: fields.length > 0, changed_fields: fields, restart_required: fields.length > 0, container_republished: false });
};

function App({ initialSettings }: { initialSettings: MinecraftSettings }) {
  const [settings, setSettings] = useState(initialSettings);
  const [restartRevision, setRestartRevision] = useState(0);
  const [changed, setChanged] = useState(stored.changed);
  const [fail, setFail] = useState(false);
  const detail = { id, name: 'Example server', kind: 'minecraft', software: 'Paper', minecraftVersion: '26.2', settings, status: 'online', configChanges: { state: changed ? 'changed' : 'no_changes', files: changed ? ['server.properties'] : [], limited: true } } as NativeServerDetail & { settings: MinecraftSettings };
  const restart = () => {
    stored = { ...stored, changed: false, restarts: stored.restarts + 1 }; saveFixture();
    setChanged(false); setRestartRevision(value => value + 1);
  };
  return <main style={{ maxWidth: '1200px', margin: '0 auto', padding: '20px' }}>
    <header class="server-detail-head"><h1>Example server</h1><button class="button button--quiet" onClick={restart}>Restart</button></header>
    <ServerConfigNotice detail={detail} />
    <nav class="server-tabs"><button class="is-active">Settings</button></nav>
    <SettingsPanel detail={detail} csrfToken="disposable-browser-test" servers={[]} canManageServers canManageNetwork={false} restartSuccessRevision={restartRevision} onRestart={restart}
      onSaved={async () => { setSettings(await getServerSettings(id, 'disposable-browser-test')); setChanged(stored.changed); }} onSessionExpired={() => { throw new Error('Unexpected session expiry'); }} />
    <aside style={{ padding: '20px', fontSize: '13px' }}>
      <p>Isolated test. No production server actions.</p>
      <output>Restarts: {stored.restarts}</output>
      <label><input type="checkbox" checked={fail} onChange={e => { failSave = e.currentTarget.checked; setFail(failSave); }} /> Simulate save conflict</label>
    </aside>
  </main>;
}
getServerSettings(id, 'disposable-browser-test').then(settings => render(<App initialSettings={settings} />, document.getElementById('app')!));
