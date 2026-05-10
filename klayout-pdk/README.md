# klayout-pdk

Typed PDK declarations for the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

The [`pdk!`] macro turns a declarative description into a struct
holding `LayerIndex` fields registered against a `Library`. Downstream
code references `pdk.WG` instead of doing a name-based layer lookup
on every access — and the compiler enforces that referenced layers
exist.

Port kinds are declared as a list of bare identifiers; each gets a
stable `PortKindId` constant on the PDK struct. Per-port-kind data
(width, etc.) is captured in user-defined types and attached via
`Port::with_kind`, keeping the macro syntax narrow.

## Example

```rust
klayout_pdk::pdk! {
    pub Demo {
        dbu: 1000,
        layers: {
            WG = (1, 0),
            METAL1 = (10, 0),
        }
        ports: { Optical, Electrical }
    }
}

let lib = Demo::new_library("test");
let pdk = Demo::register(&lib);
assert_eq!(lib.layer_info(pdk.WG).layer, 1);
assert_ne!(Demo::Optical, Demo::Electrical);
```

## License

Licensed under GPL-3.0-only.
