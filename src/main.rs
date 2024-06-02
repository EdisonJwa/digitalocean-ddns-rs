use clap::Parser;
use hickory_resolver::config::*;
use hickory_resolver::TokioAsyncResolver;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Response;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::result::Result;

// DigitalOcean Dynamic DNS Updater
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    // Set a custom config file
    #[arg(short, long)]
    config: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {
    token: String,
    records: HashMap<String, RecordConfig>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RecordConfig {
    domain: String,
    name: String,
    #[serde(rename = "type")]
    type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interface: Option<String>,
    #[serde(default = "default_false")]
    use_cn_dns: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct Record {
    id: u64,
    #[serde(rename = "type")]
    type_: String,
    name: String,
    data: String,
    priority: Option<u64>,
    port: Option<u64>,
    ttl: u64,
    weight: Option<u64>,
    flags: Option<u64>,
    tag: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Records {
    domain_records: Vec<Record>,
}

#[derive(Serialize, Deserialize, Debug)]
struct DomainRecord {
    domain_record: Vec<Record>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RecordUpdateBody {
    #[serde(rename = "type")]
    type_: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    weight: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    flags: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ErrResponse {
    id: String,
    message: String,
}

fn default_false() -> bool {
    false
}

async fn get_v4_ip() -> Result<String, Box<dyn Error>> {
    let res = reqwest::get("https://ipv4.whatismyip.akamai.com/")
        .await?
        .text()
        .await?;
    Ok(res)
}

#[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
async fn get_v4_ip_with_interface(interface: &Option<String>) -> Result<String, Box<dyn Error>> {
    let client = reqwest::Client::builder()
        .interface(interface.as_ref().map(|s| s.as_str()).unwrap())
        .build()
        .unwrap();
    let res = client
        .get("https://ipv4.whatismyip.akamai.com/")
        .send()
        .await?
        .text()
        .await?;

    Ok(res)
}

async fn get_v6_ip() -> Result<String, Box<dyn Error>> {
    let res = reqwest::get("https://ipv6.whatismyip.akamai.com/")
        .await?
        .text()
        .await?;
    Ok(res)
}

#[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
async fn get_v6_ip_with_interface(interface: &Option<String>) -> Result<String, Box<dyn Error>> {
    let client = reqwest::Client::builder()
        .interface(interface.as_ref().map(|s| s.as_str()).unwrap())
        .build()
        .unwrap();
    let res = client
        .get("https://ipv6.whatismyip.akamai.com/")
        .send()
        .await?
        .text()
        .await?;
    Ok(res)
}

async fn get_record_resolved_ip(
    name: &str,
    domain: &str,
    is_v4: bool,
    is_cn: bool,
) -> Result<Vec<IpAddr>, Box<dyn Error>> {
    let domain_to_resolve = format!("{}.{}", name, domain);
    let ali_ip = IpAddr::V4(Ipv4Addr::new(223, 5, 5, 5));

    let ali_dns = NameServerConfigGroup::from_ips_clear(&[ali_ip], 53, true);
    let resolver = if is_cn {
        TokioAsyncResolver::tokio(
            ResolverConfig::from_parts(None, vec![], ali_dns),
            ResolverOpts::default(),
        )
    } else {
        TokioAsyncResolver::tokio(ResolverConfig::cloudflare(), ResolverOpts::default())
    };

    if is_v4 {
        let res = resolver.ipv4_lookup(domain_to_resolve).await?;
        let ips: Vec<IpAddr> = res.iter().map(|rdata| IpAddr::V4(rdata.0)).collect();
        Ok(ips)
    } else {
        let res = resolver.ipv6_lookup(domain_to_resolve).await?;
        let ips: Vec<IpAddr> = res.iter().map(|rdata| IpAddr::V6(rdata.0)).collect();
        Ok(ips)
    }
}

async fn get_record_id(
    token: &str,
    name: &str,
    domain: &str,
    type_: &str,
) -> Result<u64, Box<dyn Error>> {
    let res = get_records(token, domain, name, type_).await;
    if !res.status().is_success() {
        let error_data = res.json::<ErrResponse>().await.unwrap();
        panic!("{}", format!("{}: {}", error_data.id, error_data.message));
    }

    let records = res.json::<Records>().await.unwrap();
    for record in records.domain_records {
        if record.name == name {
            return Ok(record.id);
        }
    }
    Err("Record not found".into())
}

async fn get_records(token: &str, domain: &str, name: &str, type_: &str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&("Bearer ".to_owned() + token)).unwrap(),
    );
    reqwest::Client::new()
        .get(&format!(
            "https://api.digitalocean.com/v2/domains/{}/records/?name={}.{}&type={}&per_page=200",
            domain, name, domain, type_
        ))
        .headers(headers)
        .send()
        .await
        .unwrap()
}

async fn update_record(id: u64, domain: &str, data: RecordUpdateBody, token: &str) -> Response {
    let url = format!(
        "https://api.digitalocean.com/v2/domains/{}/records/{}",
        domain, id
    );
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&("Bearer ".to_owned() + token)).unwrap(),
    );
    reqwest::Client::new()
        .patch(&url)
        .headers(headers)
        .json(&data)
        .send()
        .await
        .unwrap()
}

async fn check_ip(
    name: &str,
    domain: &str,
    is_v4: bool,
    is_cn: bool,
) -> Result<bool, Box<dyn Error>> {
    let record_ips = get_record_resolved_ip(name, domain, is_v4, is_cn).await?;
    let current_ip = if is_v4 {
        get_v4_ip().await?
    } else {
        get_v6_ip().await?
    };
    println!("Current IP: {}", current_ip);
    for ip in &record_ips {
        println!("Record IP: {}", ip);
    }
    for ip in record_ips {
        if ip.to_string() == current_ip {
            return Ok(true);
        }
    }
    Ok(false)
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // if args.config.is_none() {
    //     panic!("Config file not provided");
    // }
    let config_file = args.config.unwrap_or("config.toml".to_string());
    if !Path::new(&config_file).exists() {
        panic!("Config file not found");
    }

    let config: Config = toml::from_str(&std::fs::read_to_string(config_file).unwrap()).unwrap();
    for (name, record) in config.records.iter() {
        println!("Updating record: {}", name);
        let record_id = get_record_id(&config.token, &record.name, &record.domain, &record.type_)
            .await
            .unwrap();
        if record.type_ == "A" {
            let mut v4_ip: Option<String> = None;
            if let Some(ref _interface) = &record.interface {
                if cfg!(any(
                    target_os = "android",
                    target_os = "fuchsia",
                    target_os = "linux",
                )) {
                    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
                    {
                        v4_ip = Some(get_v4_ip_with_interface(&record.interface).await.unwrap());
                    }
                } else {
                    v4_ip = Some(get_v4_ip().await.unwrap());
                }
            } else {
                v4_ip = Some(get_v4_ip().await.unwrap());
            }

            let data = RecordUpdateBody {
                type_: record.type_.clone(),
                name: record.name.clone(),
                data: v4_ip,
                priority: None,
                port: None,
                ttl: Some(record.ttl.unwrap_or(60)),
                weight: None,
                flags: None,
                tag: None,
            };
            let is_same_ip = check_ip(&record.name, &record.domain, true, record.use_cn_dns)
                .await
                .unwrap();
            if !is_same_ip {
                let res = update_record(record_id, &record.domain, data, &config.token).await;
                if res.status().is_success() {
                    println!("{:?}", res.text().await.unwrap());
                    println!("Record updated");
                } else {
                    panic!("Failed to update record\n{:?}", res.text().await.unwrap());
                }
            } else {
                println!("IP is the same, skipping");
            }
        } else if record.type_ == "AAAA" {
            let mut v6_ip: Option<String> = None;
            if let Some(ref _interface) = &record.interface {
                if cfg!(any(
                    target_os = "android",
                    target_os = "fuchsia",
                    target_os = "linux",
                )) {
                    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
                    {
                        v6_ip = Some(get_v6_ip_with_interface(&record.interface).await.unwrap());
                    }
                } else {
                    println!("SO_BINDTODEVICE is not supported on this platform");
                    v6_ip = Some(get_v6_ip().await.unwrap());
                }
            } else {
                v6_ip = Some(get_v6_ip().await.unwrap());
            }
            let data = RecordUpdateBody {
                type_: record.type_.clone(),
                name: record.name.clone(),
                data: v6_ip,
                priority: None,
                port: None,
                ttl: Some(record.ttl.unwrap_or(60)),
                weight: None,
                flags: None,
                tag: None,
            };
            let is_same_ip = check_ip(&record.name, &record.domain, false, record.use_cn_dns)
                .await
                .unwrap();
            if !is_same_ip {
                let res = update_record(record_id, &record.domain, data, &config.token).await;
                if res.status().is_success() {
                    println!("Record updated");
                } else {
                    panic!("Failed to update record\n{:?}", res.text().await.unwrap());
                }
            } else {
                println!("IP is the same, skipping");
            }
        } else {
            panic!("Invalid record type");
        }
    }
}
