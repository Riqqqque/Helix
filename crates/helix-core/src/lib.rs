//! Shared, platform-neutral Helix domain models.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

mod u64_decimal {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.is_empty()
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(D::Error::custom("expected a canonical decimal u64 string"));
        }
        value
            .parse()
            .map_err(|_| D::Error::custom("decimal u64 is out of range"))
    }
}

/// Version compiled into the current Helix process.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Overall readiness of the local control plane.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Degraded,
}

/// Last known condition of one SQLite durability domain.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseStatus {
    Ok,
    Recovered,
    Unavailable,
}

/// Public readiness response. It deliberately contains no paths or host secrets.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub version: String,
    pub state_database: DatabaseStatus,
    pub metrics_database: DatabaseStatus,
    pub timestamp_unix_ms: u64,
}

/// A point-in-time CPU reading.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CpuOverview {
    /// `None` until two samples are far enough apart to be meaningful.
    pub usage_percent: Option<f32>,
    pub logical_cores: usize,
}

/// A point-in-time memory reading in bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MemoryOverview {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
}

/// A point-in-time swap reading in bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SwapOverview {
    pub total_bytes: u64,
    pub used_bytes: u64,
}

/// Completeness of one on-demand host-discovery collection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryAvailability {
    /// The supported collector returned every entry and field it could represent.
    Available,
    /// The collector returned useful data but had to truncate or omit some of it.
    Degraded,
    /// The current platform does not support this collector.
    Unavailable,
}

/// One locally mounted storage volume.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StorageMountOverview {
    /// Host-provided device name. `None` means it was not valid bounded host text.
    pub name: Option<String>,
    /// Host-provided filesystem name. `None` means it was not valid bounded host text.
    pub file_system: Option<String>,
    /// Host-provided mount path. `None` means it was not valid bounded host text.
    pub mount_point: Option<String>,
    #[serde(with = "u64_decimal")]
    pub total_bytes: u64,
    #[serde(with = "u64_decimal")]
    pub available_bytes: u64,
    #[serde(with = "u64_decimal")]
    pub used_bytes: u64,
    pub read_only: bool,
    pub removable: bool,
}

/// Bounded, point-in-time storage discovery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StorageOverview {
    pub availability: DiscoveryAvailability,
    pub mounts: Vec<StorageMountOverview>,
    /// Number of discovered mounts omitted because the response cap was reached.
    pub omitted_mounts: usize,
    /// Number of host-provided text fields represented as `null` instead of lossy text.
    pub omitted_text_fields: usize,
}

/// One IP address and prefix assigned to an interface.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct NetworkAddressOverview {
    pub address: String,
    pub prefix_length: u8,
}

/// One locally visible network interface.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NetworkInterfaceOverview {
    pub name: String,
    pub addresses: Vec<NetworkAddressOverview>,
    /// Cumulative byte counter reported by the host at this sample, not a transfer rate.
    #[serde(with = "u64_decimal")]
    pub total_received_bytes: u64,
    /// Cumulative byte counter reported by the host at this sample, not a transfer rate.
    #[serde(with = "u64_decimal")]
    pub total_transmitted_bytes: u64,
    #[serde(with = "u64_decimal")]
    pub mtu_bytes: u64,
}

/// Bounded, point-in-time network discovery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NetworkOverview {
    pub availability: DiscoveryAvailability,
    pub interfaces: Vec<NetworkInterfaceOverview>,
    /// Number of discovered interfaces omitted because the response cap was reached.
    pub omitted_interfaces: usize,
    /// Number of discovered addresses omitted by interface or response-wide caps.
    pub omitted_addresses: usize,
}

/// Real host information collected locally by Helix.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HostOverview {
    pub hostname: Option<String>,
    pub operating_system: Option<String>,
    pub architecture: String,
    pub kernel_version: Option<String>,
    pub uptime_seconds: u64,
    pub cpu: CpuOverview,
    pub memory: MemoryOverview,
    pub swap: SwapOverview,
    pub storage: StorageOverview,
    pub network: NetworkOverview,
    pub collected_at_unix_ms: u64,
}

/// Current wall-clock time for persisted and API metadata.
///
/// A clock before the Unix epoch is represented as zero. Helix treats an invalid
/// host clock as a health concern; it must not panic while reporting one.
#[must_use]
pub fn unix_timestamp_ms() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_enums_have_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&HealthStatus::Degraded).expect("serialize health status"),
            "\"degraded\""
        );
        assert_eq!(
            serde_json::to_string(&DatabaseStatus::Recovered).expect("serialize database status"),
            "\"recovered\""
        );
        assert_eq!(
            serde_json::to_string(&DatabaseStatus::Unavailable)
                .expect("serialize unavailable database status"),
            "\"unavailable\""
        );
    }

    #[test]
    fn discovery_availability_has_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&DiscoveryAvailability::Available)
                .expect("serialize available discovery status"),
            "\"available\""
        );
        assert_eq!(
            serde_json::to_string(&DiscoveryAvailability::Degraded)
                .expect("serialize degraded discovery status"),
            "\"degraded\""
        );
        assert_eq!(
            serde_json::to_string(&DiscoveryAvailability::Unavailable)
                .expect("serialize unavailable discovery status"),
            "\"unavailable\""
        );
    }

    #[test]
    fn storage_and_network_u64_values_use_exact_canonical_decimal_strings() {
        let overview = HostOverview {
            hostname: None,
            operating_system: None,
            architecture: "test".to_owned(),
            kernel_version: None,
            uptime_seconds: 0,
            cpu: CpuOverview {
                usage_percent: None,
                logical_cores: 1,
            },
            memory: MemoryOverview {
                total_bytes: 0,
                used_bytes: 0,
                available_bytes: 0,
            },
            swap: SwapOverview {
                total_bytes: 0,
                used_bytes: 0,
            },
            storage: StorageOverview {
                availability: DiscoveryAvailability::Available,
                mounts: vec![StorageMountOverview {
                    name: None,
                    file_system: None,
                    mount_point: None,
                    total_bytes: u64::MAX,
                    available_bytes: 0,
                    used_bytes: u64::MAX,
                    read_only: false,
                    removable: false,
                }],
                omitted_mounts: 0,
                omitted_text_fields: 0,
            },
            network: NetworkOverview {
                availability: DiscoveryAvailability::Available,
                interfaces: vec![NetworkInterfaceOverview {
                    name: "test0".to_owned(),
                    addresses: Vec::new(),
                    total_received_bytes: u64::MAX,
                    total_transmitted_bytes: 0,
                    mtu_bytes: 1_500,
                }],
                omitted_interfaces: 0,
                omitted_addresses: 0,
            },
            collected_at_unix_ms: 0,
        };

        let encoded = serde_json::to_value(&overview).expect("serialize overview");
        assert_eq!(
            encoded["storage"]["mounts"][0]["total_bytes"],
            "18446744073709551615"
        );
        assert_eq!(
            encoded["network"]["interfaces"][0]["total_received_bytes"],
            "18446744073709551615"
        );
        assert_eq!(encoded["network"]["interfaces"][0]["mtu_bytes"], "1500");
        assert_eq!(
            serde_json::from_value::<HostOverview>(encoded).expect("deserialize overview"),
            overview
        );
    }
}
