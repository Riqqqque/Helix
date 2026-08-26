//! Narrow, read-only host discovery used by the base daemon.

use helix_core::{
    CpuOverview, DiscoveryAvailability, HostOverview, MemoryOverview, NetworkAddressOverview,
    NetworkInterfaceOverview, NetworkOverview, StorageMountOverview, StorageOverview, SwapOverview,
    unix_timestamp_ms,
};
use std::{
    cmp::Ordering as CompareOrdering,
    collections::BinaryHeap,
    env,
    ffi::OsStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use sysinfo::{
    CpuRefreshKind, DiskRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System,
};

/// Response bounds keep a hostile or unusually complex host from producing an
/// unbounded API payload.
pub const MAX_STORAGE_MOUNTS: usize = 64;
pub const MAX_NETWORK_INTERFACES: usize = 64;
pub const MAX_ADDRESSES_PER_INTERFACE: usize = 16;
pub const MAX_NETWORK_ADDRESSES: usize = 256;
const MAX_HOST_TEXT_BYTES: usize = 32_768;

/// On-demand collector. It performs no background polling when no client asks
/// for a host overview.
#[derive(Clone)]
pub struct HostSampler {
    inner: Arc<Mutex<SamplerState>>,
    sample_in_flight: Arc<AtomicBool>,
}

struct SamplerState {
    system: System,
    disks: Disks,
    networks: Networks,
    last_cpu_refresh: Instant,
}

#[derive(Debug, Eq, PartialEq)]
struct StorageMountSample {
    name: Option<String>,
    file_system: Option<String>,
    mount_point: Option<String>,
    total_bytes: u64,
    available_bytes: u64,
    read_only: bool,
    removable: bool,
}

#[derive(Debug, Eq, PartialEq)]
struct NetworkInterfaceSample {
    name: String,
    addresses: Vec<NetworkAddressOverview>,
    discovered_address_count: usize,
    total_received_bytes: u64,
    total_transmitted_bytes: u64,
    mtu_bytes: u64,
}

impl Ord for StorageMountSample {
    fn cmp(&self, other: &Self) -> CompareOrdering {
        (
            &self.mount_point,
            &self.name,
            &self.file_system,
            self.total_bytes,
            self.available_bytes,
            self.read_only,
            self.removable,
        )
            .cmp(&(
                &other.mount_point,
                &other.name,
                &other.file_system,
                other.total_bytes,
                other.available_bytes,
                other.read_only,
                other.removable,
            ))
    }
}

impl PartialOrd for StorageMountSample {
    fn partial_cmp(&self, other: &Self) -> Option<CompareOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for NetworkInterfaceSample {
    fn cmp(&self, other: &Self) -> CompareOrdering {
        (
            &self.name,
            &self.addresses,
            self.discovered_address_count,
            self.total_received_bytes,
            self.total_transmitted_bytes,
            self.mtu_bytes,
        )
            .cmp(&(
                &other.name,
                &other.addresses,
                other.discovered_address_count,
                other.total_received_bytes,
                other.total_transmitted_bytes,
                other.mtu_bytes,
            ))
    }
}

impl PartialOrd for NetworkInterfaceSample {
    fn partial_cmp(&self, other: &Self) -> Option<CompareOrdering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug)]
pub struct HostSampleError;

/// An owned single-flight permit acquired before dispatching host sampling to
/// a blocking executor. Only one permit can exist for a sampler at a time.
#[must_use = "dropping the permit releases the host sampler without collecting a snapshot"]
pub struct HostSamplePermit {
    sampler: HostSampler,
}

impl std::fmt::Display for HostSampleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("host sampler mutex was poisoned")
    }
}

impl std::error::Error for HostSampleError {}

impl HostSampler {
    #[must_use]
    pub fn new() -> Self {
        let refreshes = RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
            .with_memory(MemoryRefreshKind::everything());
        let mut system = System::new_with_specifics(refreshes);
        system.refresh_memory();
        system.refresh_cpu_usage();
        Self {
            inner: Arc::new(Mutex::new(SamplerState {
                system,
                disks: Disks::new(),
                networks: Networks::new(),
                last_cpu_refresh: Instant::now(),
            })),
            sample_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Try to reserve the sampler before dispatching blocking work. This never
    /// waits: callers should reject or retry when another sample is in flight.
    #[must_use]
    pub fn try_acquire(&self) -> Option<HostSamplePermit> {
        self.sample_in_flight
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| HostSamplePermit {
                sampler: self.clone(),
            })
    }

    fn snapshot(&self) -> Result<HostOverview, HostSampleError> {
        let mut state = self.inner.lock().map_err(|_| HostSampleError)?;
        state.system.refresh_memory();

        let cpu_usage = if state.last_cpu_refresh.elapsed() >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL
        {
            state.system.refresh_cpu_usage();
            state.last_cpu_refresh = Instant::now();
            Some(state.system.global_cpu_usage())
        } else {
            None
        };

        let storage = collect_storage_overview(&mut state.disks);
        let network = collect_network_overview(&mut state.networks);

        let architecture = System::cpu_arch();
        Ok(HostOverview {
            hostname: System::host_name(),
            operating_system: System::long_os_version().or_else(System::name),
            architecture: if architecture.is_empty() {
                env::consts::ARCH.to_owned()
            } else {
                architecture
            },
            kernel_version: System::kernel_version(),
            uptime_seconds: System::uptime(),
            cpu: CpuOverview {
                usage_percent: cpu_usage,
                logical_cores: state.system.cpus().len(),
            },
            memory: MemoryOverview {
                total_bytes: state.system.total_memory(),
                used_bytes: state.system.used_memory(),
                available_bytes: state.system.available_memory(),
            },
            swap: SwapOverview {
                total_bytes: state.system.total_swap(),
                used_bytes: state.system.used_swap(),
            },
            storage,
            network,
            collected_at_unix_ms: unix_timestamp_ms(),
        })
    }
}

impl HostSamplePermit {
    /// Collect current memory and host facts. CPU becomes available only after
    /// sysinfo's documented minimum interval has elapsed since the prior sample.
    pub fn snapshot(self) -> Result<HostOverview, HostSampleError> {
        self.sampler.snapshot()
    }
}

impl Drop for HostSamplePermit {
    fn drop(&mut self) {
        self.sampler
            .sample_in_flight
            .store(false, Ordering::Release);
    }
}

impl Default for HostSampler {
    fn default() -> Self {
        Self::new()
    }
}

fn bounded_smallest<T, I>(items: I, limit: usize) -> (Vec<T>, usize)
where
    T: Ord,
    I: IntoIterator<Item = T>,
{
    let mut selected = BinaryHeap::with_capacity(limit);
    let mut discovered = 0_usize;
    for item in items {
        discovered = discovered.saturating_add(1);
        if selected.len() < limit {
            selected.push(item);
        } else if selected.peek().is_some_and(|largest| item < *largest) {
            selected.pop();
            selected.push(item);
        }
    }
    let mut selected = selected.into_vec();
    selected.sort_unstable();
    (selected, discovered)
}

fn collect_storage_overview(disks: &mut Disks) -> StorageOverview {
    if !sysinfo::IS_SUPPORTED_SYSTEM {
        return StorageOverview {
            availability: DiscoveryAvailability::Unavailable,
            mounts: Vec::new(),
            omitted_mounts: 0,
            omitted_text_fields: 0,
        };
    }

    // Storage-only refresh avoids device kind and I/O-stat probes. Because
    // linux-netdevs and linux-tmpfs remain disabled, sysinfo excludes those
    // potentially blocking/ephemeral mount classes before this list is built.
    disks.refresh_specifics(true, DiskRefreshKind::nothing().with_storage());
    let samples = disks.list().iter().map(|disk| StorageMountSample {
        name: strict_host_text(disk.name()),
        file_system: strict_host_text(disk.file_system()),
        mount_point: strict_host_text(disk.mount_point().as_os_str()),
        total_bytes: disk.total_space(),
        available_bytes: disk.available_space(),
        read_only: disk.is_read_only(),
        removable: disk.is_removable(),
    });
    storage_overview_from_samples(samples, true)
}

fn storage_overview_from_samples<I>(samples: I, supported: bool) -> StorageOverview
where
    I: IntoIterator<Item = StorageMountSample>,
{
    if !supported {
        return StorageOverview {
            availability: DiscoveryAvailability::Unavailable,
            mounts: Vec::new(),
            omitted_mounts: 0,
            omitted_text_fields: 0,
        };
    }

    let (samples, discovered_mounts) = bounded_smallest(samples, MAX_STORAGE_MOUNTS);
    let omitted_mounts = discovered_mounts.saturating_sub(samples.len());
    let mut omitted_text_fields = 0usize;
    let mut inconsistent_capacity = false;
    let mounts = samples
        .into_iter()
        .map(|mount| {
            omitted_text_fields = omitted_text_fields
                .saturating_add(if mount.name.is_none() { 1 } else { 0 })
                .saturating_add(if mount.file_system.is_none() { 1 } else { 0 })
                .saturating_add(if mount.mount_point.is_none() { 1 } else { 0 });
            let (available_bytes, used_bytes) =
                match mount.total_bytes.checked_sub(mount.available_bytes) {
                    Some(used_bytes) => (mount.available_bytes, used_bytes),
                    None => {
                        inconsistent_capacity = true;
                        (mount.total_bytes, 0)
                    }
                };
            StorageMountOverview {
                name: mount.name,
                file_system: mount.file_system,
                mount_point: mount.mount_point,
                total_bytes: mount.total_bytes,
                available_bytes,
                used_bytes,
                read_only: mount.read_only,
                removable: mount.removable,
            }
        })
        .collect();

    StorageOverview {
        availability: if omitted_mounts == 0 && omitted_text_fields == 0 && !inconsistent_capacity {
            DiscoveryAvailability::Available
        } else {
            DiscoveryAvailability::Degraded
        },
        mounts,
        omitted_mounts,
        omitted_text_fields,
    }
}

fn collect_network_overview(networks: &mut Networks) -> NetworkOverview {
    if !sysinfo::IS_SUPPORTED_SYSTEM {
        return network_overview_from_samples(Vec::new(), false);
    }

    networks.refresh(true);
    let samples = networks.list().iter().map(|(name, data)| {
        let (addresses, discovered_address_count) = bounded_smallest(
            data.ip_networks()
                .iter()
                .map(|network| NetworkAddressOverview {
                    address: network.addr.to_string(),
                    prefix_length: network.prefix,
                }),
            MAX_ADDRESSES_PER_INTERFACE,
        );
        NetworkInterfaceSample {
            name: name.clone(),
            addresses,
            discovered_address_count,
            total_received_bytes: data.total_received(),
            total_transmitted_bytes: data.total_transmitted(),
            mtu_bytes: data.mtu(),
        }
    });
    network_overview_from_samples(samples, true)
}

fn network_overview_from_samples<I>(samples: I, supported: bool) -> NetworkOverview
where
    I: IntoIterator<Item = NetworkInterfaceSample>,
{
    if !supported {
        return NetworkOverview {
            availability: DiscoveryAvailability::Unavailable,
            interfaces: Vec::new(),
            omitted_interfaces: 0,
            omitted_addresses: 0,
        };
    }

    let mut total_address_count = 0_usize;
    let counted_samples = samples.into_iter().inspect(|sample| {
        total_address_count = total_address_count
            .saturating_add(sample.discovered_address_count.max(sample.addresses.len()));
    });
    let (mut samples, discovered_interfaces) =
        bounded_smallest(counted_samples, MAX_NETWORK_INTERFACES);
    let omitted_interfaces = discovered_interfaces.saturating_sub(samples.len());
    for interface in &mut samples {
        let (addresses, _) = bounded_smallest(
            std::mem::take(&mut interface.addresses),
            MAX_ADDRESSES_PER_INTERFACE,
        );
        interface.addresses = addresses;
    }
    let mut remaining_address_slots = MAX_NETWORK_ADDRESSES;
    let interfaces = samples
        .into_iter()
        .map(|mut interface| {
            let keep = interface
                .addresses
                .len()
                .min(MAX_ADDRESSES_PER_INTERFACE)
                .min(remaining_address_slots);
            interface.addresses.truncate(keep);
            remaining_address_slots = remaining_address_slots.saturating_sub(keep);
            NetworkInterfaceOverview {
                name: interface.name,
                addresses: interface.addresses,
                total_received_bytes: interface.total_received_bytes,
                total_transmitted_bytes: interface.total_transmitted_bytes,
                mtu_bytes: interface.mtu_bytes,
            }
        })
        .collect::<Vec<_>>();
    let returned_address_count = interfaces.iter().fold(0usize, |count, interface| {
        count.saturating_add(interface.addresses.len())
    });
    let omitted_addresses = total_address_count.saturating_sub(returned_address_count);

    NetworkOverview {
        availability: if omitted_interfaces == 0 && omitted_addresses == 0 {
            DiscoveryAvailability::Available
        } else {
            DiscoveryAvailability::Degraded
        },
        interfaces,
        omitted_interfaces,
        omitted_addresses,
    }
}

fn strict_host_text(value: &OsStr) -> Option<String> {
    value
        .to_str()
        .filter(|text| {
            !text.trim().is_empty()
                && text.len() <= MAX_HOST_TEXT_BYTES
                && !text
                    .chars()
                    .any(|character| character <= '\u{1f}' || character == '\u{7f}')
        })
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_contains_real_host_shape_without_polling_task() {
        let snapshot = HostSampler::new()
            .try_acquire()
            .expect("sampling permit")
            .snapshot()
            .expect("host snapshot");

        assert!(!snapshot.architecture.is_empty());
        assert!(snapshot.memory.used_bytes <= snapshot.memory.total_bytes);
        assert!(snapshot.memory.available_bytes <= snapshot.memory.total_bytes);
        assert!(snapshot.storage.mounts.len() <= MAX_STORAGE_MOUNTS);
        assert!(snapshot.network.interfaces.len() <= MAX_NETWORK_INTERFACES);
        assert!(
            snapshot
                .network
                .interfaces
                .iter()
                .all(|interface| interface.addresses.len() <= MAX_ADDRESSES_PER_INTERFACE)
        );
        assert!(
            snapshot
                .network
                .interfaces
                .iter()
                .map(|interface| interface.addresses.len())
                .sum::<usize>()
                <= MAX_NETWORK_ADDRESSES
        );
    }

    #[test]
    fn sampling_permit_is_single_flight_and_released_on_drop() {
        let sampler = HostSampler::new();
        let first = sampler.try_acquire().expect("first permit");

        for _ in 0..128 {
            assert!(sampler.try_acquire().is_none());
        }

        drop(first);
        assert!(sampler.try_acquire().is_some());
    }

    #[test]
    fn storage_is_sorted_bounded_and_clamps_inconsistent_capacity() {
        let mut samples = (0..=MAX_STORAGE_MOUNTS)
            .rev()
            .map(|index| StorageMountSample {
                name: Some(format!("disk-{index:03}")),
                file_system: Some("testfs".to_owned()),
                mount_point: Some(format!("/mount/{index:03}")),
                total_bytes: 1_000,
                available_bytes: 250,
                read_only: false,
                removable: false,
            })
            .collect::<Vec<_>>();
        samples[MAX_STORAGE_MOUNTS / 2].available_bytes = 2_000;
        samples[MAX_STORAGE_MOUNTS / 2].name = None;

        let overview = storage_overview_from_samples(samples, true);

        assert_eq!(overview.availability, DiscoveryAvailability::Degraded);
        assert_eq!(overview.mounts.len(), MAX_STORAGE_MOUNTS);
        assert_eq!(overview.omitted_mounts, 1);
        assert_eq!(overview.omitted_text_fields, 1);
        assert!(overview.mounts.windows(2).all(|pair| {
            pair[0].mount_point.as_deref().unwrap_or_default()
                <= pair[1].mount_point.as_deref().unwrap_or_default()
        }));
        assert!(overview.mounts.iter().all(|mount| {
            mount.available_bytes <= mount.total_bytes
                && mount.used_bytes == mount.total_bytes.saturating_sub(mount.available_bytes)
        }));
        let clamped = overview
            .mounts
            .iter()
            .find(|mount| mount.name.is_none())
            .expect("mount with invalid text marker");
        assert_eq!(clamped.available_bytes, clamped.total_bytes);
        assert_eq!(clamped.used_bytes, 0);
    }

    #[test]
    fn storage_reports_unsupported_collector_without_entries() {
        let overview = storage_overview_from_samples(
            vec![StorageMountSample {
                name: Some("ignored".to_owned()),
                file_system: Some("ignored".to_owned()),
                mount_point: Some("ignored".to_owned()),
                total_bytes: 1,
                available_bytes: 1,
                read_only: false,
                removable: false,
            }],
            false,
        );

        assert_eq!(overview.availability, DiscoveryAvailability::Unavailable);
        assert!(overview.mounts.is_empty());
        assert_eq!(overview.omitted_mounts, 0);
    }

    #[test]
    fn hostile_discovery_counts_keep_deterministic_top_k_and_exact_omissions() {
        const HOSTILE_MOUNTS: usize = 4_096;
        let storage = storage_overview_from_samples(
            (0..HOSTILE_MOUNTS).rev().map(|index| StorageMountSample {
                name: Some(format!("disk-{index:05}")),
                file_system: Some("testfs".to_owned()),
                mount_point: Some(format!("/mount/{index:05}")),
                total_bytes: 1_000,
                available_bytes: 250,
                read_only: false,
                removable: false,
            }),
            true,
        );
        assert_eq!(storage.mounts.len(), MAX_STORAGE_MOUNTS);
        assert_eq!(storage.omitted_mounts, HOSTILE_MOUNTS - MAX_STORAGE_MOUNTS);
        assert_eq!(
            storage
                .mounts
                .first()
                .and_then(|mount| mount.mount_point.as_deref()),
            Some("/mount/00000")
        );
        assert_eq!(
            storage
                .mounts
                .last()
                .and_then(|mount| mount.mount_point.as_deref()),
            Some("/mount/00063")
        );

        const HOSTILE_INTERFACES: usize = 1_024;
        const HOSTILE_ADDRESSES_PER_INTERFACE: usize = 128;
        let network = network_overview_from_samples(
            (0..HOSTILE_INTERFACES).rev().map(|index| {
                let (addresses, discovered_address_count) = bounded_smallest(
                    (0..HOSTILE_ADDRESSES_PER_INTERFACE).rev().map(|address| {
                        NetworkAddressOverview {
                            address: format!("address-{address:03}"),
                            prefix_length: 64,
                        }
                    }),
                    MAX_ADDRESSES_PER_INTERFACE,
                );
                NetworkInterfaceSample {
                    name: format!("interface-{index:04}"),
                    addresses,
                    discovered_address_count,
                    total_received_bytes: 0,
                    total_transmitted_bytes: 0,
                    mtu_bytes: 1_500,
                }
            }),
            true,
        );
        assert_eq!(network.interfaces.len(), MAX_NETWORK_INTERFACES);
        assert_eq!(
            network.omitted_interfaces,
            HOSTILE_INTERFACES - MAX_NETWORK_INTERFACES
        );
        let returned_addresses = network
            .interfaces
            .iter()
            .map(|interface| interface.addresses.len())
            .sum::<usize>();
        assert_eq!(returned_addresses, MAX_NETWORK_ADDRESSES);
        assert_eq!(
            network.omitted_addresses,
            HOSTILE_INTERFACES * HOSTILE_ADDRESSES_PER_INTERFACE - returned_addresses
        );
        assert_eq!(network.interfaces[0].name, "interface-0000");
        assert_eq!(
            network.interfaces[MAX_NETWORK_INTERFACES - 1].name,
            "interface-0063"
        );
    }

    #[test]
    fn empty_control_and_oversized_host_text_is_explicitly_omitted() {
        assert_eq!(strict_host_text(OsStr::new("")), None);
        assert_eq!(strict_host_text(OsStr::new("   \t")), None);
        assert_eq!(strict_host_text(OsStr::new("disk\nname")), None);
        assert_eq!(
            strict_host_text(OsStr::new(&"x".repeat(MAX_HOST_TEXT_BYTES + 1))),
            None
        );
        assert_eq!(
            strict_host_text(OsStr::new("Windows (C:)")),
            Some("Windows (C:)".to_owned())
        );
    }

    #[cfg(unix)]
    #[test]
    fn invalid_unix_host_text_is_explicitly_omitted() {
        use std::os::unix::ffi::OsStrExt;

        assert_eq!(strict_host_text(OsStr::from_bytes(&[0xff])), None);
    }

    #[cfg(windows)]
    #[test]
    fn invalid_windows_host_text_is_explicitly_omitted() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};

        let unpaired_surrogate = OsString::from_wide(&[0xd800]);
        assert_eq!(strict_host_text(&unpaired_surrogate), None);
    }

    #[test]
    fn network_is_sorted_and_enforces_interface_and_address_caps() {
        let samples = (0..=MAX_NETWORK_INTERFACES)
            .rev()
            .map(|index| NetworkInterfaceSample {
                name: format!("interface-{index:03}"),
                addresses: (0..=MAX_ADDRESSES_PER_INTERFACE)
                    .rev()
                    .map(|address| NetworkAddressOverview {
                        address: format!("192.0.2.{address}"),
                        prefix_length: 24,
                    })
                    .collect(),
                discovered_address_count: MAX_ADDRESSES_PER_INTERFACE + 1,
                total_received_bytes: u64::MAX,
                total_transmitted_bytes: u64::MAX,
                mtu_bytes: 1_500,
            })
            .collect::<Vec<_>>();

        let overview = network_overview_from_samples(samples, true);

        assert_eq!(overview.availability, DiscoveryAvailability::Degraded);
        assert_eq!(overview.interfaces.len(), MAX_NETWORK_INTERFACES);
        assert_eq!(overview.omitted_interfaces, 1);
        assert_eq!(
            overview
                .interfaces
                .iter()
                .map(|interface| interface.addresses.len())
                .sum::<usize>(),
            MAX_NETWORK_ADDRESSES
        );
        assert_eq!(
            overview.omitted_addresses,
            (MAX_NETWORK_INTERFACES + 1) * (MAX_ADDRESSES_PER_INTERFACE + 1)
                - MAX_NETWORK_ADDRESSES
        );
        assert!(
            overview
                .interfaces
                .windows(2)
                .all(|pair| pair[0].name < pair[1].name)
        );
        assert!(overview.interfaces.iter().all(|interface| {
            interface
                .addresses
                .windows(2)
                .all(|pair| pair[0] <= pair[1])
        }));
    }

    #[test]
    fn network_total_counters_are_preserved_without_deriving_rates() {
        let overview = network_overview_from_samples(
            vec![NetworkInterfaceSample {
                name: "lan0".to_owned(),
                addresses: vec![NetworkAddressOverview {
                    address: "2001:db8::1".to_owned(),
                    prefix_length: 64,
                }],
                discovered_address_count: 1,
                total_received_bytes: u64::MAX,
                total_transmitted_bytes: u64::MAX - 1,
                mtu_bytes: 1_500,
            }],
            true,
        );

        assert_eq!(overview.availability, DiscoveryAvailability::Available);
        assert_eq!(overview.interfaces[0].total_received_bytes, u64::MAX);
        assert_eq!(overview.interfaces[0].total_transmitted_bytes, u64::MAX - 1);
    }

    #[test]
    fn network_reports_unsupported_collector_without_entries() {
        let overview = network_overview_from_samples(
            vec![NetworkInterfaceSample {
                name: "ignored".to_owned(),
                addresses: Vec::new(),
                discovered_address_count: 0,
                total_received_bytes: 1,
                total_transmitted_bytes: 1,
                mtu_bytes: 1_500,
            }],
            false,
        );

        assert_eq!(overview.availability, DiscoveryAvailability::Unavailable);
        assert!(overview.interfaces.is_empty());
        assert_eq!(overview.omitted_interfaces, 0);
        assert_eq!(overview.omitted_addresses, 0);
    }
}
