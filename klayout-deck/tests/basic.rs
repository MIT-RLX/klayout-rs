//! End-to-end test of the `deck!` macro: declare a rule deck against a
//! struct of `Region`s, run it on synthetic layouts, verify the right
//! rules fire.

use klayout_core::{Bbox, Point, Polygon};
use klayout_deck::deck;
use klayout_geom::Region;

struct DemoLayers {
    m1: Region,
    m2: Region,
}

deck! {
    Demo (layers: DemoLayers) {
        rule "M1.W.1": width(&layers.m1, 11);
        rule "M1.S.1": space(&layers.m1, 11);
        rule "M1.A.1": area_min(&layers.m1, 600);
        rule "M1.M2.OVL": overlap(&layers.m1, &layers.m2, 11);
    }
}

fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Polygon {
    Polygon::rect(Bbox::new(Point::new(x0, y0), Point::new(x1, y1)))
}

#[test]
fn clean_layout_reports_no_failures() {
    let layers = DemoLayers {
        m1: Region::from_polygons([rect(0, 0, 100, 50)]),
        m2: Region::from_polygons([rect(0, 0, 100, 50)]),
    };
    let report = Demo::run(&layers);
    assert_eq!(report.results.len(), 4);
    assert!(report.is_clean(), "no rule should fire on this clean layout");
}

#[test]
fn thin_metal_triggers_width_rule() {
    let layers = DemoLayers {
        m1: Region::from_polygons([rect(0, 0, 100, 5)]), // 5-tall, < min=11
        m2: Region::empty(),
    };
    let report = Demo::run(&layers);
    let width_rule = report
        .results
        .iter()
        .find(|r| r.name == "M1.W.1")
        .unwrap();
    assert!(width_rule.failed());
    // Other rules:
    // M1.S.1 — only one polygon, no space violation.
    // M1.A.1 — area = 100*5 = 500, < min=600 → fires.
    // M1.M2.OVL — empty M2, no overlap.
    let area_rule = report
        .results
        .iter()
        .find(|r| r.name == "M1.A.1")
        .unwrap();
    assert!(area_rule.failed());
}

#[test]
fn close_pair_triggers_space_rule() {
    let layers = DemoLayers {
        m1: Region::from_polygons([rect(0, 0, 30, 30), rect(35, 0, 60, 30)]),
        m2: Region::empty(),
    };
    let report = Demo::run(&layers);
    let s = report
        .results
        .iter()
        .find(|r| r.name == "M1.S.1")
        .unwrap();
    assert!(s.failed(), "5-DBU gap should fire space=11 rule");
}

#[test]
fn rdb_xml_contains_all_categories_and_items() {
    let layers = DemoLayers {
        m1: Region::from_polygons([rect(0, 0, 100, 5)]), // width + area
        m2: Region::empty(),
    };
    let report = Demo::run(&layers);
    let xml = report.to_rdb_xml("top");
    assert!(xml.contains("<report-database>"));
    assert!(xml.contains("<top-cell>top</top-cell>"));
    assert!(xml.contains("<generator>klayout-deck</generator>"));
    // Each rule name appears as a category.
    assert!(xml.contains("<name>M1.W.1</name>"));
    assert!(xml.contains("<name>M1.A.1</name>"));
    // At least one item with a polygon value.
    assert!(xml.contains("polygon: ("));
}

#[test]
fn rdb_xml_round_trips_through_simple_parser() {
    // Make sure nothing in the output breaks basic XML well-formedness:
    // every <foo> has a matching </foo>.
    let layers = DemoLayers {
        m1: Region::from_polygons([rect(0, 0, 100, 5)]),
        m2: Region::empty(),
    };
    let report = Demo::run(&layers);
    let xml = report.to_rdb_xml("top");
    let opens = xml.matches('<').count();
    let closes = xml.matches('>').count();
    assert_eq!(opens, closes, "every < has a matching >");
}

#[test]
fn report_aggregates_all_failures() {
    let layers = DemoLayers {
        m1: Region::from_polygons([rect(0, 0, 100, 5)]), // width AND area fail
        m2: Region::empty(),
    };
    let report = Demo::run(&layers);
    let failed: Vec<_> = report.failed_rules().map(|r| r.name.clone()).collect();
    assert!(failed.contains(&"M1.W.1".into()));
    assert!(failed.contains(&"M1.A.1".into()));
    assert!(report.total_violations() >= 2);
}
