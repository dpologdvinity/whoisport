//! Command-line interface: the arguments `whoisport` accepts.
//!
//! The `///` doc comments on each field double as the `--help` text (clap derives it).

use clap::Parser;

/// The parsed command line.
///
/// clap enforces the relationships between flags, so the rest of the program can rely on
/// them: `--kill` always comes with at least one port (it can never mean "kill
/// everything"), `--force` always comes with `--kill`, and `--json` never comes with
/// `--kill`.
#[derive(Parser)]
#[command(version, about = "Show what is listening on a TCP or UDP port")]
pub struct Args {
    /// Port(s) to inspect; omit to list every listening port
    pub ports: Vec<u16>,
    /// Ask the listener(s) on the given port(s) to exit and wait up to 3s
    #[arg(long, requires = "ports")]
    pub kill: bool,
    /// With --kill: forcibly terminate if asking did not work
    #[arg(long, requires = "kill")]
    pub force: bool,
    /// Print results as a JSON array
    #[arg(long, conflicts_with = "kill")]
    pub json: bool,
    /// Look at UDP instead of TCP (bound sockets with no remote peer; on Windows, every bound socket)
    #[arg(long)]
    pub udp: bool,
}

impl Args {
    /// `"tcp"` or `"udp"`, as shown in JSON output.
    pub fn proto(&self) -> &'static str {
        if self.udp { "udp" } else { "tcp" }
    }
}
