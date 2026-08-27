import { afterEach, describe, expect, it, vi } from 'vitest';
import { getWeatherForecast, parseWeatherForecast } from './weather-api';

const response = {
  provider: 'Open-Meteo',
  location: { name: 'Denver', adminArea: 'Colorado', country: 'United States', timezone: 'America/Denver' },
  current: { observedAt: '2026-08-26T12:00', temperature: 75, apparentTemperature: 74, relativeHumidityPercent: 32, precipitation: 0, weatherCode: 0, windSpeed: 4, temperatureUnit: '°F', precipitationUnit: 'inch', windSpeedUnit: 'mp/h' },
  daily: [{ date: '2026-08-26', weatherCode: 0, temperatureMax: 80, temperatureMin: 55, precipitationProbabilityMax: 10 }],
  fetchedAtUnixMs: 1_787_765_000_000,
} as const;

afterEach(() => vi.unstubAllGlobals());

describe('weather API', () => {
  it('parses a bounded normalized forecast', () => {
    expect(parseWeatherForecast(response)).toEqual(response);
    expect(() => parseWeatherForecast({ ...response, daily: [] })).toThrow();
    expect(() => parseWeatherForecast({ ...response, provider: 'other' })).toThrow();
  });

  it('encodes location and keeps the request same-origin', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(response), { status: 200, headers: { 'Content-Type': 'application/json' } }));
    vi.stubGlobal('fetch', fetchMock);
    await getWeatherForecast('Denver, CO', 'fahrenheit', 'EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE');
    expect(fetchMock.mock.calls[0]?.[0]).toBe('/api/v1/weather?location=Denver%2C+CO&unit=fahrenheit');
  });
});
