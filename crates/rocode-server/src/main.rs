use std::io;
use std::net::SocketAddr;
use std::process::{Child, Command as ProcessCommand, Stdio};

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

    if let Some(cwd) = cli.cwd.as_ref() {
        std::env::set_current_dir(cwd).map_err(|error| {
            anyhow::anyhow!(
                "Failed to change workspace directory to {}: {}",
                cwd.display(),
                error
            )
        })?;
    }

    if std::env::var("ROCODE_SERVER_PASSWORD")
        .or_else(|_| std::env::var("OPENCODE_SERVER_PASSWORD"))
        .is_err()
    {
        eprintln!(
            "Warning: ROCODE_SERVER_PASSWORD is not set; server is unsecured (legacy fallback: OPENCODE_SERVER_PASSWORD)."
        );
    }

    let bind_host = if cli.mdns && cli.hostname == "127.0.0.1" {
        "0.0.0.0".to_string()
    } else {
        cli.hostname
    };
    let bind_port = if cli.port == 0 { 3000 } else { cli.port };
    rocode_server::set_cors_whitelist(cli.cors);
    let _mdns_publisher =
        start_mdns_publisher_if_needed(cli.mdns, &bind_host, bind_port, &cli.mdns_domain);
    let addr: SocketAddr = format!("{}:{}", bind_host, bind_port).parse()?;
    if let Ok(cwd) = std::env::current_dir() {
        println!(
            "Starting ROCode server on {} (workspace: {})",
            addr,
            cwd.display()
        );
    } else {
        println!("Starting ROCode server on {}", addr);
    }
    rocode_server::run_server(addr).await
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn service_name_from_mdns_domain(domain: &str, port: u16) -> String {
    let trimmed = domain
        .trim()
        .trim_end_matches('.')
        .trim_end_matches(".local");
    if trimmed.is_empty() {
        format!("rocode-{}", port)
    } else {
        trimmed.to_string()
    }
}

struct MdnsPublisher {
    child: Child,
}

impl Drop for MdnsPublisher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_mdns_command(command: &str, args: &[String]) -> io::Result<MdnsPublisher> {
    let mut child = ProcessCommand::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Ok(Some(status)) = child.try_wait() {
        return Err(io::Error::other(format!(
            "mDNS publisher exited immediately with status {}",
            status
        )));
    }

    Ok(MdnsPublisher { child })
}

fn start_mdns_publisher_if_needed(
    enabled: bool,
    bind_host: &str,
    port: u16,
    mdns_domain: &str,
) -> Option<MdnsPublisher> {
    if !enabled {
        return None;
    }
    if is_loopback_host(bind_host) {
        eprintln!("Warning: mDNS enabled but hostname is loopback; skipping mDNS publish.");
        return None;
    }

    let service_name = service_name_from_mdns_domain(mdns_domain, port);
    let attempts: Vec<(String, Vec<String>)> = if cfg!(target_os = "macos") {
        vec![(
            "dns-sd".to_string(),
            vec![
                "-R".to_string(),
                service_name.clone(),
                "_http._tcp".to_string(),
                "local.".to_string(),
                port.to_string(),
                "path=/".to_string(),
            ],
        )]
    } else if cfg!(target_os = "linux") {
        vec![
            (
                "avahi-publish-service".to_string(),
                vec![
                    service_name.clone(),
                    "_http._tcp".to_string(),
                    port.to_string(),
                    "path=/".to_string(),
                ],
            ),
            (
                "avahi-publish".to_string(),
                vec![
                    "-s".to_string(),
                    service_name.clone(),
                    "_http._tcp".to_string(),
                    port.to_string(),
                    "path=/".to_string(),
                ],
            ),
        ]
    } else {
        eprintln!("Warning: mDNS requested but this platform has no configured publisher command.");
        return None;
    };

    let mut last_error: Option<String> = None;
    for (command, args) in attempts {
        match spawn_mdns_command(&command, &args) {
            Ok(publisher) => {
                eprintln!(
                    "mDNS publish enabled via `{}` as service `{}` on port {}.",
                    command, service_name, port
                );
                return Some(publisher);
            }
            Err(err) => {
                if err.kind() != io::ErrorKind::NotFound {
                    last_error = Some(format!("{}: {}", command, err));
                }
            }
        }
    }

    if let Some(err) = last_error {
        eprintln!("Warning: failed to start mDNS publisher ({})", err);
    } else {
        eprintln!("Warning: mDNS requested but no supported publisher command was found on PATH.");
    }
    None
}
