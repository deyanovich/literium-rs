# literium-core

The identity layer of the [Literium](../README.md) specification,
as a standalone crate: content extraction (SPEC §3), segmentation
(SPEC §4), and derivation of the content-addressed objects
(SPEC §5–§6) — atoms, fasciculi, spina, atom-spina, content form,
and their SHA-256 identities.

Pure functions only: no I/O, no storage, no version-control
machinery. Those live in the `literium` crate, which depends on
this one and re-exports it unchanged; existing `literium::extract`
/ `literium::segment` / `literium::derive` paths are this crate's
modules.

Depend on this crate alone when you need Literium's atom/grex
identities without the VCS — derivation pipelines, citation
services, or derived modes plugged in via `segment::register_mode`
(the SPEC §4 extension point).

The dependency closure is deliberately minimal and entirely
identity-bearing:

- `sha2` / `hex` — the SPEC §0 hash encoding;
- `unicode-segmentation = "=1.12.0"` — UAX-29 sentence boundaries
  at exactly Unicode 16.0.0 (the SPEC §4 pin). The exact-version
  pin is load-bearing: a bump changes every atom hash and is a
  segmentation-version migration, not a routine upgrade.

The v0 Perl implementation (`literium-perl`, a sibling repository)
is the conformance oracle; the differential harness lives in the
`literium` crate's test suite.
