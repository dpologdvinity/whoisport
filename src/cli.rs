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
    /// The requested ports without repeats, in the order given.
    ///
    /// `whoisport 3000 3000` would otherwise report (and, with `--kill`, handle) the same
    /// port twice.
    pub fn unique_ports(&self) -> Vec<u16> {
        let mut seen = std::collections::HashSet::new();
        self.ports
            .iter()
            .copied()
            .filter(|p| seen.insert(*p))
            .collect()
    }

    /// `"tcp"` or `"udp"`, as shown in JSON output.
    pub fn proto(&self) -> &'static str {
        if self.udp { "udp" } else { "tcp" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a command line the way the real program would (the first item is the program name).
    fn parse(args: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("whoisport").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn unique_ports_drops_repeats_and_keeps_the_given_order() {
        assert_eq!(
            parse(&["8080", "3000", "8080", "3000", "1"]).unique_ports(),
            vec![8080, 3000, 1]
        );
        assert!(parse(&[]).unique_ports().is_empty());
    }

    #[test]
    fn proto_names_the_protocol() {
        assert_eq!(parse(&[]).proto(), "tcp");
        assert_eq!(parse(&["--udp"]).proto(), "udp");
    }

    #[test]
    fn flag_combinations_are_validated_by_clap() {
        let bad = |a: &[&str]| {
            Args::try_parse_from(std::iter::once("whoisport").chain(a.iter().copied())).is_err()
        };
        assert!(bad(&["--kill"])); // never "kill everything"
        assert!(bad(&["--force", "80"])); // --force only refines --kill
        assert!(bad(&["80", "--json", "--kill"]));
        assert!(!bad(&["80", "--kill", "--force"]));
        assert!(bad(&["70000"])); // not a port
    }
}
