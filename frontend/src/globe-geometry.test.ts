import { describe, expect, it } from 'vitest';
import {
  GLOBE_HEIGHT,
  GLOBE_WIDTH,
  controlPoint,
  coverProject,
  projectLatLon,
  quadraticPoint,
  unwrapDestination,
} from './globe-geometry';

describe('globe projection', () => {
  it('places the equator and prime meridian at the map center', () => {
    const origin = projectLatLon(0, 0);
    expect(origin.x).toBe(GLOBE_WIDTH / 2);
    expect(origin.y).toBe(GLOBE_HEIGHT / 2);
  });

  it('keeps the Netherlands east of a US centroid and north of the equator', () => {
    const unitedStates = projectLatLon(38.82, -96.33);
    const netherlands = projectLatLon(52.13, 5.55);
    expect(netherlands.x).toBeGreaterThan(unitedStates.x);
    expect(netherlands.y).toBeLessThan(GLOBE_HEIGHT / 2);
  });

  it('unwraps a Pacific hop so the short path does not cross the map', () => {
    const california = projectLatLon(37, -122);
    const japan = projectLatLon(36, 138);
    const unwrapped = unwrapDestination(california, japan);
    expect(unwrapped.x).toBeLessThan(california.x);
    expect(Math.abs(unwrapped.x - california.x)).toBeLessThan(Math.abs(japan.x - california.x));
  });

  it('covers a wide widget by cropping poles instead of leaving side gutters', () => {
    const center = { x: GLOBE_WIDTH / 2, y: GLOBE_HEIGHT / 2 };
    const mapped = coverProject(center, 2000, 500);
    expect(mapped.x).toBe(1000);
    expect(mapped.y).toBe(250);
    const north = coverProject({ x: GLOBE_WIDTH / 2, y: 0 }, 2000, 500);
    expect(north.y).toBeLessThan(0);
  });

  it('covers a tall widget by cropping the date line instead of letterboxing', () => {
    const center = coverProject({ x: GLOBE_WIDTH / 2, y: GLOBE_HEIGHT / 2 }, 1000, 1000);
    expect(center.x).toBe(500);
    expect(center.y).toBe(500);
    const west = coverProject({ x: 0, y: GLOBE_HEIGHT / 2 }, 1000, 1000);
    expect(west.x).toBeLessThan(0);
  });

  it('keeps quadratic samples on the curve between two points', () => {
    const from = { x: 100, y: 100 };
    const to = { x: 200, y: 120 };
    const control = controlPoint(from, to);
    const mid = quadraticPoint(from, control, to, 0.5);
    expect(mid.x).toBeGreaterThan(100);
    expect(mid.x).toBeLessThan(200);
    expect(quadraticPoint(from, control, to, 0)).toEqual(from);
    expect(quadraticPoint(from, control, to, 1)).toEqual(to);
  });
});
