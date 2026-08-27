import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  ApiError,
  getHealth,
  login,
  logout,
  parseAuthSession,
  parseHealthResponse,
  parseSetupStatus,
  parseSystemOverview,
  updateAccount,
} from './api';
import { parseGameHostingReadiness } from './game-api';

afterEach(() => {
  vi.unstubAllGlobals();
});

const validOverview = {
  hostname: 'helix-node',
  operating_system: 'Ubuntu 26.04 LTS',
  architecture: 'x86_64',
  kernel_version: '6.17.0',
  uptime_seconds: 90_061,
  cpu: {
    usage_percent: 12.5,
    logical_cores: 8,
  },
  memory: {
    total_bytes: 17_179_869_184,
    used_bytes: 4_294_967_296,
    available_bytes: 12_884_901_888,
  },
  swap: {
    total_bytes: 2_147_483_648,
    used_bytes: 0,
  },
  storage: {
    availability: 'available',
    mounts: [
      {
        name: 'nvme0n1p2',
        file_system: 'ext4',
        mount_point: '/',
        total_bytes: '1099511627776',
        available_bytes: '824633720832',
        used_bytes: '274877906944',
        read_only: false,
        removable: false,
      },
    ],
    omitted_mounts: 0,
    omitted_text_fields: 0,
  },
  network: {
    availability: 'available',
    interfaces: [
      {
        name: 'enp1s0',
        addresses: [
          { address: '192.0.2.10', prefix_length: 24 },
          { address: '2001:db8::10', prefix_length: 64 },
        ],
        total_received_bytes: '8589934592',
        total_transmitted_bytes: '2147483648',
        mtu_bytes: '1500',
      },
    ],
    omitted_interfaces: 0,
    omitted_addresses: 0,
  },
  collected_at_unix_ms: 1_788_000_000_000,
};

const validGameReadiness = {
  schema_version: 1,
  availability: 'unavailable',
  available_features: [],
  blockers: [
    { code: 'verified_restore', status: 'required' },
    { code: 'privileged_broker', status: 'required' },
    { code: 'native_execution', status: 'required' },
  ],
  collected_at_unix_ms: 1_788_000_000_000,
};

describe('parseHealthResponse', () => {
  it('maps a valid health response', () => {
    expect(
      parseHealthResponse({
        status: 'ok',
        version: '0.1.0',
        state_database: 'ok',
        metrics_database: 'ok',
        timestamp_unix_ms: 1_788_000_000_000,
      }),
    ).toEqual({
      status: 'ok',
      version: '0.1.0',
      stateDatabase: 'ok',
      metricsDatabase: 'ok',
      timestampUnixMs: 1_788_000_000_000,
    });
  });

  it('keeps open-ended warning states intact', () => {
    const response = parseHealthResponse({
      status: 'degraded',
      version: '0.1.0',
      state_database: 'ok',
      metrics_database: 'recovered',
      timestamp_unix_ms: 1_788_000_000_000,
    });

    expect(response.status).toBe('degraded');
    expect(response.metricsDatabase).toBe('recovered');
  });

  it('rejects incomplete health data instead of inventing defaults', () => {
    expect(() =>
      parseHealthResponse({ status: 'ok', version: '0.1.0' }),
    ).toThrowError(ApiError);
  });
});

describe('parseGameHostingReadiness', () => {
  it('accepts the bounded unavailable contract without inventing instances', () => {
    expect(parseGameHostingReadiness(validGameReadiness)).toEqual({
      schemaVersion: 1,
      availability: 'unavailable',
      availableFeatures: [],
      blockers: [
        { code: 'verified_restore', status: 'required' },
        { code: 'privileged_broker', status: 'required' },
        { code: 'native_execution', status: 'required' },
      ],
      collectedAtUnixMs: 1_788_000_000_000,
    });
  });

  it('rejects duplicate or malformed capability metadata', () => {
    expect(() =>
      parseGameHostingReadiness({
        ...validGameReadiness,
        available_features: ['instances.view', 'instances.view'],
      }),
    ).toThrowError(ApiError);
    expect(() =>
      parseGameHostingReadiness({
        ...validGameReadiness,
        blockers: [{ code: '../native', status: 'required' }],
      }),
    ).toThrowError(ApiError);
  });

  it('rejects a ready state with unresolved blockers', () => {
    expect(() =>
      parseGameHostingReadiness({
        ...validGameReadiness,
        availability: 'ready',
      }),
    ).toThrowError(ApiError);
  });
});

describe('parseSystemOverview', () => {
  it('maps a complete system overview', () => {
    const overview = parseSystemOverview(validOverview);

    expect(overview.hostname).toBe('helix-node');
    expect(overview.cpu).toEqual({ usagePercent: 12.5, logicalCores: 8 });
    expect(overview.memory.usedBytes).toBe(4_294_967_296);
    expect(overview.storage.mounts[0]).toMatchObject({
      name: 'nvme0n1p2',
      fileSystem: 'ext4',
      mountPoint: '/',
      usedBytes: 274_877_906_944n,
    });
    expect(overview.network.interfaces[0]).toMatchObject({
      name: 'enp1s0',
      totalReceivedBytes: 8_589_934_592n,
    });
    expect(overview.collectedAtUnixMs).toBe(1_788_000_000_000);
  });

  it('accepts null CPU usage for the first valid sample', () => {
    const overview = parseSystemOverview({
      ...validOverview,
      cpu: { ...validOverview.cpu, usage_percent: null },
    });

    expect(overview.cpu.usagePercent).toBeNull();
  });

  it('accepts unavailable host metadata only when represented as null', () => {
    const overview = parseSystemOverview({
      ...validOverview,
      hostname: null,
      operating_system: null,
      kernel_version: null,
    });

    expect(overview.hostname).toBeNull();
    expect(overview.operatingSystem).toBeNull();
    expect(overview.kernelVersion).toBeNull();
  });

  it('preserves null storage text instead of inventing lossy host labels', () => {
    const overview = parseSystemOverview({
      ...validOverview,
      storage: {
        availability: 'degraded',
        mounts: [
          {
            ...validOverview.storage.mounts[0],
            name: null,
            file_system: null,
            mount_point: null,
          },
        ],
        omitted_mounts: 0,
        omitted_text_fields: 3,
      },
    });

    expect(overview.storage.availability).toBe('degraded');
    expect(overview.storage.mounts[0]).toMatchObject({
      name: null,
      fileSystem: null,
      mountPoint: null,
    });
    expect(overview.storage.omittedTextFields).toBe(3);
  });

  it('keeps an unlabeled Windows volume usable through its mount point', () => {
    const overview = parseSystemOverview({
      ...validOverview,
      storage: {
        availability: 'degraded',
        mounts: [
          {
            ...validOverview.storage.mounts[0],
            name: null,
            file_system: 'NTFS',
            mount_point: 'C:\\',
          },
        ],
        omitted_mounts: 0,
        omitted_text_fields: 1,
      },
    });

    expect(overview.storage).toMatchObject({
      availability: 'degraded',
      omittedTextFields: 1,
      mounts: [
        {
          name: null,
          fileSystem: 'NTFS',
          mountPoint: 'C:\\',
        },
      ],
    });
  });

  it('accepts explicit unavailable collectors without fabricating entries', () => {
    const overview = parseSystemOverview({
      ...validOverview,
      storage: {
        availability: 'unavailable',
        mounts: [],
        omitted_mounts: 0,
        omitted_text_fields: 0,
      },
      network: {
        availability: 'unavailable',
        interfaces: [],
        omitted_interfaces: 0,
        omitted_addresses: 0,
      },
    });

    expect(overview.storage).toMatchObject({
      availability: 'unavailable',
      mounts: [],
    });
    expect(overview.network).toMatchObject({
      availability: 'unavailable',
      interfaces: [],
    });
  });

  it('keeps bounded omission counts visible for degraded discovery', () => {
    const overview = parseSystemOverview({
      ...validOverview,
      storage: {
        ...validOverview.storage,
        availability: 'degraded',
        omitted_mounts: 7,
      },
      network: {
        ...validOverview.network,
        availability: 'degraded',
        omitted_interfaces: 2,
        omitted_addresses: 9,
      },
    });

    expect(overview.storage.omittedMounts).toBe(7);
    expect(overview.network.omittedInterfaces).toBe(2);
    expect(overview.network.omittedAddresses).toBe(9);
  });

  it('accepts every documented collection boundary and u64 telemetry values', () => {
    const mounts = Array.from({ length: 64 }, (_, index) => ({
      name: `disk-${index}`,
      file_system: 'ext4',
      mount_point: `/mnt/${index}`,
      total_bytes: index === 0 ? '18446744073709551615' : '4096',
      available_bytes: index === 0 ? '18446744073709551615' : '3072',
      used_bytes: index === 0 ? '0' : '1024',
      read_only: false,
      removable: false,
    }));
    const interfaces = Array.from({ length: 64 }, (_, interfaceIndex) => ({
      name: `eth${interfaceIndex}`,
      addresses: Array.from(
        { length: interfaceIndex === 0 ? 16 : interfaceIndex <= 60 ? 4 : 0 },
        (_, addressIndex) => ({
          address: `2001:db8:${interfaceIndex.toString(16)}::${addressIndex + 1}`,
          prefix_length: 64,
        }),
      ),
      total_received_bytes:
        interfaceIndex === 0 ? '18446744073709551615' : '0',
      total_transmitted_bytes:
        interfaceIndex === 0 ? '18446744073709551615' : '0',
      mtu_bytes: interfaceIndex === 0 ? '18446744073709551615' : '1500',
    }));

    const overview = parseSystemOverview({
      ...validOverview,
      storage: {
        availability: 'available',
        mounts,
        omitted_mounts: 0,
        omitted_text_fields: 0,
      },
      network: {
        availability: 'available',
        interfaces,
        omitted_interfaces: 0,
        omitted_addresses: 0,
      },
    });

    expect(overview.storage.mounts).toHaveLength(64);
    expect(overview.storage.mounts[0]?.totalBytes).toBe(
      18_446_744_073_709_551_615n,
    );
    expect(overview.network.interfaces).toHaveLength(64);
    expect(
      overview.network.interfaces.reduce(
        (count, networkInterface) => count + networkInterface.addresses.length,
        0,
      ),
    ).toBe(256);
    expect(overview.network.interfaces[0]?.addresses).toHaveLength(16);
    expect(overview.network.interfaces[0]?.totalReceivedBytes).toBe(
      18_446_744_073_709_551_615n,
    );
  });

  it('rejects impossible CPU percentages', () => {
    expect(() =>
      parseSystemOverview({
        ...validOverview,
        cpu: { ...validOverview.cpu, usage_percent: 101 },
      }),
    ).toThrowError(ApiError);
  });

  it('rejects invalid discovery bounds, invariants, and telemetry shapes', () => {
    const mount = validOverview.storage.mounts[0];
    const networkInterface = validOverview.network.interfaces[0];
    const invalidPayloads = [
      {
        ...validOverview,
        memory: { ...validOverview.memory, used_bytes: validOverview.memory.total_bytes + 1 },
      },
      {
        ...validOverview,
        memory: { ...validOverview.memory, available_bytes: 0.5 },
      },
      {
        ...validOverview,
        swap: { ...validOverview.swap, used_bytes: validOverview.swap.total_bytes + 1 },
      },
      {
        ...validOverview,
        hostname: 'host\nname',
      },
      {
        ...validOverview,
        storage: {
          ...validOverview.storage,
          mounts: Array.from({ length: 65 }, () => mount),
        },
      },
      {
        ...validOverview,
        storage: { ...validOverview.storage, omitted_text_fields: 1 },
      },
      {
        ...validOverview,
        storage: {
          availability: 'unavailable',
          mounts: [mount],
          omitted_mounts: 0,
          omitted_text_fields: 0,
        },
      },
      {
        ...validOverview,
        storage: {
          ...validOverview.storage,
          mounts: [{ ...mount, used_bytes: '1' }],
        },
      },
      {
        ...validOverview,
        network: {
          ...validOverview.network,
          interfaces: [
            {
              ...networkInterface,
              addresses: Array.from({ length: 17 }, () => ({
                address: '2001:db8::1',
                prefix_length: 64,
              })),
            },
          ],
        },
      },
      {
        ...validOverview,
        network: {
          ...validOverview.network,
          interfaces: Array.from({ length: 17 }, (_, index) => ({
            ...networkInterface,
            name: `eth${index}`,
            addresses: Array.from({ length: 16 }, (_, addressIndex) => ({
              address: `2001:db8:${index.toString(16)}::${addressIndex + 1}`,
              prefix_length: 64,
            })),
          })),
        },
      },
      {
        ...validOverview,
        network: {
          ...validOverview.network,
          omitted_addresses: 1,
        },
      },
      {
        ...validOverview,
        network: {
          ...validOverview.network,
          interfaces: [networkInterface, networkInterface],
        },
      },
      {
        ...validOverview,
        network: {
          ...validOverview.network,
          interfaces: [
            {
              ...networkInterface,
              addresses: [{ address: '192.0.2.10', prefix_length: 33 }],
            },
          ],
        },
      },
      {
        ...validOverview,
        network: {
          ...validOverview.network,
          interfaces: [
            { ...networkInterface, total_received_bytes: '0.5' },
          ],
        },
      },
      ...['+1', '-1', ' 1', '01', '1x', '18446744073709551616'].map(
        (invalidCounter) => ({
          ...validOverview,
          network: {
            ...validOverview.network,
            interfaces: [
              {
                ...networkInterface,
                total_received_bytes: invalidCounter,
              },
            ],
          },
        }),
      ),
    ];

    for (const payload of invalidPayloads) {
      expect(() => parseSystemOverview(payload)).toThrowError(ApiError);
    }
  });
});

describe('authentication response parsing', () => {
  const validUser = {
    id: '019c7714-3b77-44d1-9866-e1f484aae2ab',
    loginName: 'rique.owner',
    displayName: 'Rique',
    capabilities: ['system.view'],
  };

  it('accepts an explicit setup state without inventing an active token', () => {
    expect(
      parseSetupStatus({
        ownerExists: false,
        bootstrapAvailable: false,
        bootstrapExpiresAtUnixMs: null,
      }),
    ).toEqual({
      ownerExists: false,
      bootstrapAvailable: false,
      bootstrapExpiresAtUnixMs: null,
    });
  });

  it('parses a canonical authenticated session', () => {
    expect(
      parseAuthSession({
        user: validUser,
        csrfToken: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
        expiresAtUnixMs: 1_800_000_000_000,
      }),
    ).toEqual({
      user: validUser,
      csrfToken: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
      expiresAtUnixMs: 1_800_000_000_000,
    });
  });

  it('rejects malformed tokens, duplicate capabilities, and noncanonical names', () => {
    for (const user of [
      { ...validUser, capabilities: ['system.view', 'system.view'] },
      { ...validUser, displayName: 'Cafe\u0301' },
      { ...validUser, loginName: 'Rique' },
    ]) {
      expect(() =>
        parseAuthSession({
          user,
          csrfToken: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
          expiresAtUnixMs: 1_800_000_000_000,
        }),
      ).toThrowError(ApiError);
    }
    expect(() =>
      parseAuthSession({
        user: validUser,
        csrfToken: 'short',
        expiresAtUnixMs: 1_800_000_000_000,
      }),
    ).toThrowError(ApiError);
  });
});

describe('authentication requests', () => {
  const authResponse = {
    user: {
      id: '019c7714-3b77-44d1-9866-e1f484aae2ab',
      loginName: 'rique.owner',
      displayName: 'Rique',
      capabilities: ['system.view'],
    },
    csrfToken: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
    expiresAtUnixMs: 1_800_000_000_000,
  };

  it('posts credentials only as same-origin JSON', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(authResponse), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    );
    vi.stubGlobal('fetch', fetchMock);

    await login({ loginName: 'rique.owner', password: 'test password only' });

    expect(fetchMock).toHaveBeenCalledOnce();
    const [path, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toBe('/api/v1/auth/login');
    expect(request.method).toBe('POST');
    expect(request.credentials).toBe('same-origin');
    expect(request.headers).toMatchObject({
      Accept: 'application/json',
      'Content-Type': 'application/json',
    });
    expect(request.body).toBe(
      JSON.stringify({ loginName: 'rique.owner', password: 'test password only' }),
    );
  });

  it('preserves stable server status and code for generic login denial', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            code: 'login_rejected',
            message: 'The login could not be accepted.',
          }),
          { status: 401, headers: { 'Content-Type': 'application/json' } },
        ),
      ),
    );

    await expect(
      login({ loginName: 'unknown', password: 'test password only' }),
    ).rejects.toMatchObject({ status: 401, code: 'login_rejected' });
  });

  it('sends logout CSRF in a header and never in the URL', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal('fetch', fetchMock);
    const csrfToken = 'BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB';

    await logout(csrfToken);

    const [path, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toBe('/api/v1/auth/logout');
    expect(path).not.toContain(csrfToken);
    expect(request.headers).toMatchObject({ 'X-Helix-CSRF': csrfToken });
    expect(request.credentials).toBe('same-origin');
  });

  it('updates the owner with in-memory CSRF and accepts only an empty success', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal('fetch', fetchMock);
    const csrfToken = 'DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD';

    await updateAccount({
      currentPassword: 'current password only',
      loginName: 'rique.owner',
      displayName: 'Rique',
      newPassword: 'replacement password only',
    }, csrfToken);

    const [path, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toBe('/api/v1/auth/account');
    expect(request.headers).toMatchObject({ 'X-Helix-CSRF': csrfToken });
    expect(request.credentials).toBe('same-origin');
    expect(request.body).toBe(JSON.stringify({
      currentPassword: 'current password only',
      loginName: 'rique.owner',
      displayName: 'Rique',
      newPassword: 'replacement password only',
    }));
  });

  it('sends the in-memory CSRF proof on protected reads', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          status: 'ok',
          version: '0.1.0-alpha.1',
          state_database: 'ok',
          metrics_database: 'ok',
          timestamp_unix_ms: 1_800_000_000_000,
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      ),
    );
    vi.stubGlobal('fetch', fetchMock);
    const csrfToken = 'CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC';

    await getHealth(csrfToken);

    const [path, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(path).toBe('/api/v1/health');
    expect(request.headers).toMatchObject({ 'X-Helix-CSRF': csrfToken });
    expect(request.credentials).toBe('same-origin');
  });
});
