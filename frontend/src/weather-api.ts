import {
  ApiError,
  expectArray,
  expectNumber,
  expectRecord,
  expectString,
  requestJson,
} from './api';

export type TemperatureUnit = 'celsius' | 'fahrenheit';

export interface WeatherForecast {
  provider: 'Open-Meteo';
  location: {
    name: string;
    adminArea: string | null;
    country: string;
    timezone: string;
  };
  current: {
    observedAt: string;
    temperature: number;
    apparentTemperature: number;
    relativeHumidityPercent: number;
    precipitation: number;
    weatherCode: number;
    windSpeed: number;
    temperatureUnit: string;
    precipitationUnit: string;
    windSpeedUnit: string;
  };
  daily: Array<{
    date: string;
    weatherCode: number;
    temperatureMax: number;
    temperatureMin: number;
    precipitationProbabilityMax: number | null;
  }>;
  fetchedAtUnixMs: number;
}

function boundedText(value: unknown, context: string, maximum: number): string {
  if (typeof value !== 'string' || value.trim().length === 0 || value.length > maximum) {
    throw new ApiError(`${context} returned invalid text.`);
  }
  return value;
}

export function parseWeatherForecast(value: unknown): WeatherForecast {
  const context = 'Weather API';
  const record = expectRecord(value, context);
  const provider = expectString(record, 'provider', context);
  if (provider !== 'Open-Meteo') throw new ApiError(`${context} returned an unknown provider.`);
  const location = expectRecord(record.location, `${context} location`);
  const current = expectRecord(record.current, `${context} current conditions`);
  const rawDays = expectArray(record, 'daily', context, 5);
  if (rawDays.length === 0) throw new ApiError(`${context} returned no forecast days.`);

  const adminArea = location.adminArea === null
    ? null
    : boundedText(location.adminArea, `${context} location`, 160);
  const days = rawDays.map((value, index) => {
    const dayContext = `${context} day ${index + 1}`;
    const day = expectRecord(value, dayContext);
    const date = expectString(day, 'date', dayContext);
    if (!/^\d{4}-\d{2}-\d{2}$/u.test(date)) throw new ApiError(`${dayContext} returned an invalid date.`);
    const probability = day.precipitationProbabilityMax === null
      ? null
      : expectNumber(day, 'precipitationProbabilityMax', dayContext, { minimum: 0, maximum: 100 });
    return {
      date,
      weatherCode: expectNumber(day, 'weatherCode', dayContext, { integer: true, minimum: 0, maximum: 99 }),
      temperatureMax: expectNumber(day, 'temperatureMax', dayContext, { minimum: -200, maximum: 200 }),
      temperatureMin: expectNumber(day, 'temperatureMin', dayContext, { minimum: -200, maximum: 200 }),
      precipitationProbabilityMax: probability,
    };
  });

  return {
    provider,
    location: {
      name: boundedText(location.name, `${context} location`, 160),
      adminArea,
      country: boundedText(location.country, `${context} location`, 160),
      timezone: boundedText(location.timezone, `${context} location`, 100),
    },
    current: {
      observedAt: boundedText(current.observedAt, `${context} current conditions`, 64),
      temperature: expectNumber(current, 'temperature', context, { minimum: -200, maximum: 200 }),
      apparentTemperature: expectNumber(current, 'apparentTemperature', context, { minimum: -200, maximum: 200 }),
      relativeHumidityPercent: expectNumber(current, 'relativeHumidityPercent', context, { minimum: 0, maximum: 100 }),
      precipitation: expectNumber(current, 'precipitation', context, { minimum: 0, maximum: 1_000 }),
      weatherCode: expectNumber(current, 'weatherCode', context, { integer: true, minimum: 0, maximum: 99 }),
      windSpeed: expectNumber(current, 'windSpeed', context, { minimum: 0, maximum: 1_000 }),
      temperatureUnit: boundedText(current.temperatureUnit, `${context} units`, 16),
      precipitationUnit: boundedText(current.precipitationUnit, `${context} units`, 16),
      windSpeedUnit: boundedText(current.windSpeedUnit, `${context} units`, 16),
    },
    daily: days,
    fetchedAtUnixMs: expectNumber(record, 'fetchedAtUnixMs', context, { integer: true, minimum: 0 }),
  };
}

export function getWeatherForecast(
  location: string,
  unit: TemperatureUnit,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<WeatherForecast> {
  const query = new URLSearchParams({ location, unit });
  return requestJson(`/api/v1/weather?${query.toString()}`, parseWeatherForecast, {
    csrfToken,
    signal,
    timeoutMs: 12_000,
  });
}
