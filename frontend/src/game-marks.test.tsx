import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { GameMark, gameMarkForSoftware } from './game-marks';

describe('game marks', () => {
  it('maps Minecraft and V Rising software to distinct marks', () => {
    expect(gameMarkForSoftware('Paper', 'minecraft')).toBe('minecraft');
    expect(gameMarkForSoftware('Leaves')).toBe('minecraft');
    expect(gameMarkForSoftware('V Rising', 'vrising')).toBe('vrising');
    expect(render(<GameMark game="minecraft" />)).toContain('game-mark--minecraft');
    expect(render(<GameMark game="vrising" />)).toContain('game-mark--vrising');
    expect(render(<GameMark game="minecraft" />)).not.toBe(render(<GameMark game="vrising" />));
  });
});
