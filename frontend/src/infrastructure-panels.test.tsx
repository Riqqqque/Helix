import { render } from "preact-render-to-string";
import { describe, expect, it } from "vitest";
import { PackageInventoryView } from "./host-updates";
import { NetworkEvidenceView } from "./network-panel";
import type { NetworkInventory } from "./network-api";
import type { SystemPackageInventory } from "./package-api";

const ruleId = "6b8f95ce-9c58-4c4c-b232-627a29ca1c03";
const network: NetworkInventory = {
  collectedAtUnixMs: 1_800_000_000_000,
  addresses: {
    privateIpv4: "192.168.1.20",
    source: "router_path",
    note: "LAN address.",
  },
  router: {
    automaticPortForwardingAvailable: true,
    discovery: "upnp_igd",
    state: "available",
    externalIpv4: "203.0.113.8",
    externalAddressKind: "public",
    privateIpv4: "192.168.1.20",
    error: null,
    note: "Router available.",
  },
  listeners: {
    source: "linux_proc_net",
    items: [
      {
        protocol: "tcp",
        family: "ipv4",
        address: "0.0.0.0",
        port: 25565,
        wildcard: true,
        uid: 1000,
        inode: 1,
        process: { pid: 42, name: "java" },
      },
    ],
    truncated: false,
    ownerProcessBestEffort: true,
  },
  docker: {
    installed: true,
    publications: [
      {
        containerId: "abcdef",
        containerName: "minecraft",
        composeService: null,
        protocol: "tcp",
        containerPort: 25565,
        hostAddress: "0.0.0.0",
        hostPort: 25565,
      },
    ],
    containersTruncated: false,
    error: null,
    note: "Publication is separate evidence.",
  },
  firewall: {
    backend: "ufw",
    installed: true,
    active: true,
    status: "active",
    defaultPolicy: { incoming: "deny", outgoing: "allow", routed: null },
    rules: [],
    rulesTruncated: false,
    helixManagedRuleState: [
      {
        ruleId,
        name: "Survival",
        description: "Game server",
        protocol: "tcp",
        portStart: 25565,
        portEnd: 25565,
        state: "active",
        createdAtUnixMs: 1_800_000_000_000,
        trashedAtUnixMs: null,
        undoAvailable: false,
        undoExpiresAtUnixMs: null,
        observedInUfw: true,
        exactBodyVerified: true,
      },
    ],
    error: null,
    mutationsSupported: true,
    mutationScope: "Only exact managed rules can change.",
    inactiveNote: null,
  },
  gamePorts: [
    {
      instanceId: "one",
      name: "Survival",
      manager: "helix_native",
      port: 25565,
      protocol: "tcp",
      serverReportedRunning: true,
      listenerBound: true,
      dockerPublished: true,
      dockerPublications: [
        {
          containerId: "abcdef",
          containerName: "minecraft",
          composeService: null,
          protocol: "tcp",
          containerPort: 25565,
          hostAddress: "0.0.0.0",
          hostPort: 25565,
        },
      ],
      firewallInputAllowance: {
        applicable: true,
        allowed: true,
        state: "allowed",
      },
      privateJoinAddress: "192.168.1.20:25565",
      externalReachability: {
        state: "setup_available",
        reachable: null,
        testedFromExternalNetwork: false,
        routerMappingVerified: false,
        externalIp: "203.0.113.8",
        joinAddress: "203.0.113.8:25565",
        verifiedAtUnixMs: null,
        note: "Not tested externally.",
      },
    },
  ],
  gamePortInventoryErrors: [],
  externalReachability: {
    state: "unverified",
    testedFromExternalNetwork: false,
  },
};

const packages: SystemPackageInventory = {
  availability: "ready",
  collectedAtUnixMs: 1_800_000_000_000,
  aptCacheRefreshedAtUnixMs: 1_799_999_000_000,
  aptCacheRefreshPerformed: false,
  inventory: {
    installedTotal: 1,
    upgradeAvailableTotal: 1,
    securityUpdateTotal: 1,
    truncated: false,
    packages: [
      {
        name: "openssl",
        installedVersion: "3.0.1",
        candidateVersion: "3.0.2",
        upgradeAvailable: true,
        held: false,
        downloadSizeBytes: 2_000_000,
        installedSizeBytes: 5_000_000,
        sourcePackage: "openssl",
        candidateOrigin: "Ubuntu-Security",
        category: "libs",
        description: "Secure sockets toolkit",
        securityUpdate: true,
        restartHint: "unknown",
        restartImpactKnown: false,
      },
    ],
  },
  simulation: {
    available: true,
    upgradeCandidates: 1,
    newPackages: 0,
    removals: 0,
    heldBack: 0,
    error: null,
    stateCanChangeAfterSimulation: true,
    mutatedPackageState: false,
  },
  hostRestart: {
    rebootRequiredMarkerPresent: false,
    packages: [],
    automaticReboot: false,
  },
  upgradeApply: {
    available: true,
    reasonCode: "selected_exact_candidates_supported",
    reason: "Exact candidates are revalidated before apply.",
    wouldRequireExplicitPackageCandidates: true,
    wouldRequireDisruptionAcknowledgement: true,
    requiredCapability: "system.packages.write",
    rollbackClaimed: false,
    automaticReboot: false,
    aptOrDpkgMutationAvailable: true,
    packageListsRefreshAvailable: true,
    conffilePolicy: "preserve_existing",
    newPackagesAllowed: false,
    packageRemovalsAllowed: false,
  },
  helixSelfUpdate: {
    available: false,
    reasonCode: "verified_release_pipeline_not_implemented",
    reason: "Signed staged releases and health rollback are required.",
    gitPullUsed: false,
  },
  tools: { dpkgQuery: true, aptCache: true, aptGet: true, aptMark: true },
  errors: [],
};

describe("infrastructure evidence panels", () => {
  it("does not turn local evidence into an outside reachability claim", () => {
    const markup = render(
      <NetworkEvidenceView
        inventory={network}
        canManageFirewall
        busyRuleId={null}
        pendingDelete={null}
        onPendingDelete={() => undefined}
        onDelete={() => undefined}
        onRestore={() => undefined}
      />,
    );
    expect(markup).toContain("Local listener");
    expect(markup).toContain("Docker binding");
    expect(markup).toContain("UFW INPUT");
    expect(markup).toContain("Outside host");
    expect(markup).toContain("Unknown");
    expect(markup).toContain("Not tested externally");
    expect(markup).not.toContain("Open to internet");
  });

  it("renders exact update detail and keeps apply disabled until a package is selected", () => {
    const markup = render(
      <PackageInventoryView
        data={packages}
        filter="updates"
        query=""
        page={0}
        onFilter={() => undefined}
        onQuery={() => undefined}
        onPage={() => undefined}
        selected={new Set()}
        onToggleSelected={() => undefined}
        onSelectSafeUpdates={() => undefined}
        onApplySelected={() => undefined}
        mutationBusy={false}
      />,
    );
    expect(markup).toContain("openssl");
    expect(markup).toContain("3.0.1");
    expect(markup).toContain("3.0.2");
    expect(markup).toContain("1.9 MiB");
    expect(markup).toContain("0 selected");
    expect(markup).toContain("Select safe updates (1)");
    expect(markup).toContain("Self-update unavailable");
    expect(markup.match(/disabled/g)?.length).toBeGreaterThanOrEqual(2);
  });
});
