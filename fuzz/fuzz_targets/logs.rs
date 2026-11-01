//! Log lines come from the network (request lines, user agents): parsing
//! them never panics, and terminal sanitizing never lets control characters through.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data);
    for l in s.lines() {
        let _ = ngx::logs::parse_error_line(l, Some("example.com"));
        let _ = ngx::logs::parse_access_line(l);
    }
    let clean = ngx::validate::clean(&s);
    assert!(!clean.chars().any(|c| (c.is_control() && c != '\t') || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')));
});
