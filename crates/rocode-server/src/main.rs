use clap::Parser;

#[derive(Parser, Debug)]
struct ServerCli {
    #[arg(long, default_value_t = 0)]
    port: u16,
    #[arg(long, default_value = "127.0.0.1")]
    hostname: String,
    #[arg(long)]
    cwd: Option<std::path::PathBuf>,
    #[arg(long, default_value_t = false)]
    mdns: bool,
    #[arg(long = "mdns-domain", default_value = "rocode.local")]
    mdns_domain: String,
    #[arg(long)]
    cors: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = ServerCli::parse();
    rocode_server::run_server_runtime(rocode_server::ServerRuntimeOptions {
        port: cli.port,
        hostname: cli.hostname,
        cwd: cli.cwd,
        web_dist: None,
        embedded_web_assets: None,
        mdns: cli.mdns,
        mdns_domain: cli.mdns_domain,
        cors: cli.cors,
    })
    .await
}
