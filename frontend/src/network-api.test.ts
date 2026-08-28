import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  createFirewallRule,
  deleteFirewallRule,
  getNetworkInventory,
  parseNetworkInventory,
  restoreFirewallRule,
  validateFirewallRuleSpec,
} from './network-api';

const ruleId = '6b8f95ce-9c58-4c4c-b232-627a29ca1c03';
const inventory = {
  schema_version: 1,
  collected_at_unix_ms: 1_800_000_000_000,
  addresses: { private_ipv4: '192.168.1.20', source: 'router_path', note: 'LAN address.' },
  router: { automatic_port_forwarding_available: true, discovery: 'upnp_igd', state: 'available', external_ipv4: '8.8.8.8', external_address_kind: 'public', private_ipv4: '192.168.1.20', error: null, note: 'Router available.' },
  listeners: {
    source: 'linux_proc_net',
    items: [{ protocol: 'tcp', family: 'ipv4', address: '0.0.0.0', port: 25565, wildcard: true, uid: 1000, inode: 55, process: { pid: 42, name: 'java' } }],
    truncated: false,
    owner_process_best_effort: true,
  },
  docker: {
    installed: true,
    publications: [{ container_id: 'abcdef', container_name: 'minecraft', compose_service: null, protocol: 'tcp', container_port: 25565, host_address: '0.0.0.0', host_port: 25565 }],
    containers_truncated: false,
    error: null,
    note: 'Docker publication is separate evidence.',
  },
  firewall: {
    backend: 'ufw', installed: true, active: true, status: 'active',
    default_policy: { incoming: 'deny', outgoing: 'allow', routed: 'disabled' },
    rules: [{ number: 1, display: '25565/tcp ALLOW IN Anywhere', action: 'ALLOW IN', source: 'Anywhere', protocol: 'tcp', port_start: 25565, port_end: 25565, comment: `helix:${ruleId}`, helix_owned: true, rule_id: ruleId, managed: true, management_state: 'active', name: 'Survival', description: 'Primary server' }],
    rules_truncated: false,
    helix_managed_rule_state: [{ rule_id: ruleId, name: 'Survival', description: 'Primary server', protocol: 'tcp', port_start: 25565, port_end: 25565, state: 'active', created_at_unix_ms: 1_800_000_000_000, trashed_at_unix_ms: null, undo_available: false, undo_expires_at_unix_ms: null, observed_in_ufw: true, exact_body_verified: true }],
    error: null,
    mutations_supported: true,
    mutation_scope: 'Only exact Helix rules are changed.',
    inactive_note: null,
  },
  game_ports: [{
    instance_id: 'native-1', name: 'Survival', manager: 'helix_native', port: 25565, protocol: 'tcp', server_reported_running: true,
    listener_bound: true, docker_published: true, docker_publications: [{ container_id: 'abcdef', container_name: 'minecraft', compose_service: null, protocol: 'tcp', container_port: 25565, host_address: '0.0.0.0', host_port: 25565 }],
    firewall_input_allowance: { applicable: true, allowed: true, state: 'allowed' },
    private_join_address: '192.168.1.20:25565',
    external_reachability: { state: 'setup_available', reachable: null, tested_from_external_network: false, router_mapping_verified: false, external_ip: '8.8.8.8', join_address: '8.8.8.8:25565', note: 'Not tested outside the host.' },
  }],
  game_port_inventory_errors: [],
  external_reachability: { state: 'unverified', tested_from_external_network: false },
};

afterEach(() => vi.unstubAllGlobals());

describe('network API', () => {
  it('keeps listener, Docker, firewall, and outside evidence separate', () => {
    const parsed = parseNetworkInventory(inventory);
    expect(parsed.listeners.items[0]).toMatchObject({ port: 25565, process: { name: 'java' } });
    expect(parsed.gamePorts[0]).toMatchObject({
      listenerBound: true,
      dockerPublished: true,
      firewallInputAllowance: { allowed: true },
      externalReachability: { state: 'setup_available', reachable: null },
    });
    expect(parsed.firewall.helixManagedRuleState[0]?.ruleId).toBe(ruleId);
  });

  it('rejects invented external verification and unsupported schemas', () => {
    expect(() => parseNetworkInventory({ ...inventory, schema_version: 2 })).toThrow();
    expect(() => parseNetworkInventory({ ...inventory, external_reachability: { state: 'unverified', tested_from_external_network: true } })).toThrow();
  });

  it('validates bounded single ports and ranges', () => {
    expect(validateFirewallRuleSpec({ name: ' Web ', description: ' Local app ', protocol: 'tcp', portStart: 8080, portEnd: 8081 })).toEqual({
      name: 'Web', description: 'Local app', protocol: 'tcp', portStart: 8080, portEnd: 8081,
    });
    expect(() => validateFirewallRuleSpec({ name: '', description: '', protocol: 'tcp', portStart: 1, portEnd: 1 })).toThrow();
    expect(() => validateFirewallRuleSpec({ name: 'Too wide', description: '', protocol: 'udp', portStart: 1, portEnd: 1025 })).toThrow();
  });

  it('uses exact read, create, trash, and restore contracts', async () => {
    const evidence = { installed: true, active: true, status: 'active', rule_count: 1, captured_at_unix_ms: 1_800_000_000_100 };
    const rule = { name: 'Web', description: 'Local app', protocol: 'tcp', port_start: 8080, port_end: 8080 };
    const responses = [
      inventory,
      { rule_id: ruleId, state: 'active', rule, created_at_unix_ms: 1_800_000_000_100, undo_available: false, before_evidence: evidence, after_evidence: evidence, verified: true },
      { rule_id: ruleId, state: 'trashed', original_rule: rule, trashed_at_unix_ms: 1_800_000_000_200, undo_available: true, undo_expires_at_unix_ms: 1_800_000_900_000, before_evidence: evidence, after_evidence: evidence, verified: true },
      { rule_id: ruleId, state: 'active', rule, restored_at_unix_ms: 1_800_000_000_300, undo_available: false, before_evidence: evidence, after_evidence: evidence, verified: true },
    ];
    const fetchMock = vi.fn().mockImplementation(() => Promise.resolve(new Response(JSON.stringify(responses.shift()), {
      status: 200, headers: { 'Content-Type': 'application/json' },
    })));
    vi.stubGlobal('fetch', fetchMock);
    const csrf = 'EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE';

    await getNetworkInventory(csrf);
    await createFirewallRule({ name: 'Web', description: 'Local app', protocol: 'tcp', portStart: 8080, portEnd: 8080 }, csrf);
    await deleteFirewallRule(ruleId, csrf);
    await restoreFirewallRule(ruleId, csrf);

    expect(fetchMock.mock.calls.map((call) => call[0])).toEqual([
      '/api/v1/network/inventory',
      '/api/v1/network/firewall/rules',
      `/api/v1/network/firewall/rules/${ruleId}`,
      `/api/v1/network/firewall/rules/${ruleId}/restore`,
    ]);
    const create = fetchMock.mock.calls[1]?.[1] as RequestInit;
    expect(create.method).toBe('POST');
    expect(JSON.parse(String(create.body))).toEqual({ name: 'Web', description: 'Local app', protocol: 'tcp', port_start: 8080, port_end: 8080 });
    const remove = fetchMock.mock.calls[2]?.[1] as RequestInit;
    expect(remove.method).toBe('DELETE');
    expect(remove.body).toBe('{}');
  });
});
