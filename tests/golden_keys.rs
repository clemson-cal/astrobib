//! Golden test: cite keys must match the committed regression vectors.
//!
//! tests/golden_keys.json pins the frozen key algorithm over a real
//! library plus synthetic edge cases (accented names, old-style arXiv IDs,
//! bibcode-only records, empty author). Keys denote papers for life, so
//! the expected keys are fixtures — never edit them by hand.

use astrobib::bib::Data;

#[test]
fn keys_match_golden_vectors() {
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
