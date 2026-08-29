use quick_xml::{Reader, escape::unescape, events::Event};
use std::{
    io::{Read as _, Write as _},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    str::FromStr as _,
    time::{Duration, Instant},
};

const SSDP_TARGET: &str = "239.255.255.250:1900";
const MAX_SSDP_RESPONSES: usize = 12;
const MAX_HTTP_BODY: usize = 128 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug)]
pub struct UpnpGateway {
    host: Ipv4Addr,
    port: u16,
    control_path: String,
    service_type: String,
    pub local_ip: Ipv4Addr,
    pub external_ip: Ipv4Addr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalAddressKind {
    Public,
    CarrierGradeNat,
    PrivateOrReserved,
}

impl ExternalAddressKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::CarrierGradeNat => "carrier_grade_nat",
            Self::PrivateOrReserved => "private_or_reserved",
        }
    }
}

impl UpnpGateway {
    pub fn discover() -> Result<Self, String> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .map_err(|_| "could not open UPnP discovery".to_owned())?;
        socket
            .set_read_timeout(Some(Duration::from_millis(650)))
            .map_err(|_| "could not set the UPnP discovery timeout".to_owned())?;
        for version in ["1", "2"] {
            let request = format!(
                "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:{version}\r\n\r\n"
            );
            socket
                .send_to(request.as_bytes(), SSDP_TARGET)
                .map_err(|_| "could not send UPnP discovery".to_owned())?;
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut responses = 0_usize;
        let mut last_error = None;
        while Instant::now() < deadline && responses < MAX_SSDP_RESPONSES {
            let mut buffer = [0_u8; 2_048];
            let (size, sender) = match socket.recv_from(&mut buffer) {
                Ok(value) => value,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(_) => break,
            };
            responses = responses.saturating_add(1);
            let SocketAddr::V4(sender) = sender else {
                continue;
            };
            let response = match std::str::from_utf8(&buffer[..size]) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let Some(location) = http_header(response, "location") else {
                continue;
            };
            let parsed = match HttpTarget::parse(location) {
                Ok(parsed) if parsed.host == *sender.ip() && safe_gateway_ip(parsed.host) => parsed,
                Ok(_) => {
                    last_error =
                        Some("a router advertised an unsafe or mismatched UPnP address".to_owned());
                    continue;
                }
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            match Self::from_description(parsed) {
                Ok(gateway) => return Ok(gateway),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            "no compatible UPnP gateway answered; UPnP may be disabled or unsupported on the router"
                .to_owned()
        }))
    }

    fn from_description(target: HttpTarget) -> Result<Self, String> {
        let (body, local_ip) = http_request(&target, "GET", &[], &[])?;
        let (service_type, control_url) = find_wan_service(&body)?;
        let control = target.resolve_control(&control_url)?;
        let mut gateway = Self {
            host: control.host,
            port: control.port,
            control_path: control.path,
            service_type,
            local_ip,
            external_ip: Ipv4Addr::UNSPECIFIED,
        };
        gateway.external_ip = gateway.get_external_ip()?;
        Ok(gateway)
    }

    pub fn add_tcp_mapping(&self, port: u16, description: &str) -> Result<(), String> {
        if port < 1_024 || description.is_empty() || description.len() > 64 {
            return Err("the requested router mapping is invalid".to_owned());
        }
        let body = format!(
            "<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>TCP</NewProtocol><NewInternalPort>{port}</NewInternalPort><NewInternalClient>{}</NewInternalClient><NewEnabled>1</NewEnabled><NewPortMappingDescription>{description}</NewPortMappingDescription><NewLeaseDuration>0</NewLeaseDuration>",
            self.local_ip
        );
        self.soap("AddPortMapping", &body).map(|_| ())
    }

    pub fn tcp_mapping_description(&self, port: u16) -> Result<Option<String>, String> {
        let body = format!(
            "<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>TCP</NewProtocol>"
        );
        match self.soap("GetSpecificPortMappingEntry", &body) {
            Ok(response) => Ok(Some(
                xml_text(&response, "NewPortMappingDescription").unwrap_or_default(),
            )),
            Err(error) if missing_mapping_error(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn verify_tcp_mapping(&self, port: u16, description: &str) -> Result<bool, String> {
        let body = format!(
            "<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>TCP</NewProtocol>"
        );
        let response = self.soap("GetSpecificPortMappingEntry", &body)?;
        Ok(
            xml_text(&response, "NewInternalPort").as_deref() == Some(&port.to_string())
                && xml_text(&response, "NewInternalClient").as_deref()
                    == Some(&self.local_ip.to_string())
                && matches!(
                    xml_text(&response, "NewEnabled").as_deref(),
                    Some("1" | "true")
                )
                && xml_text(&response, "NewPortMappingDescription").as_deref() == Some(description),
        )
    }

    pub fn delete_tcp_mapping(&self, port: u16) -> Result<(), String> {
        let body = format!(
            "<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>TCP</NewProtocol>"
        );
        self.soap("DeletePortMapping", &body).map(|_| ())
    }

    fn get_external_ip(&self) -> Result<Ipv4Addr, String> {
        let response = self.soap("GetExternalIPAddress", "")?;
        let value = xml_text(&response, "NewExternalIPAddress")
            .ok_or_else(|| "the router omitted its external IPv4 address".to_owned())?;
        Ipv4Addr::from_str(value.trim())
            .map_err(|_| "the router returned an invalid external IPv4 address".to_owned())
    }

    fn soap(&self, action: &str, inner: &str) -> Result<String, String> {
        let envelope = format!(
            "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">{inner}</u:{action}></s:Body></s:Envelope>",
            self.service_type
        );
        let target = HttpTarget {
            host: self.host,
            port: self.port,
            path: self.control_path.clone(),
        };
        let action_header = format!("\"{}#{action}\"", self.service_type);
        let headers = [
            ("Content-Type", "text/xml; charset=\"utf-8\""),
            ("SOAPAction", action_header.as_str()),
        ];
        let (response, _) = http_request(&target, "POST", &headers, envelope.as_bytes())?;
        if let Some(code) = xml_text(&response, "errorCode") {
            let detail = xml_text(&response, "errorDescription")
                .unwrap_or_else(|| "router rejected the request".to_owned());
            return Err(format!("router UPnP error {code}: {detail}"));
        }
        Ok(response)
    }
}

pub fn classify_external_address(address: Ipv4Addr) -> ExternalAddressKind {
    let octets = address.octets();
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return ExternalAddressKind::CarrierGradeNat;
    }
    if address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_unspecified()
        || octets[0] == 0
        || octets[0] >= 224
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 192 && octets[1] == 88 && octets[2] == 99)
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
    {
        ExternalAddressKind::PrivateOrReserved
    } else {
        ExternalAddressKind::Public
    }
}

#[derive(Clone, Debug)]
struct HttpTarget {
    host: Ipv4Addr,
    port: u16,
    path: String,
}

impl HttpTarget {
    fn parse(value: &str) -> Result<Self, String> {
        let rest = value
            .trim()
            .strip_prefix("http://")
            .ok_or_else(|| "the router advertised a non-HTTP UPnP address".to_owned())?;
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.contains(['@', '[', ']']) || rest.contains(['#', '?']) {
            return Err("the router advertised an unsupported UPnP address".to_owned());
        }
        let (host, port) = authority
            .rsplit_once(':')
            .map_or((authority, Ok(80_u16)), |(host, port)| {
                (host, port.parse::<u16>())
            });
        let host = Ipv4Addr::from_str(host)
            .map_err(|_| "the router UPnP address was not a literal IPv4 address".to_owned())?;
        let port = port.map_err(|_| "the router UPnP port was invalid".to_owned())?;
        if port == 0 {
            return Err("the router UPnP port was invalid".to_owned());
        }
        let path = format!("/{path}");
        if path.len() > 2_048 || path.chars().any(char::is_control) {
            return Err("the router UPnP path was invalid".to_owned());
        }
        Ok(Self { host, port, path })
    }

    fn resolve_control(&self, value: &str) -> Result<Self, String> {
        if value.starts_with("http://") {
            let target = Self::parse(value)?;
            if target.host != self.host || target.port != self.port {
                return Err("the router advertised a cross-origin UPnP control URL".to_owned());
            }
            return Ok(target);
        }
        if value.contains(['#', '?', '\\']) || value.chars().any(char::is_control) {
            return Err("the router UPnP control path was invalid".to_owned());
        }
        let path = if value.starts_with('/') {
            value.to_owned()
        } else {
            let base = self.path.rsplit_once('/').map_or("/", |(base, _)| base);
            format!("{base}/{value}")
        };
        if path.len() > 2_048 || path.split('/').any(|part| part == "..") {
            return Err("the router UPnP control path was invalid".to_owned());
        }
        Ok(Self {
            host: self.host,
            port: self.port,
            path,
        })
    }
}

fn safe_gateway_ip(address: Ipv4Addr) -> bool {
    address.is_private() || address.is_link_local()
}

fn missing_mapping_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("error 713")
        || normalized.contains("error 714")
        || normalized.contains("nosuchentry")
        || normalized.contains("specifiedarrayindexinvalid")
}

fn http_request(
    target: &HttpTarget,
    method: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<(String, Ipv4Addr), String> {
    let address = SocketAddr::new(IpAddr::V4(target.host), target.port);
    let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT)
        .map_err(|_| "could not connect to the router UPnP service".to_owned())?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok();
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok();
    let local_ip = match stream.local_addr().map(|value| value.ip()) {
        Ok(IpAddr::V4(value)) => value,
        _ => {
            return Err(
                "could not determine the private IPv4 address used for the router".to_owned(),
            );
        }
    };
    let mut request = format!(
        "{method} {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nContent-Length: {}\r\n",
        target.path,
        target.host,
        target.port,
        body.len()
    );
    for (name, value) in headers {
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.contains(['\r', '\n'])
        {
            return Err("the UPnP request headers were invalid".to_owned());
        }
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.write_all(body))
        .and_then(|()| stream.flush())
        .map_err(|_| "could not send the router UPnP request".to_owned())?;
    let mut response = Vec::new();
    stream
        .take((MAX_HTTP_BODY + 32 * 1024 + 1) as u64)
        .read_to_end(&mut response)
        .map_err(|_| "the router UPnP response timed out".to_owned())?;
    if response.len() > MAX_HTTP_BODY + 32 * 1024 {
        return Err("the router UPnP response exceeded the safety limit".to_owned());
    }
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "the router returned an invalid HTTP response".to_owned())?;
    let header = std::str::from_utf8(&response[..header_end])
        .map_err(|_| "the router returned invalid HTTP headers".to_owned())?;
    let status = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "the router returned an invalid HTTP status".to_owned())?;
    let raw_body = &response[header_end + 4..];
    let decoded = if http_header(header, "transfer-encoding")
        .is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
    {
        decode_chunked(raw_body)?
    } else if let Some(length) = http_header(header, "content-length") {
        let length = length
            .parse::<usize>()
            .map_err(|_| "the router returned an invalid content length".to_owned())?;
        if length > MAX_HTTP_BODY || raw_body.len() < length {
            return Err("the router returned an incomplete or oversized response".to_owned());
        }
        raw_body[..length].to_vec()
    } else {
        raw_body.to_vec()
    };
    if decoded.len() > MAX_HTTP_BODY {
        return Err("the router UPnP response exceeded the safety limit".to_owned());
    }
    let body = String::from_utf8(decoded)
        .map_err(|_| "the router returned invalid UPnP text".to_owned())?;
    if !(200..300).contains(&status) && xml_text(&body, "errorCode").is_none() {
        return Err(format!("the router UPnP service returned HTTP {status}"));
    }
    Ok((body, local_ip))
}

fn decode_chunked(mut input: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    loop {
        let line_end = input
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| "the router returned invalid chunked data".to_owned())?;
        let size_text = std::str::from_utf8(&input[..line_end])
            .map_err(|_| "the router returned invalid chunked data".to_owned())?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or(""), 16)
            .map_err(|_| "the router returned invalid chunked data".to_owned())?;
        input = &input[line_end + 2..];
        if size == 0 {
            return Ok(output);
        }
        if size > MAX_HTTP_BODY.saturating_sub(output.len()) || input.len() < size + 2 {
            return Err("the router returned incomplete or oversized chunked data".to_owned());
        }
        output.extend_from_slice(&input[..size]);
        if &input[size..size + 2] != b"\r\n" {
            return Err("the router returned invalid chunk framing".to_owned());
        }
        input = &input[size + 2..];
    }
}

fn http_header<'a>(input: &'a str, wanted: &str) -> Option<&'a str> {
    input.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(wanted)
            .then(|| value.trim())
    })
}

fn find_wan_service(xml: &str) -> Result<(String, String), String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut in_service = false;
    let mut current = None::<String>;
    let mut service_type = None::<String>;
    let mut control_url = None::<String>;
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let name = event.local_name().into_inner().to_owned();
                if name == "service" {
                    in_service = true;
                    service_type = None;
                    control_url = None;
                } else if in_service && matches!(name.as_str(), "serviceType" | "controlURL") {
                    current = Some(name);
                }
            }
            Ok(Event::Text(text)) if in_service => {
                let value = unescape(text.as_ref())
                    .map_err(|_| "the router returned invalid UPnP XML".to_owned())?
                    .trim()
                    .to_owned();
                match current.as_deref() {
                    Some("serviceType") => service_type = Some(value),
                    Some("controlURL") => control_url = Some(value),
                    _ => {}
                }
            }
            Ok(Event::End(event)) => {
                let name = event.local_name().into_inner().to_owned();
                if name == "service" && in_service {
                    if let (Some(service), Some(control)) = (&service_type, &control_url)
                        && matches!(
                            service.as_str(),
                            "urn:schemas-upnp-org:service:WANIPConnection:2"
                                | "urn:schemas-upnp-org:service:WANIPConnection:1"
                                | "urn:schemas-upnp-org:service:WANPPPConnection:1"
                        )
                    {
                        return Ok((service.clone(), control.clone()));
                    }
                    in_service = false;
                }
                current = None;
            }
            Ok(Event::Eof) => break,
            Ok(Event::DocType(_)) => {
                return Err(
                    "the router UPnP description used a disallowed document type".to_owned(),
                );
            }
            Ok(_) => {}
            Err(_) => return Err("the router returned invalid UPnP XML".to_owned()),
        }
    }
    Err("the router does not advertise a compatible WAN port-mapping service".to_owned())
}

fn xml_text(xml: &str, wanted: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut matching = false;
    loop {
        match reader.read_event().ok()? {
            Event::Start(event) => {
                matching = event.local_name().into_inner() == wanted;
            }
            Event::Text(text) if matching => {
                return unescape(text.as_ref()).ok().map(|value| value.into_owned());
            }
            Event::End(_) => matching = false,
            Event::Eof => return None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_classification_does_not_call_cgnat_public() {
        assert_eq!(
            classify_external_address(Ipv4Addr::new(100, 64, 1, 2)),
            ExternalAddressKind::CarrierGradeNat
        );
        assert_eq!(
            classify_external_address(Ipv4Addr::new(192, 168, 1, 2)),
            ExternalAddressKind::PrivateOrReserved
        );
        assert_eq!(
            classify_external_address(Ipv4Addr::new(8, 8, 8, 8)),
            ExternalAddressKind::Public
        );
    }

    #[test]
    fn discovery_url_is_literal_http_ipv4_only() {
        assert!(HttpTarget::parse("https://192.168.1.1/root.xml").is_err());
        assert!(HttpTarget::parse("http://router.local/root.xml").is_err());
        assert!(HttpTarget::parse("http://user@192.168.1.1/root.xml").is_err());
        let target = HttpTarget::parse("http://192.168.1.1:5000/root.xml").unwrap();
        assert_eq!(target.host, Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(target.port, 5000);
        assert_eq!(target.path, "/root.xml");
    }

    #[test]
    fn parses_wan_service_and_namespaced_soap_values() {
        let xml = r#"<root><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>/ctl/IPConn</controlURL></service></root>"#;
        assert_eq!(
            find_wan_service(xml).unwrap(),
            (
                "urn:schemas-upnp-org:service:WANIPConnection:1".to_owned(),
                "/ctl/IPConn".to_owned()
            )
        );
        assert_eq!(
            xml_text(
                "<u:Response><NewExternalIPAddress>1.2.3.4</NewExternalIPAddress></u:Response>",
                "NewExternalIPAddress"
            )
            .as_deref(),
            Some("1.2.3.4")
        );
        assert_eq!(
            xml_text(
                "<s:Fault><detail><u:UPnPError><u:errorCode>714</u:errorCode></u:UPnPError></detail></s:Fault>",
                "errorCode"
            )
            .as_deref(),
            Some("714")
        );
        assert!(missing_mapping_error(
            "router UPnP error 713: SpecifiedArrayIndexInvalid"
        ));
        assert!(missing_mapping_error(
            "router UPnP error 714: NoSuchEntryInArray"
        ));
    }

    #[test]
    fn chunk_decoder_is_bounded_and_strict() {
        assert_eq!(decode_chunked(b"4\r\ntest\r\n0\r\n\r\n").unwrap(), b"test");
        assert!(decode_chunked(b"4\r\nno").is_err());
    }
}
