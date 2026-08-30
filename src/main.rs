mod config;
mod do_api;
mod ip;

use anyhow::Result;
use clap::Parser;

use crate::config::{Config, RecordConfig};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the config file
    #[arg(short, long)]
    config: Option<String>,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let args = Args::parse();
    let config_path = args.config.as_deref().unwrap_or("config.toml");
    let config: Config = config::load(config_path)?;
    let do_client = do_api::DoClient::new(&config.token)?;
    let http = reqwest::Client::new();

    let mut failures = 0;
    for (key, record) in &config.records {
        println!("Updating record: {key}");
        if let Err(e) = update_record(&do_client, &http, record).await {
            eprintln!("Error updating record '{key}': {e:#}");
            failures += 1;
        }
    }

    if failures > 0 {
        anyhow::bail!("{failures} record(s) failed to update");
    }
    Ok(())
}

async fn update_record(
    do_client: &do_api::DoClient,
    http: &reqwest::Client,
    record: &RecordConfig,
) -> Result<()> {
    let is_v4 = record.type_.is_v4();

    let current_ip = ip::current_public_ip(http, is_v4, record.interface.as_deref()).await?;
    let record_ips =
        ip::resolve_record_ips(&record.name, &record.domain, is_v4, record.use_cn_dns).await?;

    for record_ip in &record_ips {
        println!("Record IP: {record_ip}");
    }
    println!("Current IP: {current_ip}");

    if record_ips.contains(&current_ip) {
        println!("IP is the same, skipping");
        return Ok(());
    }

    let ids = do_client
        .record_ids(&record.domain, &record.name, record.type_)
        .await?;
    if ids.is_empty() {
        anyhow::bail!(
            "record '{}.{} ({})' not found in DigitalOcean",
            record.name,
            record.domain,
            record.type_
        );
    }

    for id in ids {
        do_client
            .update_record(
                &record.domain,
                id,
                record.type_,
                &record.name,
                &current_ip.to_string(),
                record.ttl,
            )
            .await?;
    }
    println!("Record updated");
    Ok(())
}
