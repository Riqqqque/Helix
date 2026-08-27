import { useEffect, useState } from 'preact/hooks';
import { Icon } from './icons';
import type { ModpackSelection } from './modpack-api';
import type { ModpackPicker, ModpackPickerProps } from './modpack-picker';

type ModpackPickerComponent = typeof ModpackPicker;
let loadedPicker: ModpackPickerComponent | null = null;
let pendingPicker: Promise<ModpackPickerComponent> | null = null;

export function loadModpackPicker(): Promise<ModpackPickerComponent> {
  if (loadedPicker !== null) return Promise.resolve(loadedPicker);
  pendingPicker ??= import('./modpack-picker').then((module) => {
    loadedPicker = module.ModpackPicker;
    return loadedPicker;
  }).catch((error: unknown) => {
    pendingPicker = null;
    throw error;
  });
  return pendingPicker;
}

export function preloadModpackPicker(): void {
  void loadModpackPicker().catch(() => undefined);
}

export function ModpackRoute(props: ModpackPickerProps & { selection: ModpackSelection | null }) {
  const [Picker, setPicker] = useState<ModpackPickerComponent | null>(() => loadedPicker);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (Picker !== null || error !== null) return;
    let mounted = true;
    void loadModpackPicker().then((component) => {
      if (mounted) setPicker(() => component);
    }).catch(() => {
      if (mounted) setError('The Modrinth browser could not be loaded.');
    });
    return () => { mounted = false; };
  }, [Picker, error]);

  if (Picker !== null) return <Picker {...props} />;
  return <div class="modpack-state" role={error === null ? 'status' : 'alert'}><Icon name={error === null ? 'servers' : 'warning'} /><span>{error ?? 'Loading the Modrinth browser…'}</span>{error !== null && <button type="button" onClick={() => setError(null)}>Try again</button>}</div>;
}
