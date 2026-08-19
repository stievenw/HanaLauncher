#![allow(dead_code)]
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use sha1::{Digest, Sha1};

use crate::util::hex;
use crate::worker::Reporter;

/// Download `url` to `dest`, verifying SHA1 when provided.
/// Skips the download if the destination already matches.
pub fn download_file(
    client: &Client,
    url: &str,
    dest: &Path,
    expected_sha1: Option<&str>,
    expected_size: Option<u64>,
    reporter: &Reporter,
    stage: &str,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    if let Some(sha1) = expected_sha1 {
        if dest.exists() && sha1_of_path(dest)? == sha1.to_lowercase() {
            return Ok(());
        }
    } else if dest.exists() {
        if let Some(size) = expected_size {
            if fs::metadata(dest)?.len() == size {
                return Ok(());
            }
        } else {
            return Ok(());
        }
    }

    let tmp = dest.with_extension("part");

    let mut resp = client
        .get(url)
        .send()
        .with_context(|| crate::lang::current().failed_download.replace("{}", url))?;
    if !resp.status().is_success() {
        bail!(
            crate::lang::current()
                .server_status
                .replace("{}", &resp.status().to_string())
                .replacen("{}", url, 1)
        );
    }

    let total = resp.content_length().or(expected_size).unwrap_or(0);
    let mut file = BufWriter::new(File::create(&tmp)?);
    let mut hasher = Sha1::new();
    let mut buf = [0u8; 65536];
    let mut current: u64 = 0;

    loop {
        let n = resp
            .read(&mut buf)
            .with_context(|| crate::lang::current().failed_read_response.replace("{}", url))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
        current += n as u64;
        if total > 0 && (current % (1 << 20) == 0 || current == total) {
            reporter.progress(stage, current, total);
        }
    }
    file.flush()?;

    if let Some(sha1) = expected_sha1 {
        let actual = hex(&hasher.finalize());
        if actual != sha1.to_lowercase() {
            let _ = fs::remove_file(&tmp);
            bail!(
                crate::lang::current()
                    .sha1_mismatch
                    .replace("{}", url)
                    .replacen("{}", sha1, 1)
                    .replacen("{}", &actual, 1)
            );
        }
    }

    fs::rename(&tmp, dest).with_context(|| {
        crate::lang::current().failed_save_download.replace("{}", url)
    })?;
    reporter.progress(stage, total, total);
    Ok(())
}

/// Download a small resource (JSON etc.) to memory with a sane timeout.
pub fn download_bytes(client: &Client, url: &str) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .send()
        .with_context(|| crate::lang::current().failed_download.replace("{}", url))?;
    if !resp.status().is_success() {
        bail!(
            crate::lang::current()
                .server_status
                .replace("{}", &resp.status().to_string())
                .replacen("{}", url, 1)
        );
    }
    Ok(resp
        .bytes()
        .context(crate::lang::current().failed_read_bytes)?
        .to_vec())
}

/// Download a file trying a list of candidate URLs, verifying sha1.
pub fn download_with_fallback(
    client: &Client,
    urls: &[String],
    dest: &Path,
    expected_sha1: Option<&str>,
    reporter: &Reporter,
    stage: &str,
) -> Result<()> {
    let mut last_err: Option<anyhow::Error> = None;
    for url in urls {
        match download_file(client, url, dest, expected_sha1, None, reporter, stage) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!(crate::lang::current().no_url_available)))
}

/// Fetch JSON text with a fallback list of URLs.
pub fn fetch_json_with_fallback(client: &Client, urls: &[String]) -> Result<serde_json::Value> {
    let mut last_err: Option<anyhow::Error> = None;
    for url in urls {
        match download_bytes(client, url) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(v) => return Ok(v),
                Err(e) => last_err = Some(e.into()),
            },
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!(crate::lang::current().all_urls_failed)))
}

pub fn sha1_of_path(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(data);
    hex(&hasher.finalize())
}