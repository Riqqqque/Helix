import { useEffect, useMemo, useRef } from 'preact/hooks';
import { GLOBE_LAND_PATHS, GLOBE_LAND_VIEWBOX } from './globe-land';
import {
  arcPath,
  controlPoint,
  projectLatLon,
  quadraticPoint,
  unwrapDestination,
  type GlobePoint,
} from './globe-geometry';
import type { GlobeLink, GlobeSnapshot } from './globe-api';
import './globe.css';

const GRATICULE_LONS = [-150, -90, -30, 30, 90, 150];
const GRATICULE_LATS = [-60, -30, 0, 30, 60];

interface PreparedLink {
  link: GlobeLink;
  from: GlobePoint;
  to: GlobePoint;
  control: GlobePoint;
  path: string;
}

function prefersReducedMotion(): boolean {
  return typeof window !== 'undefined'
    && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

function prepareLinks(snapshot: GlobeSnapshot | null): PreparedLink[] {
  if (snapshot === null || !snapshot.origin.available || snapshot.origin.lat === null || snapshot.origin.lon === null) {
    return [];
  }
  const from = projectLatLon(snapshot.origin.lat, snapshot.origin.lon);
  return snapshot.links.map((link) => {
    const raw = projectLatLon(link.lat, link.lon);
    const to = unwrapDestination(from, raw);
    const control = controlPoint(from, to);
    return { link, from, to, control, path: arcPath(from, to) };
  });
}

function destinationPins(snapshot: GlobeSnapshot | null): Array<{ link: GlobeLink; point: GlobePoint }> {
  if (snapshot === null) return [];
  return snapshot.links.map((link) => ({ link, point: projectLatLon(link.lat, link.lon) }));
}

export function GlobeMap({
  snapshot,
  flow,
  compact = false,
}: {
  snapshot: GlobeSnapshot | null;
  flow: boolean;
  compact?: boolean;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const prepared = useMemo(() => prepareLinks(snapshot), [snapshot]);
  const pins = useMemo(() => destinationPins(snapshot), [snapshot]);
  const origin = snapshot?.origin.available && snapshot.origin.lat !== null && snapshot.origin.lon !== null
    ? projectLatLon(snapshot.origin.lat, snapshot.origin.lon)
    : null;
  const animate = flow && !prefersReducedMotion();

  useEffect(() => {
    const canvas = canvasRef.current;
    const wrap = wrapRef.current;
    if (canvas === null || wrap === null || !animate || prepared.length === 0) return undefined;

    const particles = prepared.flatMap((item, linkIndex) => {
      const extra = item.link.activity > 0.6 ? 1 : 0;
      const count = (item.link.kind === 'player' ? 2 : 1) + extra;
      return Array.from({ length: count }, (_, index) => ({
        linkIndex,
        t: index / count,
        direction: index % 2 === 0 ? 1 : -1,
        speed: 0.14 + item.link.activity * 0.72,
      }));
    });

    const context = canvas.getContext('2d');
    if (context === null) return undefined;

    let frame = 0;
    let last = performance.now();
    let visible = true;
    let running = true;

    const paint = (now: number): void => {
      frame = 0;
      if (!running) return;
      const hidden = typeof document !== 'undefined' && document.visibilityState === 'hidden';
      if (!visible || hidden) {
        last = now;
        return;
      }
      const bounds = wrap.getBoundingClientRect();
      const ratio = Math.min(2, window.devicePixelRatio || 1);
      const width = Math.max(1, Math.round(bounds.width * ratio));
      const height = Math.max(1, Math.round(bounds.height * ratio));
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }
      const elapsed = Math.min(0.05, (now - last) / 1000);
      last = now;
      context.clearRect(0, 0, width, height);
      const styles = getComputedStyle(wrap);
      const accent = styles.getPropertyValue('--accent').trim() || '#d7f64d';
      const text = styles.getPropertyValue('--text').trim() || '#f1f0eb';
      for (const particle of particles) {
        const arc = prepared[particle.linkIndex];
        if (arc === undefined) continue;
        particle.t += particle.direction * particle.speed * elapsed;
        if (particle.t > 1) particle.t -= 1;
        if (particle.t < 0) particle.t += 1;
        const point = quadraticPoint(arc.from, arc.control, arc.to, particle.t);
        const x = (point.x / 1000) * width;
        const y = (point.y / 500) * height;
        const player = arc.link.kind === 'player';
        context.beginPath();
        context.fillStyle = player ? accent : text;
        context.globalAlpha = player ? 0.92 : 0.55;
        context.arc(x, y, (player ? 2.1 : 1.5) * ratio, 0, Math.PI * 2);
        context.fill();
      }
      context.globalAlpha = 1;
      frame = window.requestAnimationFrame(paint);
    };
    const resume = (): void => {
      if (!running || frame !== 0) return;
      last = performance.now();
      frame = window.requestAnimationFrame(paint);
    };
    const observer = new IntersectionObserver((entries) => {
      visible = entries.some((entry) => entry.isIntersecting);
      if (visible) resume();
    }, { threshold: 0.05 });
    observer.observe(wrap);
    const onVisibility = (): void => {
      if (document.visibilityState === 'visible') resume();
    };
    document.addEventListener('visibilitychange', onVisibility);
    resume();
    return () => {
      running = false;
      if (frame !== 0) window.cancelAnimationFrame(frame);
      observer.disconnect();
      document.removeEventListener('visibilitychange', onVisibility);
    };
  }, [animate, prepared]);

  return (
    <div ref={wrapRef} class={`globe-map${compact ? ' globe-map--compact' : ''}${animate ? ' is-flowing' : ''}`}>
      <svg class="globe-map__svg" viewBox={GLOBE_LAND_VIEWBOX} role="img" aria-label="World map of this host and country-level connections">
        <rect class="globe-map__ocean" width="1000" height="500" />
        {GRATICULE_LONS.map((lon) => {
          const x = projectLatLon(0, lon).x;
          return <line key={`lon-${lon}`} class="globe-map__grid" x1={x} y1="0" x2={x} y2="500" />;
        })}
        {GRATICULE_LATS.map((lat) => {
          const y = projectLatLon(lat, 0).y;
          return <line key={`lat-${lat}`} class="globe-map__grid" x1="0" y1={y} x2="1000" y2={y} />;
        })}
        <g class="globe-map__land">
          {GLOBE_LAND_PATHS.map((d, index) => <path key={index} d={d} />)}
        </g>
        {origin !== null && prepared.map((item) => (
          <path
            key={item.link.id}
            class={`globe-map__arc globe-map__arc--${item.link.kind}`}
            d={item.path}
            style={{ opacity: 0.28 + item.link.activity * 0.45 }}
          />
        ))}
        {pins.map(({ link, point }) => (
          <g key={`pin-${link.id}`} class={`globe-map__pin globe-map__pin--${link.kind}`} transform={`translate(${point.x} ${point.y})`}>
            <circle r={link.kind === 'player' ? 4.2 : 3.2} />
            <circle class="globe-map__pin-core" r={link.kind === 'player' ? 1.7 : 1.3} />
          </g>
        ))}
        {origin !== null && (
          <g class="globe-map__origin" transform={`translate(${origin.x} ${origin.y})`}>
            <circle class="globe-map__origin-glow" r="14" />
            <circle class="globe-map__origin-ring" r="7.5" />
            <circle class="globe-map__origin-core" r="3.4" />
          </g>
        )}
      </svg>
      {animate && <canvas ref={canvasRef} class="globe-map__particles" aria-hidden="true" />}
    </div>
  );
}
