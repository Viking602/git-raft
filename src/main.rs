#[tokio::main]
async fn main() {
    if let Err(err) = git_raft::run().await {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}
