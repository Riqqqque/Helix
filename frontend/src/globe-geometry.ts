export const GLOBE_WIDTH = 1000;
export const GLOBE_HEIGHT = 500;

export interface GlobePoint {
  x: number;
  y: number;
}

export function projectLatLon(lat: number, lon: number): GlobePoint {
  return {
    x: ((lon + 180) / 360) * GLOBE_WIDTH,
    y: ((90 - lat) / 180) * GLOBE_HEIGHT,
  };
}

export function unwrapDestination(origin: GlobePoint, destination: GlobePoint): GlobePoint {
  let { x } = destination;
  const half = GLOBE_WIDTH / 2;
  if (x - origin.x > half) x -= GLOBE_WIDTH;
  if (origin.x - x > half) x += GLOBE_WIDTH;
  return { x, y: destination.y };
}

export function controlPoint(from: GlobePoint, to: GlobePoint): GlobePoint {
  const mx = (from.x + to.x) / 2;
  const my = (from.y + to.y) / 2;
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const dist = Math.hypot(dx, dy) || 1;
  const bulge = Math.min(88, 16 + dist * 0.11);
  return {
    x: mx - (dy / dist) * bulge,
    y: my + (dx / dist) * bulge,
  };
}

export function quadraticPoint(from: GlobePoint, control: GlobePoint, to: GlobePoint, t: number): GlobePoint {
  const u = 1 - t;
  return {
    x: u * u * from.x + 2 * u * t * control.x + t * t * to.x,
    y: u * u * from.y + 2 * u * t * control.y + t * t * to.y,
  };
}

export function arcPath(from: GlobePoint, to: GlobePoint): string {
  const control = controlPoint(from, to);
  return `M${from.x.toFixed(1)} ${from.y.toFixed(1)} Q${control.x.toFixed(1)} ${control.y.toFixed(1)} ${to.x.toFixed(1)} ${to.y.toFixed(1)}`;
}
