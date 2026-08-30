use anyhow::{bail, Context, Result};
use hickory_resolver::config::{NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::RData;
use hickory_resolver::Resolver;
use reqwest::Client;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

const IP_ECHO_V4: &str = "https://ipv4.whatismyip.akamai.com/";
const IP_ECHO_V6: &str = "https://ipv6.whatismyip.akamai.com/";

const CN_DNS: IpAddr = IpAddr::V4(Ipv4Addr::new(223, 5, 5, 5));

const CLOUDFLARE_DNS: &[IpAddr] = &[
    IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
    IpAddr::V4(Ipv4Addr::new(1, 0, 0, 1)),
    IpAddr::V6(Ipv6Addr::new(
        0x2606, 0x4700, 0x4700, 0, 0, 0, 0x1111, 0x1111,
    )),
    IpAddr::V6(Ipv6Addr::new(
        0x2606, 0x4700, 0x4700, 0, 0, 0, 0x1001, 0x1001,
    )),
];

#[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
fn bind_interface_client(interface: &str) -> Result<Client> {
    Client::builder()
        .interface(interface)
        .build()
        .with_context(|| format!("failed to build HTTP client bound to interface '{interface}'"))
}

/// Fetch the current public IP, optionally binding to a network interface
/// (Linux/Android/Fuchsia only). The response is parsed as an IP address so
/// whitespace or garbage from the echo service never leaks into a comparison.
pub async fn current_public_ip(
    default_client: &Client,
    is_v4: bool,
    interface: Option<&str>,
) -> Result<IpAddr> {
    let url = if is_v4 { IP_ECHO_V4 } else { IP_ECHO_V6 };
    let family = if is_v4 { "IPv4" } else { "IPv6" };

    let client = match interface {
        #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
        Some(iface) => bind_interface_client(iface)?,
        #[cfg(not(any(target_os = "android", target_os = "fuchsia", target_os = "linux")))]
        Some(iface) => {
            eprintln!(
                "Warning: interface binding is not supported on this platform, ignoring interface '{iface}'"
            );
            default_client.clone()
        }
        None => default_client.clone(),
    };

    let text = client
        .get(url)
        .send()
        .await
        .context("failed to query public IP")?
        .text()
        .await
        .context("failed to read public IP response")?;

    let ip: IpAddr = text.trim().parse().with_context(|| {
        format!("invalid {family} address returned by IP echo service: {text:?}")
    })?;

    if is_v4 != ip.is_ipv4() {
        bail!("IP echo service returned a mismatched address family: {ip}");
    }
    Ok(ip)
}

/// Resolve the IPs currently published in DNS for `name.domain`, restricted to
/// the requested address family.
pub async fn resolve_record_ips(
    name: &str,
    domain: &str,
    is_v4: bool,
    use_cn_dns: bool,
) -> Result<Vec<IpAddr>> {
    let fqdn = format!("{name}.{domain}");

    let ips: &[IpAddr] = if use_cn_dns {
        std::slice::from_ref(&CN_DNS)
    } else {
        CLOUDFLARE_DNS
    };
    let config = ResolverConfig::from_parts(
        None,
        vec![],
        ips.iter()
            .map(|&ip| NameServerConfig::udp_and_tcp(ip))
            .collect(),
    );

    let resolver = Resolver::builder_with_config(config, TokioRuntimeProvider::default())
        .build()
        .context("failed to build DNS resolver")?;

    let lookup = if is_v4 {
        resolver.ipv4_lookup(fqdn.clone()).await?
    } else {
        resolver.ipv6_lookup(fqdn.clone()).await?
    };

    let want_v4 = is_v4;
    let ips = lookup
        .answers()
        .iter()
        .filter_map(|record| match &record.data {
            RData::A(a) => want_v4.then_some(IpAddr::V4(a.0)),
            RData::AAAA(aaaa) => (!want_v4).then_some(IpAddr::V6(aaaa.0)),
            _ => None,
        })
        .collect();

    Ok(ips)
}
