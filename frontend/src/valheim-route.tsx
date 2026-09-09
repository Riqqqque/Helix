import { useEffect, useState } from 'preact/hooks';
import type { ValheimPanel, ValheimSettingsFields } from './valheim-panel';

type Module = { ValheimPanel: typeof ValheimPanel; ValheimSettingsFields: typeof ValheimSettingsFields };
let loaded: Module | null = null;
let pending: Promise<Module> | null = null;
function load(): Promise<Module> {
  pending ??= import('./valheim-panel').then(value => { loaded = value; return value; }).catch(error => { pending = null; throw error; });
  return pending;
}
function useValheimModule() {
  const [module, setModule] = useState(() => loaded);
  const [error, setError] = useState(false);
  useEffect(() => {
    if (module || error) return;
    let active = true;
    void load().then(value => { if (active) setModule(value); }).catch(() => { if (active) setError(true); });
    return () => { active = false; };
  }, [module, error]);
  return { module, fallback: <p role={error ? 'alert' : 'status'}>{error ? 'Valheim tools could not load. Your server was not changed.' : 'Loading Valheim tools…'}{error && <button class="button button--quiet" onClick={() => setError(false)}>Try again</button>}</p> };
}
export function ValheimPanelRoute(props: Parameters<typeof ValheimPanel>[0]) {
  const { module, fallback } = useValheimModule();
  return module ? <module.ValheimPanel {...props} /> : fallback;
}
export function ValheimFieldsRoute(props: Parameters<typeof ValheimSettingsFields>[0]) {
  const { module, fallback } = useValheimModule();
  return module ? <module.ValheimSettingsFields {...props} /> : fallback;
}
