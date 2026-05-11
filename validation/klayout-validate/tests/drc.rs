//! Differential test: our DRC primitives against `db.Region.{width,space,...}_check`.
//!
//! Comparison is by strict vertex equality after canonicalizing each
//! polygon to lowest-y/lowest-x first vertex (KLayout's convention).

use klayout_validate::corpus_path;

include!("drc_support.inc");

fn load() -> DrcCorpus {
    let path = corpus_path("drc.json");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "missing {}: {e}\nrun: python validation/oracle.py drc",
            path.display()
        )
    });
    serde_json::from_slice(&bytes).expect("invalid drc corpus JSON")
}

#[test]
fn drc_rules_match_klayout() {
    let c = load();
    for case in &c.cases {
        let label = format!("[{}/{}]", case.rule, case.name);
        let a = region_from_rects(&case.rects_a);
        let b = region_from_rects(&case.rects_b);
        let got = apply_drc_case(case, &a, &b);
        assert_region_eq(&label, &got, &case.result);
    }
    eprintln!("{} DRC cases match KLayout {}", c.cases.len(), c.klayout_version);
}
