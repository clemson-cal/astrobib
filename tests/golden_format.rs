//! Golden test: .bib serialization must match Python's format_bib_entry.
//!
//! Vectors are (data map, expected file text) pairs generated from the
//! Python implementation over the real library. Field order matters:
//! fields outside FIELD_ORDER serialize in insertion order, so the data
//! maps here (and bib::Data itself) preserve it — serde_json is built
//! with preserve_order for exactly this reason.

use astrobib::bib::Data;

#[test]
fn format_matches_python_implementation() {
    let raw = include_str!("golden_format.json");
    let vectors: Vec<serde_json::Value> = serde_json::from_str(raw).unwrap();
    assert!(!vectors.is_empty());
    for v in &vectors {
        let data: Data = v["data"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, val)| (k.clone(), val.as_str().unwrap_or_default().to_string()))
            .collect();
        let expected = v["formatted"].as_str().unwrap();
        let got = astrobib::bib::format_entry(&data);
        assert_eq!(got, expected, "serialization mismatch for {}", data["ID"]);
    }
}
