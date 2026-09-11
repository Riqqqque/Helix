import type { NativeServerDetail } from './control-api';
import { Icon } from './icons';
import './server-config-notice.css';

export function ServerConfigNotice({ detail }: { detail: NativeServerDetail }) {
  const changes = detail.configChanges;
  if (detail.status !== 'online' || changes?.state !== 'changed') return null;
  const propertiesChanged = changes.files.includes('server.properties');
  const otherFilesChanged = changes.files.some(path => path !== 'server.properties');
  return (
    <section class="server-config-status" aria-label="Saved configuration">
      <Icon name="check" size={18} />
      <div>
        <p role="status">
          <strong>{propertiesChanged ? 'Settings saved.' : 'Configuration files changed.'}</strong>{' '}
          {propertiesChanged
            ? 'Restart when you’re ready to apply them. Your server is still running.'
            : 'Some mods reload automatically; others need a restart.'}
        </p>
        <details>
          <summary>View changed files</summary>
          <ul>{changes.files.map(path => <li key={path}><code>{path}</code></li>)}</ul>
          {otherFilesChanged && <p>These files changed since this run was first checked. A file change alone does not confirm that a mod loaded it.</p>}
          {changes.limited && <p>This is a bounded check, not a full scan. Other configuration files may also have changed.</p>}
        </details>
      </div>
    </section>
  );
}
