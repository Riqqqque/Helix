import { afterEach, describe, expect, it, vi } from "vitest";
import {
  applySystemPackageUpdates,
  getSystemPackageInventory,
  parseSystemPackageInventory,
  refreshSystemPackageLists,
} from "./package-api";

const response = {
  schema_version: 1,
  availability: "ready",
  collected_at_unix_ms: 1_800_000_000_000,
  apt_cache_refreshed_at_unix_ms: 1_799_999_000_000,
  apt_cache_refresh_performed: false,
  inventory: {
    installed_total: 1200,
    upgrade_available_total: 1,
    security_update_total: 1,
    truncated: false,
    packages: [
      {
        name: "openssl",
        installed_version: "3.0.1",
        candidate_version: "3.0.2",
        upgrade_available: true,
        held: false,
        download_size_bytes: 2_000_000,
        installed_size_bytes: 5_000_000,
        source_package: "openssl",
        candidate_origin: "Ubuntu-Security",
        category: "libs",
        description: "Secure sockets toolkit",
        security_update: true,
        restart_hint: "unknown",
        restart_impact_known: false,
      },
    ],
  },
  simulation: {
    available: true,
    upgrade_candidates: 1,
    new_packages: 0,
    removals: 0,
    held_back: 0,
    error: null,
    state_can_change_after_simulation: true,
    mutated_package_state: false,
  },
  host_restart: {
    reboot_required_marker_present: false,
    packages: [],
    automatic_reboot: false,
  },
  upgrade_apply: {
    available: true,
    reason_code: "selected_exact_candidates_supported",
    reason: "Selected candidates are revalidated before one background job.",
    would_require_explicit_package_candidates: true,
    would_require_disruption_acknowledgement: true,
    required_capability: "system.packages.write",
    rollback_claimed: false,
    automatic_reboot: false,
    apt_or_dpkg_mutation_available: true,
    package_lists_refresh_available: true,
    conffile_policy: "preserve_existing",
    new_packages_allowed: false,
    package_removals_allowed: false,
  },
  helix_self_update: {
    available: false,
    reason_code: "verified_release_pipeline_not_implemented",
    reason: "Signed staged releases and health rollback are required.",
    git_pull_used: false,
  },
  tools: { dpkg_query: true, apt_cache: true, apt_get: true, apt_mark: true },
  errors: [],
};

afterEach(() => vi.unstubAllGlobals());

describe("package inventory API", () => {
  it("parses versions, sizes, update hints, and the bounded write readiness", () => {
    const parsed = parseSystemPackageInventory(response);
    expect(parsed.inventory.packages[0]).toMatchObject({
      name: "openssl",
      upgradeAvailable: true,
      securityUpdate: true,
      downloadSizeBytes: 2_000_000,
    });
    expect(parsed.upgradeApply).toMatchObject({
      available: true,
      rollbackClaimed: false,
      automaticReboot: false,
      newPackagesAllowed: false,
      packageRemovalsAllowed: false,
    });
    expect(parsed.helixSelfUpdate).toMatchObject({
      available: false,
      gitPullUsed: false,
    });
  });

  it("rejects a response that claims an inventory refresh mutated state or package apply allows removals", () => {
    expect(() =>
      parseSystemPackageInventory({
        ...response,
        apt_cache_refresh_performed: true,
      }),
    ).toThrow();
    expect(() =>
      parseSystemPackageInventory({
        ...response,
        upgrade_apply: {
          ...response.upgrade_apply,
          package_removals_allowed: true,
        },
      }),
    ).toThrow();
  });

  it("calls only the read-only package endpoint", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(
        new Response(JSON.stringify(response), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      );
    vi.stubGlobal("fetch", fetchMock);
    await getSystemPackageInventory(
      "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE",
    );
    expect(fetchMock).toHaveBeenCalledOnce();
    expect(fetchMock.mock.calls[0]?.[0]).toBe("/api/v1/system/packages");
    expect((fetchMock.mock.calls[0]?.[1] as RequestInit).method).toBe("GET");
  });

  it("posts only exact selected versions and a separate package-list refresh", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({ job_id: "12345678-1234-4234-8234-123456789abc" }),
          { status: 200 },
        ),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({ job_id: "22345678-1234-4234-8234-123456789abc" }),
          { status: 200 },
        ),
      );
    vi.stubGlobal("fetch", fetchMock);
    const parsed = parseSystemPackageInventory(response);
    await refreshSystemPackageLists(
      "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE",
    );
    await applySystemPackageUpdates(
      parsed.inventory.packages,
      "APPLY 1 UPDATE",
      true,
      "EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE",
    );
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "/api/v1/system/packages/refresh",
    );
    expect(
      JSON.parse(String((fetchMock.mock.calls[1]?.[1] as RequestInit).body)),
    ).toEqual({
      packages: [
        {
          name: "openssl",
          installed_version: "3.0.1",
          candidate_version: "3.0.2",
        },
      ],
      confirmation: "APPLY 1 UPDATE",
      disruption_acknowledged: true,
    });
  });
});
