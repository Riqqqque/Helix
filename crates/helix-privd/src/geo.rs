// Packed IPv4 country ranges (NRO whois / ip-location-db, CC0) plus centroids.
// Regenerate with scripts/pack-globe-data.mjs. Gaps in IANA space are not
// filled; an address must fall inside a stored inclusive range.
use serde::Deserialize;
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::OnceLock,
};

const IPV4_COUNTRY_DB: &[u8] = include_bytes!("../data/ipv4-country.bin");
const COUNTRY_META_JSON: &str = include_str!("../data/countries.json");

#[derive(Clone, Debug)]
pub struct Country {
    pub code: String,
    pub name: String,
    pub lat: f32,
    pub lon: f32,
}

#[derive(Clone, Debug, Deserialize)]
struct CountryRecord {
    code: String,
    name: String,
    lat: f32,
    lon: f32,
}

struct Ipv4CountryDb {
    starts: Vec<u32>,
    ends: Vec<u32>,
    countries: Vec<u8>,
    meta: Vec<Option<Country>>,
}

static DATABASE: OnceLock<Ipv4CountryDb> = OnceLock::new();

fn database() -> &'static Ipv4CountryDb {
    DATABASE.get_or_init(parse_database)
}

fn parse_database() -> Ipv4CountryDb {
    let records: Vec<CountryRecord> = serde_json::from_str(COUNTRY_META_JSON).unwrap_or_default();
    let by_code = records
        .into_iter()
        .map(|record| (record.code.clone(), record))
        .collect::<std::collections::HashMap<_, _>>();

    if IPV4_COUNTRY_DB.len() < 16 || &IPV4_COUNTRY_DB[0..4] != b"HELX" {
        return Ipv4CountryDb {
            starts: Vec::new(),
            ends: Vec::new(),
            countries: Vec::new(),
            meta: Vec::new(),
        };
    }
    let version = u16::from_le_bytes([IPV4_COUNTRY_DB[4], IPV4_COUNTRY_DB[5]]);
    let country_count = u16::from_le_bytes([IPV4_COUNTRY_DB[6], IPV4_COUNTRY_DB[7]]) as usize;
    let range_count = u32::from_le_bytes([
        IPV4_COUNTRY_DB[8],
        IPV4_COUNTRY_DB[9],
        IPV4_COUNTRY_DB[10],
        IPV4_COUNTRY_DB[11],
    ]) as usize;
    if version != 1 {
        return Ipv4CountryDb {
            starts: Vec::new(),
            ends: Vec::new(),
            countries: Vec::new(),
            meta: Vec::new(),
        };
    }

    let mut offset = 16_usize;
    let mut codes = Vec::with_capacity(country_count);
    for _ in 0..country_count {
        let Some(end) = IPV4_COUNTRY_DB[offset..]
            .iter()
            .position(|byte| *byte == b'\n')
        else {
            break;
        };
        let code = std::str::from_utf8(&IPV4_COUNTRY_DB[offset..offset + end]).unwrap_or("");
        codes.push(code.to_owned());
        offset += end + 1;
    }
    let mut meta = Vec::with_capacity(codes.len());
    for code in &codes {
        meta.push(by_code.get(code).map(|record| Country {
            code: record.code.clone(),
            name: record.name.clone(),
            lat: record.lat,
            lon: record.lon,
        }));
    }

    let table = &IPV4_COUNTRY_DB[offset..];
    let available = table.len() / 9;
    let count = range_count.min(available);
    let mut starts = Vec::with_capacity(count);
    let mut ends = Vec::with_capacity(count);
    let mut countries = Vec::with_capacity(count);
    for index in 0..count {
        let row = &table[index * 9..index * 9 + 9];
        starts.push(u32::from_le_bytes([row[0], row[1], row[2], row[3]]));
        ends.push(u32::from_le_bytes([row[4], row[5], row[6], row[7]]));
        countries.push(row[8]);
    }
    Ipv4CountryDb {
        starts,
        ends,
        countries,
        meta,
    }
}

pub fn lookup_ipv4(address: Ipv4Addr) -> Option<Country> {
    if !ipv4_is_globally_routable(address) {
        return None;
    }
    let db = database();
    if db.starts.is_empty() {
        return None;
    }
    let ip = u32::from(address);
    let index = match db.starts.binary_search(&ip) {
        Ok(found) => found,
        Err(insert) => insert.checked_sub(1)?,
    };
    if ip < db.starts[index] || ip > db.ends[index] {
        return None;
    }
    let country = *db.countries.get(index)?;
    db.meta.get(country as usize).cloned().flatten()
}

pub fn lookup_ip(address: IpAddr) -> Option<Country> {
    match address {
        IpAddr::V4(address) => lookup_ipv4(address),
        IpAddr::V6(address) => lookup_ipv4(ipv4_from_mapped(address)?),
    }
}

pub fn ipv4_is_globally_routable(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return false;
    }
    !(address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_unspecified()
        || address.is_broadcast()
        || octets[0] == 0
        || octets[0] >= 224
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19)))
}

fn ipv4_from_mapped(address: Ipv6Addr) -> Option<Ipv4Addr> {
    address
        .to_ipv4_mapped()
        .filter(|value| ipv4_is_globally_routable(*value))
}

#[cfg(test)]
mod tests {
    use super::{ipv4_is_globally_routable, lookup_ipv4};
    use std::net::Ipv4Addr;

    #[test]
    fn private_and_cgnat_addresses_are_not_mapped() {
        assert!(!ipv4_is_globally_routable(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(!ipv4_is_globally_routable(Ipv4Addr::new(192, 168, 1, 20)));
        assert!(!ipv4_is_globally_routable(Ipv4Addr::new(100, 64, 1, 1)));
        assert!(!ipv4_is_globally_routable(Ipv4Addr::LOCALHOST));
        assert!(ipv4_is_globally_routable(Ipv4Addr::new(1, 1, 1, 1)));
        assert!(ipv4_is_globally_routable(Ipv4Addr::new(8, 8, 8, 8)));
    }

    #[test]
    fn known_public_addresses_resolve_to_countries() {
        let australia = lookup_ipv4(Ipv4Addr::new(1, 0, 0, 1)).expect("Cloudflare 1.0.0.1");
        assert_eq!(australia.code, "AU");
        assert!(australia.lat < 0.0);
        let united_states = lookup_ipv4(Ipv4Addr::new(8, 8, 8, 8)).expect("8.8.8.8");
        assert_eq!(united_states.code, "US");
        assert!(united_states.lon < 0.0);
    }
}
