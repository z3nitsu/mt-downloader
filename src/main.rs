use anyhow::{anyhow, Context, Result};
use clap::Parser;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use tokio::{fs, io::AsyncWriteExt, sync::Semaphore};
use url::Url;

#[derive(Parser, Debug)]
#[command(author, version, about = "Beginner async multi-file downloader")]
struct Cli {
    /// One or more URLs to download
    urls: Vec<String>,

    /// Output directory
    #[arg(short, long, default_value = ".")]
    out: String,

    /// Max concurrent downloads
    #[arg(short = 'c', long, default_value_t = 4)]
    concurrency: usize,

    /// Number of retry attempts per file
    #[arg(short = 'r', long, default_value_t = 3)]
    retries: u32,

    /// Base backoff in milliseconds (exponential: base * 2^(attempt-1))
    #[arg(long, default_value_t = 500)]
    backoff_ms: u64,

    /// Overwrite existing files instead of adding (1), (2), ...
    #[arg(long, default_value_t = false)]
    overwrite: bool,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.urls.is_empty() {
        eprintln!("No URLs provided");
        std::process::exit(2);
    }

    // Ensure output dir exists
    fs::create_dir_all(&cli.out)
        .await
        .with_context(|| format!("create output dir {}", cli.out))?;

    let client = reqwest::Client::new();

    // Copy simple values out of `cli` so we don't capture it inside tasks
    let retries = cli.retries;
    let backoff_ms = cli.backoff_ms;
    let out_dir = cli.out.clone();
    let overwrite = cli.overwrite;

    // Bounded parallelism
    let sem = std::sync::Arc::new(Semaphore::new(cli.concurrency));
    let mut handles = Vec::new();

    for raw in cli.urls.iter() {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let out = out_dir.clone();
        let raw = raw.clone();
        let r = retries;
        let b = backoff_ms;
        let ow = overwrite;

        let h = tokio::spawn(async move {
            let _p = permit; // keep a slot until task finishes

            let url = match Url::parse(&raw) {
                Ok(u) => u,
                Err(e) => {
                    eprintln!("Invalid URL '{}': {}", raw, e);
                    return;
                }
            };

            let fname = file_name_from_url(&url);
            let path = pick_output_path(std::path::Path::new(&out), &fname, ow);

            if let Err(e) = download_with_retries(&client, &url, &path, r, b).await {
                eprintln!("FAILED {}: {e:#}", url);
            } else {
                println!("saved -> {}", path.display());
            }
        });

        handles.push(h);
    }

    for h in handles {
        let _ = h.await; // tasks already log their own errors
    }

    Ok(())
}

fn file_name_from_url(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut segs| segs.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("download")
        .to_string()
}

fn pick_output_path(out_dir: &std::path::Path, base: &str, overwrite: bool) -> std::path::PathBuf {
    use std::path::Path;
    let mut path = out_dir.join(base);
    if overwrite || !path.exists() {
        return path;
    }

    let stem = Path::new(base)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = Path::new(base)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    for i in 1..=9999 {
        let candidate = if ext.is_empty() {
            out_dir.join(format!("{stem} ({i})"))
        } else {
            out_dir.join(format!("{stem} ({i}).{ext}"))
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}

async fn download_once(
    client: &reqwest::Client,
    url: &Url,
    path: &std::path::Path,
) -> Result<()> {
    let resp = client
        .get(url.clone())
        .send()
        .await
        .with_context(|| format!("request failed: {url}"))?;

    if !resp.status().is_success() {
        return Err(anyhow!("non-success status {} for {}", resp.status(), url));
    }

    // Progress bar
    let pb = ProgressBar::new(resp.content_length().unwrap_or(0));
    let prefix = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".to_string());
    pb.set_prefix(prefix);
    pb.set_style(
        ProgressStyle::with_template(
            "{prefix:.cyan.bold} [{bar:40.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec}",
        )?
        .progress_chars("##-"),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(100));

    // Stream response to file
    let mut file = fs::File::create(path)
        .await
        .with_context(|| format!("create file {}", path.display()))?;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        pb.inc(chunk.len() as u64);
    }
    file.flush().await?;
    pb.finish_with_message("done");

    Ok(())
}

async fn download_with_retries(
    client: &reqwest::Client,
    url: &Url,
    path: &std::path::Path,
    retries: u32,
    backoff_ms: u64,
) -> Result<()> {
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 1..=retries.max(1) {
        match download_once(client, url, path).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                if attempt < retries {
                    let delay = backoff_ms * (1u64 << (attempt - 1));
                    println!("retry {}/{} for {} in {}ms", attempt, retries, url, delay);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("unknown error")))
}
