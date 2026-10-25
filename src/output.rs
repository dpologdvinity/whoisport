//! Turning scan results into text and JSON.
//!
//! Formatting is separated from printing: the `format_*` and `json_rows` functions return
//! values and are unit-tested; the `print_*` functions only write them out.

use crate::docker::Container;
use crate::scan::Row;
use serde_json::{Value, json};

/// Explains a `?` owner: the process exists but this user may not look inside it.
pub const HIDDEN: &str = "owned by another user; run with sudo to see it";

/// How a port is written in text output: `3000`, or `5353/udp` for UDP.
pub fn port_label(port: u16, udp: bool) -> String {
    if udp {
        format!("{port}/udp")
    } else {
        port.to_string()
    }
}

/// A duration in seconds as a short human string: `45s`, `3m 7s`, `2h 5m`, `1d 4h`.
pub fn fmt_duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {}s", secs / 60, secs % 60),
        3600..=86399 => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
        _ => format!("{}d {}h", secs / 86400, secs % 86400 / 3600),
    }
}

/// Make text from another process safe to print.
///
/// Command lines and paths are controlled by whoever started the process, and may hold
/// terminal escape sequences that move the cursor, clear the screen, or set the clipboard
/// (OSC 52), as well as bidi override characters that visually reorder text. Control
/// characters and unsafe unicode format characters are written out as escapes (`\u{1b}`).
pub fn shown(text: &str) -> String {
    let is_unsafe = |c: char| {
        c.is_control()
            || matches!(
                c,
                '\u{061c}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{200b}'..='\u{200f}'
                    | '\u{feff}'
            )
    };
    text.chars()
        .flat_map(|c| match is_unsafe(c) {
            true => c.escape_default().collect::<Vec<_>>(),
            false => vec![c],
        })
        .collect()
}

/// Cut `text` to at most `max` characters, ending in `…` when something was removed.
pub fn truncate(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars()
        .take(max.saturating_sub(1))
        .chain(['…'])
        .collect()
}

/// `docker: name (image)`, the table's stand-in for a command line.
fn container_summary(c: &Container) -> String {
    format!("docker: {} ({})", c.name, c.image)
}

/// Build the one-line-per-listener table for list mode.
///
/// Columns are `PORT PID USER CMD`. Rows whose owner is unreadable show `?`, or the
/// Docker container when one is publishing the port. The command is truncated to 80
/// characters so every row fits a normal terminal.
pub fn format_table(rows: &[Row], udp: bool) -> String {
    // UDP labels carry a "/udp" suffix, so that column is wider.
    let width = if udp { 10 } else { 6 };
    let mut out = format!("{:<width$} {:<8} {:<10} CMD\n", "PORT", "PID", "USER");
    for row in rows {
        let port = port_label(row.port, udp);
        let docker = row.container.as_ref().map(container_summary);
        match &row.info {
            Some(i) => {
                let cmd = docker.unwrap_or_else(|| i.cmd.clone());
                out += &format!(
                    "{port:<width$} {:<8} {:<10} {}\n",
                    i.pid,
                    truncate(&shown(&i.user), 10),
                    truncate(&shown(&cmd), 80)
                );
            }
            None => {
                let pid = row.pid.map_or("?".into(), |p| p.to_string());
                let cmd = docker.unwrap_or_else(|| "?".into());
                out += &format!("{port:<width$} {:<8} {:<10} {}\n", pid, "?", shown(&cmd));
            }
        }
    }
    out
}

/// Print list mode: the table, then a note on `?` rows (to stderr) if there are any.
pub fn print_table(rows: &[Row], udp: bool) {
    print!("{}", format_table(rows, udp));
    if rows.iter().any(|r| r.info.is_none()) {
        eprintln!("ports marked ? are {HIDDEN}");
    }
}

/// Build the multi-line detail block for one listener (used for `whoisport <PORT>`).
///
/// The text ends with a newline so that printing it with `println!` leaves a blank line
/// between blocks.
pub fn format_details(row: &Row, udp: bool) -> String {
    let port = port_label(row.port, udp);
    let docker = row.container.as_ref().map_or(String::new(), |c| {
        format!("docker  {} ({})\n", shown(&c.name), shown(&c.image))
    });
    match &row.info {
        Some(i) => format!(
            "port    {port}\npid     {}\nparent  {}\nuser    {}\ncmd     {}\n{docker}cwd     {}\nuptime  {}\n",
            i.pid,
            i.ppid,
            shown(&i.user),
            shown(&i.cmd),
            shown(&i.cwd),
            fmt_duration(i.uptime_secs)
        ),
        None => {
            // The pid may still be known (the process exited between the scan and now).
            let why = match row.pid {
                Some(pid) => format!("{pid} (details unavailable; it may have just exited)"),
                None => format!("? ({HIDDEN})"),
            };
            format!("port    {port}\npid     {why}\n{docker}")
        }
    }
}

/// Print the detail block for one listener, followed by a blank line.
pub fn print_details(row: &Row, udp: bool) {
    println!("{}", format_details(row, udp));
}

/// Build the JSON array printed by `--json`: one object per listener.
///
/// Fields that are unknown (an unreadable owner, no container) are `null`, so every
/// object has the same keys: `port`, `proto`, `pid`, `ppid`, `user`, `cmd`, `cwd`,
/// `uptime_secs`, `container`, `image`.
pub fn json_rows(rows: &[Row], proto: &str) -> Value {
    let rows: Vec<Value> = rows
        .iter()
        .map(|row| {
            let info = row.info.as_ref();
            json!({
                "port": row.port,
                "proto": proto,
                "pid": row.pid,
                "ppid": info.map(|i| i.ppid),
                "user": info.map(|i| &i.user),
                "cmd": info.map(|i| &i.cmd),
                "cwd": info.map(|i| &i.cwd),
                "uptime_secs": info.map(|i| i.uptime_secs),
                "container": row.container.as_ref().map(|c| &c.name),
                "image": row.container.as_ref().map(|c| &c.image),
            })
        })
        .collect();
    Value::Array(rows)
}

/// Print `--json` output, pretty-printed.
pub fn print_json(rows: &[Row], proto: &str) {
    println!("{:#}", json_rows(rows, proto));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Info;

    /// A fully known listener.
    fn known() -> Row {
        Row {
            port: 3000,
            pid: Some(42),
            info: Some(Info {
                pid: 42,
                ppid: 1,
                user: "kaitlyn".into(),
                cmd: "python3 -m http.server 3000".into(),
                cwd: "/srv".into(),
                uptime_secs: 125,
                started: 9,
            }),
            container: None,
        }
    }

    /// A listener whose owner cannot be seen, optionally published by a container.
    fn hidden(port: u16, container: Option<Container>) -> Row {
        Row {
            port,
            pid: None,
            info: None,
            container,
        }
    }

    #[test]
    fn durations_use_the_two_largest_units() {
        assert_eq!(fmt_duration(0), "0s");
        assert_eq!(fmt_duration(59), "59s");
        assert_eq!(fmt_duration(60), "1m 0s");
        assert_eq!(fmt_duration(3599), "59m 59s");
        assert_eq!(fmt_duration(3600), "1h 0m");
        assert_eq!(fmt_duration(86399), "23h 59m");
        assert_eq!(fmt_duration(86400), "1d 0h");
        assert_eq!(fmt_duration(90_000), "1d 1h");
    }

    #[test]
    fn truncate_keeps_short_text_and_marks_cut_text() {
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "ab…");
        assert_eq!(truncate("héllo wörld", 6), "héllo…"); // counts characters, not bytes
    }

    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(
            shown("ok \u{1b}]52;c;aGk=\u{7}x"),
            "ok \\u{1b}]52;c;aGk=\\u{7}x"
        );
        assert_eq!(shown("héllo wörld"), "héllo wörld");
        assert_eq!(shown("a\u{202e}b"), "a\\u{202e}b");
    }

    #[test]
    fn port_labels_mark_udp() {
        assert_eq!(port_label(53, false), "53");
        assert_eq!(port_label(53, true), "53/udp");
    }

    #[test]
    fn table_has_a_header_and_one_line_per_row() {
        let table = format_table(&[known(), hidden(22, None)], false);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines[0], "PORT   PID      USER       CMD");
        assert_eq!(
            lines[1],
            "3000   42       kaitlyn    python3 -m http.server 3000"
        );
        assert_eq!(lines[2], "22     ?        ?          ?");
    }

    #[test]
    fn table_shows_a_known_pid_even_when_its_details_are_unreadable() {
        let mut row = hidden(22, None);
        row.pid = Some(99); // the process exited between the scan and reading its details
        let table = format_table(&[row], false);
        assert!(table.contains("22     99       ?          ?"), "{table}");
    }

    #[test]
    fn bidi_and_invisible_characters_are_escaped_too() {
        // U+202E reverses the text that follows it: "exe.txt" can pose as "txt.exe".
        assert_eq!(shown("a\u{202e}b"), "a\\u{202e}b");
        assert_eq!(shown("zero\u{200b}width"), "zero\\u{200b}width");
        assert_eq!(shown("\u{feff}bom"), "\\u{feff}bom");
        assert_eq!(shown("a\u{061c}b"), "a\\u{61c}b");
        assert_eq!(shown("line\u{2028}sep"), "line\\u{2028}sep");
        assert_eq!(shown("para\u{2029}sep"), "para\\u{2029}sep");
    }

    #[test]
    fn table_names_the_container_for_a_hidden_docker_row() {
        let c = Container {
            name: "web".into(),
            image: "nginx".into(),
        };
        let table = format_table(&[hidden(8080, Some(c))], false);
        assert!(table.contains("8080   ?        ?          docker: web (nginx)"));
    }

    #[test]
    fn udp_table_widens_the_port_column() {
        let mut row = known();
        row.port = 5353;
        let table = format_table(&[row], true);
        assert!(table.starts_with("PORT       PID"));
        assert!(table.contains("5353/udp   42"));
    }

    #[test]
    fn details_show_every_field_and_the_container() {
        let mut row = known();
        row.container = Some(Container {
            name: "web".into(),
            image: "nginx".into(),
        });
        let text = format_details(&row, false);
        assert!(text.starts_with("port    3000\npid     42\nparent  1\nuser    kaitlyn\n"));
        assert!(text.contains("docker  web (nginx)\ncwd     /srv\nuptime  2m 5s\n"));
    }

    #[test]
    fn details_explain_an_unreadable_owner() {
        let text = format_details(&hidden(22, None), false);
        assert!(text.contains("pid     ? (owned by another user"));
        let mut gone = hidden(22, None);
        gone.pid = Some(99);
        assert!(format_details(&gone, false).contains("pid     99 (details unavailable"));
    }

    #[test]
    fn details_escape_hostile_command_lines() {
        let mut row = known();
        row.info.as_mut().unwrap().cmd = "evil \u{1b}[2J".into();
        assert!(!format_details(&row, false).contains('\u{1b}'));
    }

    #[test]
    fn json_objects_always_have_the_same_keys() {
        let value = json_rows(&[known(), hidden(22, None)], "tcp");
        let (full, bare) = (&value[0], &value[1]);
        assert_eq!(full["pid"], 42);
        assert_eq!(full["user"], "kaitlyn");
        assert_eq!(full["proto"], "tcp");
        assert!(full["container"].is_null());
        // The unreadable row has the same keys, with null for what is unknown.
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(full), keys(bare));
        assert!(bare["pid"].is_null() && bare["cmd"].is_null() && bare["ppid"].is_null());
    }

    #[test]
    fn json_carries_the_container_for_a_hidden_row() {
        let c = Container {
            name: "web".into(),
            image: "nginx".into(),
        };
        let value = json_rows(&[hidden(8080, Some(c))], "tcp");
        assert_eq!(value[0]["container"], "web");
        assert_eq!(value[0]["image"], "nginx");
    }
}
