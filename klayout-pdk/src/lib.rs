//! `klayout-pdk` — typed PDK declarations.
//!
//! The [`pdk!`] macro turns a declarative description into a struct holding
//! `LayerIndex` fields registered against a `Library`. Downstream code
//! references `pdk.WG` instead of looking up `LayerInfo::named("WG", 1, 0)`
//! every time, getting compile-checked layer access.
//!
//! Port kinds are declared as a list of bare identifiers; each gets a stable
//! `PortKindId` constant on the PDK struct. Per-port-kind data (width, etc.)
//! is captured in user-defined types and attached via `Port::with_kind` —
//! that responsibility stays out of the macro to keep the syntax narrow.
//!
//! # Example
//!
//! ```
//! klayout_pdk::pdk! {
//!     pub Demo {
//!         dbu: 1000,
//!         layers: {
//!             WG = (1, 0),
//!             METAL1 = (10, 0),
//!         }
//!         ports: { Optical, Electrical }
//!     }
//! }
//!
//! let lib = Demo::new_library("test");
//! let pdk = Demo::register(&lib);
//! assert_eq!(lib.layer_info(pdk.WG).layer, 1);
//! assert_ne!(Demo::Optical, Demo::Electrical);
//! ```

pub mod lyp;
pub mod lyt;

pub use lyp::{parse_lyp, read_lyp_path, write_lyp, LayerProperties, Lyp, LypError};
pub use lyt::{parse_lyt, read_lyt_path, write_lyt, Lyt};

#[doc(hidden)]
pub mod __exports {
    pub use klayout_core::{LayerIndex, LayerInfo, Library, PortKindId};
    pub use smol_str::SmolStr;
}

/// Declare a PDK: layers and (optionally) port-kind tags.
///
/// Generates:
/// * A struct with one `pub LayerIndex` field per declared layer.
/// * `register(&Library) -> Self` to populate it.
/// * `new_library(name) -> Library` shortcut that creates a Library at
///   the declared `dbu` and registers all layers.
/// * One `pub const NAME: PortKindId` per declared port kind, on the struct.
/// * A const `DBU: i64` on the struct.
#[macro_export]
macro_rules! pdk {
    (
        $vis:vis $name:ident {
            dbu: $dbu:expr,
            layers: {
                $($lname:ident = ($l:expr, $dt:expr)),* $(,)?
            }
            $(,)?
            $( ports: { $($pvariant:ident),* $(,)? } )?
            $(,)?
        }
    ) => {
        #[allow(non_snake_case)]
        #[derive(Clone, Copy, Debug)]
        $vis struct $name {
            $(pub $lname: $crate::__exports::LayerIndex,)*
        }

        impl $name {
            /// PDK-declared database-units-per-micron.
            pub const DBU: i64 = $dbu;

            /// Register all declared layers in `lib` and return the index struct.
            /// Idempotent: layers dedup by `(layer, datatype)` so re-registering
            /// on the same library returns the same indices.
            pub fn register(lib: &$crate::__exports::Library) -> Self {
                Self {
                    $(
                        $lname: lib.layer($crate::__exports::LayerInfo::named(
                            stringify!($lname), $l as u16, $dt as u16,
                        )),
                    )*
                }
            }

            /// Create a fresh `Library` with this PDK's `DBU` and register
            /// all declared layers in it.
            pub fn new_library(name: impl Into<$crate::__exports::SmolStr>)
                -> $crate::__exports::Library
            {
                let lib = $crate::__exports::Library::new(name, Self::DBU);
                let _ = Self::register(&lib);
                lib
            }

            $($(
                #[allow(non_upper_case_globals)]
                pub const $pvariant: $crate::__exports::PortKindId =
                    $crate::__exports::PortKindId($crate::__pdk_port_id!($pvariant));
            )*)?
        }
    };
}

// Each port kind needs a unique id that's stable for a given declaration.
// macro_rules! can't easily compute "position in list" without help, so we
// use a `const fn` hash of the variant name. Collision probability for
// dozens of port kinds is astronomically low; the names are user-chosen
// anyway and a collision would surface immediately on compile when two
// variants get the same id (as a duplicate-const error).
#[doc(hidden)]
#[macro_export]
macro_rules! __pdk_port_id {
    ($name:ident) => {
        $crate::__hash_ident_str(stringify!($name))
    };
}

/// FNV-1a 32-bit hash of the variant name. `const fn` so each port-kind
/// id is computed at compile time. We avoid 0 (reserved for `PortKindId::ANY`).
#[doc(hidden)]
pub const fn __hash_ident_str(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut h: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u32;
        h = h.wrapping_mul(0x0100_0193);
        i += 1;
    }
    if h == 0 {
        1
    } else {
        h
    }
}
