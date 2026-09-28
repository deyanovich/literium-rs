# Literium (Rust core)

Version control for literary texts, where the atom of change is
the sentence (prose) or the line (poetry) — not the line of code.

Rust core of the Literium specification (`SPEC.md` in the
Literium spec repository, published separately). The completed
Perl v0 implementation (`literium-perl`, a sibling repository) is
the **conformance oracle**: the differential tests in
`literium/tests/oracle.rs` assert both implementations derive
byte-identical objects from the same input.

## Status

Full v0 port, feature-equivalent with literium-perl: the
pure-functions core (SPEC §3–§6), the storage layer (theca behind
the `ObjectStore` trait, version objects, histories), the
revision engine (grex ranges, revision detection, splice,
classification, the §7.4 diff, the §9.6 combining merge, verbatim
atom spans), and the complete `lit` CLI (conscribe / inscribe /
status / exscribe / contexe / conflicts / tabula / experiment /
show / diff / stemma / gc / fsck, register aliases, elevation).
The
`ObjectStore` trait is the seam where syntheca cellae attach at
the litorium/v2 remoting layer.

## Workspace layout

A two-crate Cargo workspace:

- [`literium-core/`](literium-core/) — the identity layer
  (extraction, segmentation, derivation; SPEC §3–§6) as a
  standalone, I/O-free crate carrying the Unicode pin. Non-VCS
  consumers (citanda, litorium, litodex) depend on this crate
  alone; it also builds for `wasm32-unknown-unknown`.
- [`literium/`](literium/) — storage, versions, histories, and
  the `lit` CLI; depends on `literium-core` and re-exports it, so
  `literium::extract` / `literium::segment` / `literium::derive`
  remain valid paths.

Conformance is enforced two ways: unit suites ported from the
Perl `t/` tests, and differential oracle tests
(`literium/tests/oracle.rs`, `literium/tests/cli_oracle.rs`) that
drive both implementations over the same inputs and assert
byte-identical derivations, transcripts, and `.lit/` trees
(timestamps masked).

## Segmentation pin

SPEC §4 pins segmentation to Unicode 16.0.0. `literium-core`
carries its UAX-29 sentence-boundary property tables via
`unicode-segmentation = "=1.12.0"` (whose data is exactly
16.0.0); the exact-version pin is load-bearing — bumping it
changes every atom hash and is a segmentation-version migration,
not a routine upgrade. The pin's equivalence with Perl `\b{sb}`
at UCD 16.0.0 was verified against the UCD `SentenceBreakTest`
file (512/512) and differentially over literary corpus,
adversarial, and fuzz inputs (zero divergences).

## Tests

```
cargo test
```

The oracle tests skip with a note when the sibling
`literium-perl` checkout or a perl with UCD 16.0.0 is
unavailable.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
