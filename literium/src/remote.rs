//! The litorium remote client (litorium `SYNC.md` §6–§7):
//! push, pull, and clone against a litorium gate.
//!
//! The wire model, in one breath: immutable objects (atoms,
//! fasciculi, spinae, atom-spinae, volumina, and version
//! **pre-images**) travel as SHA-256-named deposita inside
//! batch-put sarcinae; the two mutable artefacts are pinakes —
//! `historia` (the commit pointer, advanced by compare-and-swap)
//! and `manifest/<vhash>` (each version's mutable manifest
//! region). Pull walks the sarcina index past a local
//! **watermark**, stages records by hash, and lets the closure
//! walk assign types — records are untyped on the wire.
//!
//! A synced repository records `{remote, project, watermark}`
//! in `.lit/litorium` (`key = "value"` lines, like the
//! credentials file). The bearer is the account's access token
//! (account.rs); `LIT_TOKEN` overrides it for CI and testing
//! (the static-token path of SYNC.md §4), and `LIT_GATE_URL`
//! overrides the gate for local development.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;

use crate::account;
use crate::derive::hash_bytes;
use crate::error::{Error, Result};
use crate::repository::Repository;
use crate::theca::ObjectStore;
use crate::version_object::VersionObject;

pub const DEFAULT_GATE: &str = "https://p.litorium.org";
const USER_AGENT: &str = concat!("lit/", env!("CARGO_PKG_VERSION"));

/// Batch caps (the gate's sarcina limits).
const MAX_BATCH_RECORDS: usize = 8192;
const MAX_BATCH_BYTES: usize = 48 * 1024 * 1024; // headroom under 64 MiB

pub fn gate_url() -> String {
    std::env::var("LIT_GATE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_GATE.to_string())
        .trim_end_matches('/')
        .to_string()
}

/// The access token: `LIT_TOKEN` (static tokens, CI) or the
/// signed-in account's, refreshed per operation.
pub fn token() -> Result<String> {
    if let Ok(t) = std::env::var("LIT_TOKEN") {
        if !t.trim().is_empty() {
            return Ok(t.trim().to_string());
        }
    }
    let Some(mut credentials) = account::load()? else {
        err!("not signed in — run `lit account login` (or set LIT_TOKEN)");
    };
    account::access_token(&mut credentials)
}

// ── The .lit/litorium record ────────────────────────────────────

pub struct Remote {
    pub url: String,
    pub project: String,
    pub watermark: u64,
}

impl Remote {
    pub fn load(repo: &Repository) -> Result<Option<Remote>> {
        let path = repo.root().join(".lit/litorium");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => err!("read .lit/litorium: {e}"),
        };
        let mut fields = HashMap::new();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                fields.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
            }
        }
        let take = |k: &str| -> Result<String> {
            fields
                .get(k)
                .cloned()
                .ok_or_else(|| Error::new(format!(".lit/litorium lacks {k}")))
        };
        Ok(Some(Remote {
            url: take("remote")?,
            project: take("project")?,
            watermark: take("watermark")?.parse().unwrap_or(0),
        }))
    }

    pub fn save(&self, repo: &Repository) -> Result<()> {
        let body = format!(
            "remote = \"{}\"\nproject = \"{}\"\nwatermark = \"{}\"\n",
            self.url, self.project, self.watermark
        );
        std::fs::write(repo.root().join(".lit/litorium"), body)?;
        Ok(())
    }
}

/// A client-generated v4 UUID (litorium SPEC §3): random,
/// carrying no semantic content.
pub fn new_project_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| Error::new(format!("no randomness source: {e}")))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h = hex::encode(bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}

// ── HTTP ────────────────────────────────────────────────────────

pub struct Reply {
    pub status: u16,
    pub bytes: Vec<u8>,
}

impl Reply {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    fn error_message(&self) -> String {
        serde_json::from_slice::<serde_json::Value>(&self.bytes)
            .ok()
            .and_then(|v| {
                v.get("message")
                    .or_else(|| v.get("error"))
                    .and_then(|m| m.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| format!("http {}", self.status))
    }
}

fn call(
    method: &str,
    url: &str,
    token: &str,
    body: Option<&[u8]>,
    headers: &[(&str, &str)],
) -> Result<Reply> {
    let mut request = ureq::request(method, url)
        .set("User-Agent", USER_AGENT)
        .set("Authorization", &format!("Bearer {token}"));
    for (k, v) in headers {
        request = request.set(k, v);
    }
    let outcome = match body {
        Some(bytes) => request.send_bytes(bytes),
        None => request.call(),
    };
    let response = match outcome {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => err!("cannot reach {url}: {e}"),
    };
    let status = response.status();
    let mut bytes = Vec::new();
    // A 204 (and some empty bodies) can surface as an EOF from
    // ureq's reader; the status is authoritative, so a failed
    // read of an empty body is not an error.
    let _ = response.into_reader().read_to_end(&mut bytes);
    Ok(Reply { status, bytes })
}

fn sha256_hex(bytes: &[u8]) -> String {
    hash_bytes(bytes)
}

fn json_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(String::from)
}

// ── SRCN sarcina streams (the gate's sarcina framing) ───────────

fn encode_sarcina(records: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"SRCN");
    out.push(1);
    out.push(0);
    out.extend_from_slice(&(records.len() as u32).to_be_bytes());
    for (name, body) in records {
        out.push(b'D');
        out.extend_from_slice(&(name.len() as u16).to_be_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(body);
    }
    out
}

fn decode_sarcina(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    if bytes.len() < 10 || &bytes[0..4] != b"SRCN" || bytes[4] != 1 || bytes[5] != 0 {
        err!("bad sarcina stream from the gate");
    }
    let count = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
    let mut records = Vec::new();
    let mut i = 10usize;
    for _ in 0..count {
        if bytes.len() < i + 3 || bytes[i] != b'D' {
            err!("truncated sarcina stream");
        }
        let nlen = u16::from_be_bytes([bytes[i + 1], bytes[i + 2]]) as usize;
        i += 3;
        if bytes.len() < i + nlen + 4 {
            err!("truncated sarcina stream");
        }
        let name = std::str::from_utf8(&bytes[i..i + nlen])
            .map_err(|_| Error::new("bad record name in sarcina"))?
            .to_string();
        i += nlen;
        let blen =
            u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        i += 4;
        if bytes.len() < i + blen {
            err!("truncated sarcina stream");
        }
        records.push((name, bytes[i..i + blen].to_vec()));
        i += blen;
    }
    Ok(records)
}

// ── Closures ────────────────────────────────────────────────────

/// Everything a set of versions reaches, per type — the gc mark
/// walk (SPEC §6.10) reused as the transfer unit.
pub struct Closure {
    pub atoms: BTreeSet<String>,
    pub fasciculi: BTreeSet<String>,
    pub spinae: BTreeSet<String>,
    pub atom_spinae: BTreeSet<String>,
    pub volumina: BTreeSet<String>,
    pub versions: BTreeSet<String>,
}

impl Closure {
    pub fn empty() -> Closure {
        Closure {
            atoms: BTreeSet::new(),
            fasciculi: BTreeSet::new(),
            spinae: BTreeSet::new(),
            atom_spinae: BTreeSet::new(),
            volumina: BTreeSet::new(),
            versions: BTreeSet::new(),
        }
    }
}

/// Digest helper for callers (historia bytes, manifest CAS).
pub fn digest(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

pub fn closure(theca: &dyn ObjectStore, vhashes: &[String]) -> Result<Closure> {
    let mut c = Closure {
        atoms: BTreeSet::new(),
        fasciculi: BTreeSet::new(),
        spinae: BTreeSet::new(),
        atom_spinae: BTreeSet::new(),
        volumina: BTreeSet::new(),
        versions: vhashes.iter().cloned().collect(),
    };
    for vhash in &c.versions {
        let Some(vo) = VersionObject::load(theca, vhash)? else {
            err!("version {vhash} is not in the theca");
        };
        for (v, (s, a)) in vo.trios() {
            c.volumina.insert(v.clone());
            c.spinae.insert(s.clone());
            c.atom_spinae.insert(a.clone());
        }
    }
    for s in c.spinae.clone() {
        let Some(bytes) = theca.get("spina", &s)? else {
            err!("spina {s} is not in the theca");
        };
        for f in String::from_utf8_lossy(&bytes).split('\n').filter(|l| !l.is_empty()) {
            c.fasciculi.insert(f.to_string());
        }
    }
    for f in c.fasciculi.clone() {
        let Some(bytes) = theca.get("fasciculus", &f)? else {
            err!("fasciculus {f} is not in the theca");
        };
        for a in String::from_utf8_lossy(&bytes).split('\n').filter(|l| !l.is_empty()) {
            c.atoms.insert(a.to_string());
        }
    }
    Ok(c)
}

// ── Gate operations ─────────────────────────────────────────────

pub struct Gate {
    pub url: String,
    pub project: String,
    pub token: String,
}

impl Gate {
    fn route(&self, tail: &str) -> String {
        format!("{}/v1/c/{}/{}", self.url, self.project, tail)
    }

    pub fn create_project(&self) -> Result<()> {
        let reply = call(
            "POST",
            &format!("{}/v1/c/{}", self.url, self.project),
            &self.token,
            Some(b""),
            &[],
        )?;
        if reply.status != 201 {
            err!("project creation refused: {}", reply.error_message());
        }
        Ok(())
    }

    /// Which of `names` are absent (loose-cella sync
    /// negotiation, ≤512 per call — used against the corpus).
    pub fn absent(&self, names: &[String]) -> Result<Vec<String>> {
        let body = serde_json::json!({ "names": names }).to_string();
        let reply = call("POST", &self.route("absent"), &self.token, Some(body.as_bytes()), &[])?;
        if reply.status != 200 {
            err!("absent negotiation: {}", reply.error_message());
        }
        let value: serde_json::Value =
            serde_json::from_slice(&reply.bytes).map_err(|_| Error::new("bad absent response"))?;
        Ok(value
            .get("absent")
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default())
    }

    /// (bytes, sha256) of a pinax, or None when absent.
    pub fn get_pinax(&self, name: &str) -> Result<Option<(Vec<u8>, String)>> {
        let reply = call("GET", &self.route(&format!("pinax/{name}")), &self.token, None, &[])?;
        match reply.status {
            200 => {
                let digest = sha256_hex(&reply.bytes);
                Ok(Some((reply.bytes, digest)))
            }
            404 => Ok(None),
            _ => err!("pinax {name}: {}", reply.error_message()),
        }
    }

    /// CAS a pinax; `expected` is a digest or None for absence.
    /// Returns the outcome's `actual` digest on conflict.
    pub fn set_pinax(
        &self,
        name: &str,
        bytes: &[u8],
        expected: Option<&str>,
    ) -> Result<std::result::Result<(), Option<String>>> {
        let expected = expected.unwrap_or("none");
        let reply = call(
            "POST",
            &self.route(&format!("pinax/{name}")),
            &self.token,
            Some(bytes),
            &[("Apotheca-Expected", expected)],
        )?;
        match reply.status {
            200 => Ok(Ok(())),
            409 => {
                let actual = serde_json::from_slice::<serde_json::Value>(&reply.bytes)
                    .ok()
                    .and_then(|v| v.get("actual").and_then(|a| a.as_str()).map(String::from));
                Ok(Err(actual))
            }
            _ => err!("pinax {name}: {}", reply.error_message()),
        }
    }

    /// Batch-put one chunk; verifies every record was accepted.
    pub fn put_chunk(&self, records: &[(String, Vec<u8>)]) -> Result<()> {
        let reply = call(
            "POST",
            &self.route("deposita"),
            &self.token,
            Some(&encode_sarcina(records)),
            &[],
        )?;
        if reply.status != 200 {
            err!("batch-put refused: {}", reply.error_message());
        }
        let value: serde_json::Value = serde_json::from_slice(&reply.bytes)
            .map_err(|_| Error::new("bad batch-put response"))?;
        let failures = value
            .get("failures")
            .and_then(|f| f.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        if failures > 0 {
            err!(
                "the gate refused {failures} record(s): {}",
                value["failures"][0]["message"].as_str().unwrap_or("?")
            );
        }
        Ok(())
    }

    /// The sarcina index: (pack, seq_lo, seq_hi).
    pub fn index(&self) -> Result<Vec<(String, u64, u64)>> {
        let reply = call("GET", &self.route("sarcinae"), &self.token, None, &[])?;
        if reply.status != 200 {
            err!("sarcina index: {}", reply.error_message());
        }
        let mut lines = Vec::new();
        for raw in reply.text().lines() {
            let fields: Vec<&str> = raw.split('\t').collect();
            if fields.len() != 5 {
                err!("bad index line from the gate");
            }
            lines.push((
                fields[0].to_string(),
                fields[1].parse().map_err(|_| Error::new("bad index seq"))?,
                fields[2].parse().map_err(|_| Error::new("bad index seq"))?,
            ));
        }
        Ok(lines)
    }

    /// Fetch and verify one pack; the compaction race (a listed
    /// pack 404ing) surfaces as Ok(None) so the caller refetches
    /// the index.
    pub fn pack(&self, name: &str) -> Result<Option<Vec<(String, Vec<u8>)>>> {
        let reply = call("GET", &self.route(&format!("sarcina/{name}")), &self.token, None, &[])?;
        match reply.status {
            200 => {
                if sha256_hex(&reply.bytes) != name {
                    err!("pack {name} fails digest verification");
                }
                Ok(Some(decode_sarcina(&reply.bytes)?))
            }
            404 => Ok(None),
            _ => err!("pack {name}: {}", reply.error_message()),
        }
    }
}

// ── Shared sync plumbing ────────────────────────────────────────

const TYPES: [&str; 6] = [
    "atom",
    "fasciculus",
    "spina",
    "atom-spina",
    "volumen",
    "version",
];

fn theca_has_any(theca: &dyn ObjectStore, hash: &str) -> Result<bool> {
    for t in TYPES {
        if theca.has(t, hash)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Fetch every pack past the watermark into a hash-keyed staging
/// pool, retrying the index on a compaction race. Returns the
/// pool and the new watermark.
pub fn fetch_past_watermark(
    gate: &Gate,
    theca: &dyn ObjectStore,
    watermark: u64,
) -> Result<(HashMap<String, Vec<u8>>, u64)> {
    'attempt: for _ in 0..3 {
        let index = gate.index()?;
        let mut staged: HashMap<String, Vec<u8>> = HashMap::new();
        let mut highest = watermark;
        for (pack, _lo, hi) in &index {
            highest = highest.max(*hi);
            if *hi <= watermark {
                continue;
            }
            let Some(records) = gate.pack(pack)? else {
                continue 'attempt; // compaction won the race
            };
            for (name, body) in records {
                if sha256_hex(&body) != name {
                    err!("record {name} fails digest verification");
                }
                if !staged.contains_key(&name) && !theca_has_any(theca, &name)? {
                    staged.insert(name, body);
                }
            }
        }
        return Ok((staged, highest));
    }
    err!("the sarcina index kept changing underneath; try again");
}

/// Ingest new versions: recompose each from its staged pre-image
/// plus the manifest pinax, then let the closure walk type and
/// store every staged object. Completeness is an invariant —
/// any missing hash aborts before the historia advances.
pub fn ingest_versions(
    gate: &Gate,
    repo: &Repository,
    staged: &mut HashMap<String, Vec<u8>>,
    vhashes: &[String],
) -> Result<usize> {
    let theca = repo.theca();
    let mut stored = 0usize;

    let take = |hash: &str,
                    staged: &mut HashMap<String, Vec<u8>>|
     -> Result<Option<Vec<u8>>> { Ok(staged.remove(hash)) };

    // Versions first: pre-image + manifest region → version file.
    let mut new_objects: Vec<VersionObject> = Vec::new();
    for vhash in vhashes {
        if theca.has("version", vhash)? {
            continue;
        }
        let Some(preimage) = take(vhash, staged)? else {
            err!("corrupt remote: version pre-image {vhash} is missing");
        };
        let Some((manifest, _)) = gate.get_pinax(&format!("manifest%2F{vhash}"))? else {
            err!("corrupt remote: manifest pinax for {vhash} is missing");
        };
        let mut file = preimage.clone();
        file.extend_from_slice(&manifest);
        let vo = VersionObject::parse(&file)?;
        if vo.vhash() != *vhash {
            err!("corrupt remote: recomposed version does not hash to {vhash}");
        }
        theca.put_at("version", vhash, &file, true)?;
        stored += 1;
        new_objects.push(vo);
    }

    // The walk types everything else.
    let mut want: Vec<(&str, String)> = Vec::new();
    let mut spina_set: BTreeSet<String> = BTreeSet::new();
    for vo in &new_objects {
        for (v, (s, a)) in vo.trios() {
            want.push(("volumen", v.clone()));
            want.push(("spina", s.clone()));
            want.push(("atom-spina", a.clone()));
            spina_set.insert(s.clone());
        }
    }
    // Store what we can type so far (spinae needed for the next
    // level are read back from the theca or the pool).
    let mut store_typed = |type_: &str, hash: &str, staged: &mut HashMap<String, Vec<u8>>| -> Result<()> {
        if theca.has(type_, hash)? {
            return Ok(());
        }
        let Some(bytes) = staged.get(hash).cloned() else {
            err!("corrupt remote: {type_} {hash} is missing");
        };
        let written = theca.put(type_, &bytes)?;
        if written != hash {
            err!("corrupt remote: {type_} bytes do not hash to {hash}");
        }
        stored += 1;
        Ok(())
    };

    for (type_, hash) in &want {
        store_typed(type_, hash, staged)?;
    }
    let mut fasciculus_set: BTreeSet<String> = BTreeSet::new();
    for s in &spina_set {
        let bytes = theca
            .get("spina", s)?
            .ok_or_else(|| Error::new(format!("spina {s} vanished")))?;
        for f in String::from_utf8_lossy(&bytes).split('\n').filter(|l| !l.is_empty()) {
            fasciculus_set.insert(f.to_string());
        }
    }
    for f in &fasciculus_set {
        store_typed("fasciculus", f, staged)?;
    }
    let mut atom_set: BTreeSet<String> = BTreeSet::new();
    for f in &fasciculus_set {
        let bytes = theca
            .get("fasciculus", f)?
            .ok_or_else(|| Error::new(format!("fasciculus {f} vanished")))?;
        for a in String::from_utf8_lossy(&bytes).split('\n').filter(|l| !l.is_empty()) {
            atom_set.insert(a.to_string());
        }
    }
    for a in &atom_set {
        store_typed("atom", a, staged)?;
    }
    Ok(stored)
}

pub struct UploadSet {
    /// Referenced-before-referencer: atoms → fasciculi →
    /// spinae/atom-spinae/volumina → version pre-images.
    pub records: Vec<(String, Vec<u8>)>,
    /// vhash → manifest region, for the per-version pinakes.
    pub manifests: BTreeMap<String, Vec<u8>>,
}

/// The upload set for a push. Every record is
/// (sha256-name, bytes).
pub fn upload_set(
    repo: &Repository,
    local: &Closure,
    remote: &Closure,
) -> Result<UploadSet> {
    let theca = repo.theca();
    let mut records: Vec<(String, Vec<u8>)> = Vec::new();
    let push_new = |type_: &str,
                        ours: &BTreeSet<String>,
                        theirs: &BTreeSet<String>,
                        records: &mut Vec<(String, Vec<u8>)>|
     -> Result<()> {
        for hash in ours.difference(theirs) {
            let Some(bytes) = theca.get(type_, hash)? else {
                err!("{type_} {hash} is not in the theca");
            };
            records.push((hash.clone(), bytes));
        }
        Ok(())
    };
    push_new("atom", &local.atoms, &remote.atoms, &mut records)?;
    push_new("fasciculus", &local.fasciculi, &remote.fasciculi, &mut records)?;
    push_new("spina", &local.spinae, &remote.spinae, &mut records)?;
    push_new("atom-spina", &local.atom_spinae, &remote.atom_spinae, &mut records)?;
    push_new("volumen", &local.volumina, &remote.volumina, &mut records)?;
    // Version PRE-IMAGES: the depositum is the hashed pre-image,
    // named by the version hash; the manifest region rides its
    // pinax separately.
    let mut version_manifests: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for vhash in local.versions.difference(&remote.versions) {
        let Some(vo) = VersionObject::load(&theca, vhash)? else {
            err!("version {vhash} is not in the theca");
        };
        let preimage = vo.preimage().into_bytes();
        let full = vo.serialize();
        let manifest_region = full[preimage.len()..].to_vec();
        records.push((vhash.clone(), preimage));
        version_manifests.insert(vhash.clone(), manifest_region);
    }
    Ok(UploadSet {
        records,
        manifests: version_manifests,
    })
}

/// Chunk records under the batch caps and upload them all.
pub fn push_records(gate: &Gate, records: Vec<(String, Vec<u8>)>) -> Result<usize> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let records: Vec<(String, Vec<u8>)> = records
        .into_iter()
        .filter(|(name, _)| seen.insert(name.clone()))
        .collect();
    let total = records.len();
    let mut chunk: Vec<(String, Vec<u8>)> = Vec::new();
    let mut chunk_bytes = 0usize;
    for record in records {
        let record_bytes = record.0.len() + record.1.len() + 16;
        if !chunk.is_empty()
            && (chunk.len() >= MAX_BATCH_RECORDS || chunk_bytes + record_bytes > MAX_BATCH_BYTES)
        {
            gate.put_chunk(&chunk)?;
            chunk.clear();
            chunk_bytes = 0;
        }
        chunk_bytes += record_bytes;
        chunk.push(record);
    }
    if !chunk.is_empty() {
        gate.put_chunk(&chunk)?;
    }
    Ok(total)
}

// ── the opera surface (ref./opus. shares; SYNC via SPEC §5–§6) ──

/// The opera control plane (ref.litorium.org's /v1); override
/// with `LIT_OPERA_URL` for local dev.
pub fn opera_url() -> String {
    std::env::var("LIT_OPERA_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://ref.litorium.org".to_string())
        .trim_end_matches('/')
        .to_string()
}

pub struct Opera {
    pub url: String,
    pub token: String,
}

impl Opera {
    fn post(&self, path: &str, body: &[u8]) -> Result<Reply> {
        call("POST", &format!("{}{path}", self.url), &self.token, Some(body), &[])
    }

    /// Store a blob content-addressed; returns its sha256 key.
    pub fn put_artifact(&self, bytes: &[u8]) -> Result<String> {
        let reply = self.post("/v1/artifacts", bytes)?;
        if reply.status != 200 {
            err!("artifact upload: {}", reply.error_message());
        }
        let v: serde_json::Value = serde_json::from_slice(&reply.bytes)
            .map_err(|_| Error::new("bad artifact response"))?;
        json_field(&v, "artifact").ok_or_else(|| Error::new("no artifact key returned"))
    }

    pub fn create_share(
        &self,
        version: &str,
        project: &str,
        ttl_days: Option<u64>,
        page: &str,
        archive: &str,
    ) -> Result<String> {
        let mut body = serde_json::json!({
            "version": version, "project": project, "page": page, "archive": archive,
        });
        if let Some(d) = ttl_days {
            body["ttl"] = serde_json::json!(d);
        }
        let reply = self.post("/v1/shares", body.to_string().as_bytes())?;
        if reply.status != 200 {
            err!("share refused: {}", reply.error_message());
        }
        let v: serde_json::Value =
            serde_json::from_slice(&reply.bytes).map_err(|_| Error::new("bad share response"))?;
        json_field(&v, "url").ok_or_else(|| Error::new("no url returned"))
    }

    pub fn register_opus(
        &self,
        slug: &str,
        version: &str,
        project: &str,
        page: &str,
        archive: &str,
    ) -> Result<String> {
        let body = serde_json::json!({
            "slug": slug, "version": version, "project": project,
            "page": page, "archive": archive,
        });
        let reply = self.post("/v1/opera", body.to_string().as_bytes())?;
        if reply.status != 200 {
            err!("opus refused: {}", reply.error_message());
        }
        let v: serde_json::Value =
            serde_json::from_slice(&reply.bytes).map_err(|_| Error::new("bad opus response"))?;
        json_field(&v, "url").ok_or_else(|| Error::new("no url returned"))
    }

    pub fn list_shares(&self) -> Result<Vec<(String, i64)>> {
        let reply = call("GET", &format!("{}/v1/shares", self.url), &self.token, None, &[])?;
        if reply.status != 200 {
            err!("share list: {}", reply.error_message());
        }
        let v: serde_json::Value =
            serde_json::from_slice(&reply.bytes).map_err(|_| Error::new("bad share list"))?;
        Ok(v.get("shares")
            .and_then(|s| s.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| {
                        Some((
                            json_field(s, "token")?,
                            s.get("expires_at")?.as_i64()?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn list_opera(&self) -> Result<Vec<String>> {
        let reply = call("GET", &format!("{}/v1/opera", self.url), &self.token, None, &[])?;
        if reply.status != 200 {
            err!("opera list: {}", reply.error_message());
        }
        let v: serde_json::Value =
            serde_json::from_slice(&reply.bytes).map_err(|_| Error::new("bad opera list"))?;
        Ok(v.get("opera")
            .and_then(|o| o.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            .unwrap_or_default())
    }

    pub fn revoke_share(&self, token: &str) -> Result<()> {
        let reply = call(
            "DELETE",
            &format!("{}/v1/shares/{token}", self.url),
            &self.token,
            None,
            &[],
        )?;
        match reply.status {
            204 => Ok(()),
            404 => err!("no such share (already revoked or expired?)"),
            _ => err!("revoke: {}", reply.error_message()),
        }
    }
}

/// Materialize a version's documents + the exemplar archive +
/// a self-contained reading page (SPEC §9.7 / §5–§6). Returns
/// (archive_bytes, page_bytes).
pub fn share_artifacts(repo: &Repository, vhash: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let theca = repo.theca();
    let Some(vo) = VersionObject::load(&theca, vhash)? else {
        err!("version {vhash} is not in the theca");
    };
    let mut docs: Vec<(String, Vec<u8>)> = Vec::new();
    for path in vo.paths().cloned().collect::<Vec<_>>() {
        let doc = vo.document_at(&path).expect("path from manifest");
        let Some(bytes) = theca.get("volumen", doc)? else {
            err!("missing volumen for {path}");
        };
        docs.push((path, bytes));
    }
    let metadata = crate::exemplar::Metadata {
        basis: vhash.to_string(),
        segmentatio: repo.segmentation_version(),
    };
    // The archive to a temp file, then read back (write_archive
    // takes a path; the exchange format is identical to
    // `lit exemplar`).
    let tmp = std::env::temp_dir().join(format!("lit-share-{}.tgz", &vhash[..12]));
    crate::exemplar::write_archive(&tmp, &metadata, &docs)?;
    let archive = std::fs::read(&tmp).map_err(|e| Error::new(format!("read archive: {e}")))?;
    let _ = std::fs::remove_file(&tmp);
    let page = reading_page(vhash, &docs);
    Ok((archive, page))
}

/// A minimal self-contained reading page: the version's
/// documents as escaped text, no scripts, no external requests
/// — the litorium.org aesthetic. A literary work should be
/// readable at its link.
fn reading_page(vhash: &str, docs: &[(String, Vec<u8>)]) -> Vec<u8> {
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }
    let mut body = String::new();
    for (path, bytes) in docs {
        let text = String::from_utf8_lossy(bytes);
        body.push_str(&format!(
            "<section><h2>{}</h2><pre>{}</pre></section>\n",
            esc(path),
            esc(&text)
        ));
    }
    let doc = format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>A work on Litorium</title>\n<style>\n\
         :root{{color-scheme:light dark}}\n\
         body{{max-width:40rem;margin:0 auto;padding:2.5rem 1.25rem;\
         font:1.05rem/1.7 Georgia,serif;color:#26221c;background:#faf7f1}}\n\
         @media(prefers-color-scheme:dark){{body{{color:#e8e2d6;background:#1d1b17}}}}\n\
         h1{{font-weight:normal}}h2{{font-size:1rem;font-variant:small-caps;\
         letter-spacing:.08em;color:#8a7f6f;border-top:1px solid #d9d2c5;\
         padding-top:1.25rem;margin-top:2rem}}\n\
         pre{{white-space:pre-wrap;font:inherit;margin:.5rem 0}}\n\
         footer{{margin-top:3rem;padding-top:1rem;border-top:1px solid #d9d2c5;\
         font-size:.85rem;color:#8a7f6f}}\n</style></head>\n<body>\n\
         <h1>A work on Litorium</h1>\n{body}\
         <footer>Shared via <a href=\"https://litorium.org\">Litorium</a> · \
         version <code>{}</code></footer>\n</body></html>\n",
        &vhash[..12]
    );
    doc.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_ids_are_v4_uuids() {
        let id = new_project_id().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4');
        assert!("89ab".contains(id.chars().nth(19).unwrap()));
        assert_ne!(new_project_id().unwrap(), id);
    }

    #[test]
    fn sarcina_stream_roundtrip() {
        let records = vec![
            ("a".repeat(64), b"alpha".to_vec()),
            ("b".repeat(64), Vec::new()),
        ];
        let bytes = encode_sarcina(&records);
        assert_eq!(decode_sarcina(&bytes).unwrap(), records);
        assert!(decode_sarcina(&bytes[..bytes.len() - 2]).is_err());
    }
}
