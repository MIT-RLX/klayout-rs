# klayout-deck

Declarative DRC rule decks for the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

Replaces hand-rolled chains of `klayout_drc::*` calls with a readable
rule-deck literal that compiles down to ordinary Rust functions. Each
rule expression evaluates to a `Region` (the violation area) and is
collected into a `RuleDeckReport`.

## Example

```rust,ignore
use klayout_deck::deck;
use klayout_geom::Region;

struct Layers {
    m1: Region,
    m2: Region,
    via12: Region,
}

deck! {
    Sky130 (layers: Layers) {
        rule "M1.W.1":   width(layers.m1, 100);
        rule "M1.S.1":   space(layers.m1, 200);
        rule "M1.A.1":   area_min(layers.m1, 6000);
        rule "V12.E":    enclosing(layers.m1, layers.via12, 50);
        rule "M1.M2.OL": overlap(layers.m1, layers.m2, 100);
    }
}

let report = Sky130::run(&layers);
assert_eq!(report.failed_rules().count(), 0);
```

Rules are evaluated in parallel with `rayon`. The macro lowers each
clause to a typed call into `klayout-drc`, so a typo in a rule name
or a missing layer surfaces at compile time.

## License

Licensed under GPL-3.0-only.
