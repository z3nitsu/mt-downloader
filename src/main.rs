use clap::Parser;
use tokio::{fs, io::AsyncWriteExt};
use tokio::sync::Semaphore;
use url::Url;

#[derive(Parser, Debug)]
#[command(author, version, about = "Beginner async multi-file downloader")]
struct Cli {
    urls: Vec<String>,
    #[arg(short, long, default_value = ".")] out: String,
    #[arg(short = 'c', long, default_value_t = 4)] concurrency: usize,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    if cli.urls.is_empty() { eprintln!("No URLs provided"); std::process::exit(2); }

    let client = reqwest::Client::new();
    
    // Download only the first URL for now
    let sem = std::sync::Arc::new(Semaphore::new(cli.concurrency));
let mut handles = Vec::new();

for raw in cli.urls.iter() {
    let permit = sem.clone().acquire_owned().await.unwrap();
    let client = client.clone();
    let out = cli.out.clone();
    let raw = raw.clone();

    let h = tokio::spawn(async move {
        let _p = permit; // holds a slot until this task finishes
        let url = Url::parse(&raw).unwrap();
        let fname = file_name_from_url(&url);
        let path = std::path::Path::new(&out).join(fname);
        if let Err(e) = download_once(&client, &url, &path).await {
            eprintln!("FAILED {}: {}", url, e);
        }
    });
    handles.push(h);
}

for h in handles { let _ = h.await; }
    Ok(())
}

fn file_name_from_url(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut segs| segs.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("download")
        .to_string()
}

async fn download_once(
    client: &reqwest::Client,
    url: &Url,
    path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let resp = client.get(url.clone()).send().await?;
    if !resp.status().is_success() { return Err("non-success status".into()); }

    let mut file = fs::File::create(path).await?;
    let bytes = resp.bytes().await?;
    file.write_all(&bytes).await?;
    file.flush().await?;
    Ok(())
}
