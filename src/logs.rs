//! Reading nginx error / access logs.

use crate::env::Env;
use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDateTime};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

fn read_tail(path: &Path, max: u64) -> Option<String> {
    let mut f = std::fs::File::open(Env::get().host(path)).ok()?;
    let len = f.metadata().ok()?.len();
    if len > max {
        f.seek(SeekFrom::Start(len - max)).ok()?;
    }
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let mut s = String::from_utf8_lossy(&buf).into_owned();
    if len > max
        && let Some(i) = s.find('\n') {
            s.drain(..=i);
        }
    Some(s)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ErrEntry {
    pub time: NaiveDateTime,
    pub code: Option<u16>,
    pub msg: String,
}

/// Parse one error_log line:
/// `2026/10/31 20:11:03 [error] 123#123: *5 connect() failed (111: ...) while connecting to upstream, client: ...`
pub fn parse_error_line(line: &str, domain: Option<&str>) -> Option<ErrEntry> {
    let time = NaiveDateTime::parse_from_str(line.get(..19)?, "%Y/%m/%d %H:%M:%S").ok()?;
    let rest = line.get(20..)?;
    let (level, rest) = rest.strip_prefix('[')?.split_once("] ")?;
    if !matches!(level, "error" | "crit" | "alert" | "emerg") {
        return None;
    }
    if let Some(d) = domain {
        let tagged = rest.contains(&format!("server: {d},")) || rest.contains(&format!("host: \"{d}\""));
        if !tagged {
            return None;
        }
    }
    // strip "pid#tid: *conn "
    let rest = rest.split_once(": ").map(|(_, r)| r).unwrap_or(rest);
    let rest = if rest.starts_with('*') { rest.split_once(' ').map(|(_, r)| r).unwrap_or(rest) } else { rest };
    let mut msg = rest.split(", client: ").next().unwrap_or(rest).to_string();
    if let Some(i) = msg.find(" while connecting to ") {
        msg = format!("{} to {}", &msg[..i], &msg[i + 21..]);
    } else if let Some(i) = msg.find(" while ") {
        msg.truncate(i);
    }
    let code = if msg.contains("upstream timed out") {
        Some(504)
    } else if msg.contains("upstream") || msg.contains("connect() failed") || msg.contains("connect() to unix") {
        Some(502)
    } else if msg.contains("No such file or directory") || msg.contains("is not found") {
        Some(404)
    } else if msg.contains("Permission denied") || msg.contains("forbidden") {
        Some(403)
    } else {
        None
    };
    Some(ErrEntry { time, code, msg })
}

/// Most recent `n` errors (newest first). `domain` filters shared log files.
pub fn recent_errors(path: &Path, domain: Option<&str>, n: usize) -> Vec<ErrEntry> {
    let Some(s) = read_tail(path, 2 << 20) else { return vec![] };
    s.lines().rev().filter_map(|l| parse_error_line(l, domain)).take(n).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Traffic {
    pub requests: u64,
    pub errors_5xx: u64,
    pub avg_ms: Option<f64>,
}

/// `(time, status, request_time)` from a combined-format line (+ optional `rt=`).
pub fn parse_access_line(line: &str) -> Option<(DateTime<FixedOffset>, u16, Option<f64>)> {
    let t0 = line.find('[')? + 1;
    let t1 = t0 + line[t0..].find(']')?;
    let time = DateTime::parse_from_str(&line[t0..t1], "%d/%b/%Y:%H:%M:%S %z").ok()?;
    let after = &line[t1..];
    let q0 = after.find('"')? + 1;
    let q1 = q0 + after[q0..].find('"')?;
    let status = after[q1 + 1..].split_whitespace().next()?.parse().ok()?;
    let rt = line
        .rsplit(' ')
        .find_map(|f| f.strip_prefix("rt="))
        .and_then(|v| v.trim_matches('"').parse().ok());
    Some((time, status, rt))
}

pub fn traffic_24h(path: &Path) -> Option<Traffic> {
    let since = Local::now() - Duration::hours(24);
    let mut text = String::new();
    let rotated = path.with_file_name(format!("{}.1", path.file_name()?.to_string_lossy()));
    if let Some(s) = read_tail(&rotated, 32 << 20) {
        text.push_str(&s);
    }
    let cur = read_tail(path, 64 << 20)?;
    text.push_str(&cur);
    let (mut req, mut e5, mut rt_sum, mut rt_n) = (0u64, 0u64, 0f64, 0u64);
    for l in text.lines() {
        let Some((t, status, rt)) = parse_access_line(l) else { continue };
        if t < since {
            continue;
        }
        req += 1;
        if status >= 500 {
            e5 += 1;
        }
        if let Some(r) = rt {
            rt_sum += r;
            rt_n += 1;
        }
    }
    Some(Traffic { requests: req, errors_5xx: e5, avg_ms: (rt_n > 0).then(|| rt_sum / rt_n as f64 * 1000.0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_lines() {
        let l = "2026/10/31 20:11:03 [error] 811#811: *1532 connect() failed (111: Connection refused) while connecting to upstream, client: 1.2.3.4, server: staging.example.com, request: \"GET / HTTP/1.1\", upstream: \"http://127.0.0.1:4000/\", host: \"staging.example.com\"";
        let e = parse_error_line(l, None).unwrap();
        assert_eq!(e.msg, "connect() failed (111: Connection refused) to upstream");
        assert_eq!(e.code, Some(502));
        assert!(parse_error_line(l, Some("staging.example.com")).is_some());
        assert!(parse_error_line(l, Some("blog.example.com")).is_none());
        let l2 = "2026/10/31 18:47:00 [error] 811#811: *99 upstream prematurely closed connection while reading response header from upstream, client: 1.2.3.4";
        assert_eq!(parse_error_line(l2, None).unwrap().msg, "upstream prematurely closed connection");
        assert!(parse_error_line("2026/10/31 18:47:00 [notice] 1#1: signal process started", None).is_none());
    }

    #[test]
    fn access_lines() {
        let l = r#"1.2.3.4 - - [31/Oct/2026:20:11:03 +0100] "GET /a b HTTP/1.1" 502 157 "-" "curl/8.0" rt=0.084"#;
        let (_, s, rt) = parse_access_line(l).unwrap();
        assert_eq!(s, 502);
        assert_eq!(rt, Some(0.084));
        let l = r#"1.2.3.4 - - [31/Oct/2026:20:11:03 +0100] "GET / HTTP/1.1" 200 15 "-" "Mozilla""#;
        assert_eq!(parse_access_line(l).unwrap().2, None);
    }
}
