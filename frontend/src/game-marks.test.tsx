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
    expect(gameMarkForSoftware('Valheim', 'valheim')).toBe('valheim');
    expect(gameMarkForSoftware('tModLoader', 'terraria')).toBe('terraria');
    expect(render(<GameMark game="valheim" />)).toContain('game-mark--valheim');
    expect(render(<GameMark game="terraria" />)).toContain('game-mark--terraria');
    expect(gameMarkForSoftware('Palworld', 'palworld')).toBe('palworld');
    expect(gameMarkForSoftware('Palworld Dedicated Server')).toBe('palworld');
    expect(render(<GameMark game="palworld" />)).toContain('game-mark--palworld');
  });

  it('maps the managed dedicated games to their marks', () => {
    expect(gameMarkForSoftware('Satisfactory Dedicated', 'satisfactory')).toBe('satisfactory');
    expect(gameMarkForSoftware('Project Zomboid Dedicated')).toBe('project_zomboid');
    expect(gameMarkForSoftware('7 Days to Die', 'seven_days_to_die')).toBe('seven_days_to_die');
    expect(gameMarkForSoftware('Rust Dedicated Server')).toBe('rust');
    expect(gameMarkForSoftware('Sons of the Forest', 'sons_of_the_forest')).toBe('sons_of_the_forest');
    expect(gameMarkForSoftware('Factorio headless')).toBe('factorio');
    expect(gameMarkForSoftware("Don't Starve Together", 'dont_starve_together')).toBe('dont_starve_together');
    expect(gameMarkForSoftware('Vintage Story Server')).toBe('vintage_story');
    for (const game of [
      'satisfactory',
      'project_zomboid',
      'seven_days_to_die',
      'rust',
      'sons_of_the_forest',
      'factorio',
      'dont_starve_together',
      'vintage_story',
    ] as const) {
      expect(render(<GameMark game={game} />)).toContain(`game-mark--${game}`);
    }
  });
});
