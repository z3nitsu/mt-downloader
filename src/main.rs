use clap::Parser;
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
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let cli = Cli::parse();

    if cli.urls.is_empty() {
        eprintln!("No URLs provided");
        std::process::exit(2);
    }

    // Validate URLs early 
    let urls: Vec<Url> = cli
        .urls
        .iter()
        .map(|s| Url::parse(s).expect("invalid URL"))
        .collect();

    println!(
        "OK: parsed {} URL(s); out='{}'; concurrency={}",
        urls.len(),
        cli.out,
        cli.concurrency
    );
}
