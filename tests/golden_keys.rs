//! Golden test: cite keys must match the Python implementation exactly.
//!
//! tests/golden_keys.json is generated from astrobib/keys.py over the real
//! library plus synthetic edge cases (accented names, old-style arXiv IDs,
//! bibcode-only records, empty author). Regenerate it from the Python side
//! whenever vectors are added — never edit expected keys by hand.

use astrobib::bib::Data;

#[test]
fn keys_match_python_implementation() {
    let raw = include_str!("golden_keys.json");
    let vectors: Vec<serde_json::Value> = serde_json::from_str(raw).unwrap();
    assert!(!vectors.is_empty());
    for v in &vectors {
        let data: Data = v["data"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, val)| (k.clone(), val.as_str().unwrap_or_default().to_string()))
            .collect();
        let expected = v["expected_key"].as_str().unwrap();
        let got = astrobib::keys::generate_key(&data);
        assert_eq!(
            got, expected,
            "key mismatch for stable id fields: eprint={:?} adsurl={:?}",
            data.get("eprint"),
            data.get("adsurl"),
        );
    }
}
