import { render } from 'preact';
import { App } from './shell';
import { initializeTheme } from './theme';
import './styles.css';

initializeTheme();

const root = document.querySelector('#app');
if (root === null) {
  throw new Error('Helix could not find its application root.');
}

render(<App />, root);
