import { render } from 'preact';
import { visualGameInstances, visualGameReadiness } from './game-fixtures';
import { GameWorkspaceView } from './games';
import './styles.css';
import './game-preview.css';

const root = document.querySelector<HTMLDivElement>('#game-preview-root');
if (root === null) throw new Error('Game preview root is missing.');

const parameters = new URLSearchParams(window.location.search);
const locked = parameters.get('state') === 'locked';
const narrow = parameters.get('viewport') === 'narrow';
const theme = parameters.get('theme');
if (theme === 'light' || theme === 'midnight' || theme === 'oled') {
  document.documentElement.dataset.theme = theme;
}

render(
  <main class={`game-preview-shell${narrow ? ' game-preview-shell--narrow' : ''}`}>
    <div class="game-preview-label">Visual test fixture · not production data</div>
    <GameWorkspaceView
      readiness={locked ? { ...visualGameReadiness, availability: 'unavailable' } : visualGameReadiness}
      readinessPhase="ready"
      instances={locked ? null : visualGameInstances}
      totalInstances={locked ? 0 : visualGameInstances.length}
    />
  </main>,
  root,
);
