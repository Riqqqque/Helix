import { describe, expect, it } from 'vitest';
import { placeInfoTip } from './info-tip';

describe('info tip placement', () => {
  it('keeps the bubble inside the viewport near either edge', () => {
    expect(placeInfoTip({ left: 2, right: 17, top: 40, bottom: 55, width: 15 }, 70, 320, 240)).toMatchObject({ left: 12, side: 'below' });
    const right = placeInfoTip({ left: 303, right: 318, top: 40, bottom: 55, width: 15 }, 70, 320, 240);
    expect(right.left + right.width).toBeLessThanOrEqual(308);
    expect(right.arrowLeft).toBeLessThan(right.width);
  });

  it('moves above the trigger when the lower edge has no room', () => {
    const placement = placeInfoTip({ left: 150, right: 165, top: 205, bottom: 220, width: 15 }, 80, 360, 240);
    expect(placement.side).toBe('above');
    expect(placement.top).toBe(115);
  });

  it('clamps unusually tall bubbles instead of placing them off screen', () => {
    const placement = placeInfoTip({ left: 100, right: 115, top: 50, bottom: 65, width: 15 }, 300, 240, 180);
    expect(placement.top).toBe(12);
    expect(placement.width).toBe(216);
  });
});
