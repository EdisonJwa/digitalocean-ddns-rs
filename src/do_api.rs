use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::RecordType;

const API_BASE: &str = "https://api.digitalocean.com/v2/domains";

#[derive(Deserialize, Debug)]
struct Record {
    id: u64,
    name: String,
    #[serde(rename = "type")]
    type_: String,
}

#[derive(Deserialize, Debug)]
struct RecordsResponse {
    domain_records: Vec<Record>,
}

#[derive(Deserialize, Debug)]
struct ErrResponse {
    id: String,
    message: String,
}

#[derive(Serialize, Debug)]
struct RecordUpdateBody {
    #[serde(rename = "type")]
    type_: RecordType,
    name: String,
    data: String,
    ttl: u64,
}

pub struct DoClient {
    http: reqwest::Client,
    token: String,
}

impl DoClient {
    pub fn new(token: &str) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::new(),
            token: token.to_string(),
        })
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T> {
        let res = self
            .http
            .get(url)
            .bearer_auth(&self.token)
            .send()
            .await
            .with_context(|| format!("request to {url} failed"))?;

        let status = res.status();
        let body = res.text().await.context("failed to read response body")?;
        if !status.is_success() {
            bail!("DigitalOcean API error: {}", describe_error(status, &body));
        }

        serde_json::from_str(&body)
            .with_context(|| format!("failed to parse response from {url}: {body}"))
    }

    /// List every record ID matching the given name and type. Unlike the old
    /// single-ID lookup this handles round-robin setups where several records
    /// share the same name.
    pub async fn record_ids(
        &self,
        domain: &str,
        name: &str,
        type_: RecordType,
    ) -> Result<Vec<u64>> {
        let url =
            format!("{API_BASE}/{domain}/records/?name={name}.{domain}&type={type_}&per_page=200");
        let records: RecordsResponse = self.get_json(&url).await?;
        Ok(records
            .domain_records
            .into_iter()
            .filter(|r| r.name == name && r.type_ == type_.to_string())
            .map(|r| r.id)
            .collect())
    }

    pub async fn update_record(
        &self,
        domain: &str,
        id: u64,
        type_: RecordType,
        name: &str,
        data: &str,
        ttl: u64,
    ) -> Result<()> {
        let url = format!("{API_BASE}/{domain}/records/{id}");
        let body = RecordUpdateBody {
            type_,
            name: name.to_string(),
            data: data.to_string(),
            ttl,
        };

        let res = self
            .http
            .patch(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("request to {url} failed"))?;

        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.unwrap_or_default();
            bail!(
                "failed to update record {id}: {}",
                describe_error(status, &text)
            );
        }
        Ok(())
    }
}

fn describe_error(status: reqwest::StatusCode, body: &str) -> String {
    match serde_json::from_str::<ErrResponse>(body) {
        Ok(err) => format!("{} ({}, {})", err.message, err.id, status.as_u16()),
        Err(_) => format!("HTTP {}: {}", status.as_u16(), body),
    }
}
