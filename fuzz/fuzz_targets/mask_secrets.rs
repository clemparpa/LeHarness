#![no_main]

use libfuzzer_sys::fuzz_target;

// Input format: first line = secret, rest = text to mask. Invariant: a known
// non-empty secret (with at least one non-'*' char, since the mask is ***)
// never appears in the masked output.
fuzz_target!(|data: &[u8]| {
    let s = String::from_utf8_lossy(data);
    let mut parts = s.splitn(2, '\n');
    let secret = parts.next().unwrap_or("");
    let text = parts.next().unwrap_or("");
    if secret.is_empty() || secret.chars().all(|c| c == '*') {
        return;
    }
    let mut store = harness::secrets::SecretStore::new();
    store.insert("k", secret, None);
    let masked = store.mask(text);
    assert!(
        !masked.contains(secret),
        "secret {secret:?} leaked in masked output {masked:?}"
    );
});
