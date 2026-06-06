use agendao_client::transport::TransportSelector;
use clap::Parser;

pub mod cli;
mod server_lifecycle;

pub use server_lifecycle::{CliRuntimeContext, ServerDiscoveryRequest};

pub fn parse_cli_from<I, T>(args: I) -> cli::Cli
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    cli::Cli::parse_from(args)
}

pub fn resolve_socket_path(enabled: bool) -> anyhow::Result<Option<String>> {
    if !enabled {
        return Ok(None);
    }
    TransportSelector::default_unix_socket_path()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("--socket is not supported on this platform"))
}
