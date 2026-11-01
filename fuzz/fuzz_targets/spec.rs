//! The security property behind generated configs: whatever spec passes
//! validation renders to a config that parses, has exactly the server blocks
//! ngx meant to write, and embeds the same spec back.
#![no_main]
use libfuzzer_sys::fuzz_target;
use ngx::nginx::template::{self, Spec};

fuzz_target!(|data: &[u8]| {
    let Ok(spec) = serde_json::from_slice::<Spec>(data) else { return };
    if ngx::validate::spec(&spec).is_err() {
        return;
    }
    let src = template::render(&spec, false);
    let dirs = ngx::nginx::parser::parse(&src).expect("a validated spec must render to a parseable config");
    // no value may have injected a directive at the top level
    for d in &dirs {
        assert!(matches!(d.name.as_str(), "server" | "limit_req_zone"), "unexpected top-level directive {:?}", d.name);
    }
    let servers = dirs.iter().filter(|d| d.name == "server").count();
    let expected = if spec.cert_paths().is_some() && spec.inbound != 80 { 2 } else { 1 };
    assert_eq!(servers, expected, "unexpected number of server blocks");
    assert_eq!(Spec::from_config(&src).as_ref(), Some(&spec), "the embedded spec must round-trip");
});
