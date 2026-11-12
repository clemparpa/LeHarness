#![no_main]

use libfuzzer_sys::fuzz_target;

// Fail-closed contract: any input is either rejected (Malformed) or parsed
// into a complete ToolResponse that survives a serialize/parse round-trip.
fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(resp) = harness::process::parse_line(s) {
        let line = serde_json::to_string(&resp).unwrap();
        let reparsed = harness::process::parse_line(&line).unwrap();
        assert_eq!(resp, reparsed, "round-trip mismatch for input {s:?}");
    }
});
