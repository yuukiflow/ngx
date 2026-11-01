//! Any input: the nginx parser, site extraction and nginx -t output parsing
//! never panic.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    if let Ok(dirs) = ngx::nginx::parser::parse(&src) {
        let _ = ngx::nginx::site::extract(&dirs);
    }
    let _ = ngx::nginx::ctl::parse_output(&src);
});
