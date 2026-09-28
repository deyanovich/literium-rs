//! `lit serve` — the protocol (`spec/PROTOCOL.md`) over JSON-RPC 2.0
//! on stdio.
//!
//! This is the first transport binding of the operation contract.
//! It carries the session lifecycle (§4) and, in this slice, the
//! read-only methods `log` and `show` (§7.2). Read methods build
//! directly on the library API (`Repository`, `History`,
//! `VersionObject`); the write methods and the napi transport
//! reuse the same operation layer, so nothing here is throwaway
//! (PROTOCOL §11).
//!
//! Framing is the LSP-style header block (§3.2): `Content-Length:
//! <N>\r\n\r\n` then N UTF-8 bytes. `stderr` is diagnostics only.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::bonds::{self, Bonds, Choice};
use crate::history::{History, Inscriptio};
use crate::repository::Repository;
use crate::theca::ObjectStore;
use crate::version_object::VersionObject;
use crate::revision_detect::{detect, SideInput};
use std::collections::HashSet;
use std::collections::{BTreeMap, HashMap};

const PROTOCOL_VERSION: &str = "0.1";

// ── error codes (PROTOCOL §5) ───────────────────────────────────

const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const NOT_INITIALIZED: i64 = -33000;
const NOT_A_REPOSITORY: i64 = -33001;
const UNKNOWN_VERSION: i64 = -33005;
const AMBIGUOUS_VERSION: i64 = -33006;
const UNKNOWN_HISTORY: i64 = -33007;

struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> RpcError {
        RpcError { code, message: message.into() }
    }
}

type MethodResult = Result<Value, RpcError>;

/// Session state, fixed at `initialize`.
struct Session {
    root: Option<PathBuf>,
    initialized: bool,
}

impl Session {
    /// The repository for this session, or a `NotARepository` error.
    fn repo(&self) -> Result<Repository, RpcError> {
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| RpcError::new(NOT_INITIALIZED, "no workspace; call initialize first"))?;
        let found = Repository::discover(root)
            .ok_or_else(|| RpcError::new(NOT_A_REPOSITORY, "no enclosing .lit/ repository"))?;
        Repository::new(found).map_err(|e| RpcError::new(NOT_A_REPOSITORY, e.to_string()))
    }
}

// ── entry point ─────────────────────────────────────────────────

/// Run the server on stdin/stdout until `exit`. Returns the exit
/// status.
pub fn serve(reader: &mut dyn BufRead, out: &mut dyn Write) -> i32 {
    let mut session = Session { root: None, initialized: false };

    loop {
        let message = match read_message(reader) {
            Ok(Some(m)) => m,
            Ok(None) => return 0, // EOF: peer closed the stream
            Err(e) => {
                eprintln!("lit serve: {e}");
                return 1;
            }
        };

        let request: Value = match serde_json::from_slice(&message) {
            Ok(v) => v,
            Err(_) => {
                write_message(out, &error_response(&Value::Null, -32700, "parse error"));
                continue;
            }
        };

        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        // Notifications (no id) that end the session.
        if method == "exit" {
            return if session.initialized { 0 } else { 1 };
        }
        if method == "initialized" {
            session.initialized = true;
            continue;
        }

        let outcome = dispatch(&mut session, method, params);

        // A notification (no id) gets no response.
        let Some(id) = id else { continue };
        let response = match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(e) => error_response(&id, e.code, &e.message),
        };
        write_message(out, &response);
    }
}

fn dispatch(session: &mut Session, method: &str, params: Value) -> MethodResult {
    match method {
        "initialize" => method_initialize(session, params),
        "shutdown" => Ok(Value::Null),
        // Every method below needs an initialized session.
        _ if !session.initialized && method != "initialize" => {
            Err(RpcError::new(NOT_INITIALIZED, "call initialize first"))
        }
        "log" => method_log(session, params),
        "show" => method_show(session, params),
        "status" => method_status(session, params),
        "diff" => method_diff(session, params),
        "add" => method_add(session, params),
        "commit" => method_commit(session, params),
        other => Err(RpcError::new(
            METHOD_NOT_FOUND,
            format!("method '{other}' is not implemented in this build"),
        )),
    }
}

// ── methods ─────────────────────────────────────────────────────

fn method_initialize(session: &mut Session, params: Value) -> MethodResult {
    let root = params
        .get("workspaceRoot")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    session.root = root.clone();

    // The engine walks upward to the nearest .lit/ (SPEC §1.3);
    // discover returns the repository root directly.
    let found = root.as_deref().and_then(Repository::discover);

    let account = match crate::account::load() {
        Ok(Some(c)) => json!({ "signedIn": true, "email": c.email, "issuer": c.issuer }),
        _ => json!({ "signedIn": false }),
    };

    Ok(json!({
        "serverInfo": { "name": "literium", "version": env!("CARGO_PKG_VERSION") },
        "protocolVersion": PROTOCOL_VERSION,
        "engine": {
            "formatVersion": crate::FORMAT_VERSION,
            "segmentationVersion": crate::SEGMENTATION_VERSION,
        },
        "repository": match &found {
            Some(root) => json!({ "found": true, "root": root.to_string_lossy() }),
            None => json!({ "found": false }),
        },
        "capabilities": {
            "methods": ["initialize", "log", "show", "status", "diff", "add", "commit"],
            "bonds": true,
            "remote": false,
            "progress": false,
            "cancellation": false,
        },
        "account": account,
    }))
}

fn inscriptio_json(entry: &Inscriptio, history: &str, ordinal: usize) -> Value {
    json!({
        "version": entry.vhash,
        "history": history,
        "timestamp": entry.timestamp,
        "message": entry.message,
        "ordinal": ordinal,
    })
}

fn method_log(session: &mut Session, params: Value) -> MethodResult {
    let repo = session.repo()?;
    let name = params.get("history").and_then(Value::as_str).unwrap_or("historia");
    let history = resolve_history(&repo, name)?;
    let entries = history
        .entries()
        .map_err(|e| RpcError::new(-32603, e.to_string()))?;
    // With a `path`, keep only the inscriptiones where that
    // document's content changed — a note's own history. The
    // ordinal stays the entry's position in the full history.
    let path = params.get("path").and_then(Value::as_str);
    let mut inscriptiones: Vec<Value> = Vec::new();
    let mut prev: Option<String> = None;
    for (i, e) in entries.iter().enumerate() {
        match path {
            None => inscriptiones.push(inscriptio_json(e, name, i + 1)),
            Some(p) => {
                let vo = repo
                    .version(&e.vhash)
                    .map_err(|err| RpcError::new(-32603, err.to_string()))?;
                let cur = vo.and_then(|v| v.document_at(p).map(str::to_string));
                if cur.is_some() && cur != prev {
                    inscriptiones.push(inscriptio_json(e, name, i + 1));
                }
                prev = cur;
            }
        }
    }
    Ok(json!({ "history": name, "inscriptiones": inscriptiones, "path": path }))
}

fn method_show(session: &mut Session, params: Value) -> MethodResult {
    let repo = session.repo()?;
    let version = params
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, "show: 'version' is required"))?;
    let vhash = resolve_version(&repo, version)?;
    let vo = repo
        .version(&vhash)
        .map_err(|e| RpcError::new(-32603, e.to_string()))?
        .ok_or_else(|| RpcError::new(UNKNOWN_VERSION, format!("no version {vhash}")))?;

    if let Some(path) = params.get("path").and_then(Value::as_str) {
        let volumen = vo
            .document_at(path)
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("no document at '{path}'")))?;
        if params.get("atoms").and_then(Value::as_bool).unwrap_or(false) {
            return show_atoms(&repo, &vo, path, volumen, &params);
        }
        let trio = vo.trio_for(volumen);
        return Ok(json!({
            "path": path,
            "document": document_json(&vo, path),
            "trio": trio.map(|[v, s, a]| json!({ "volumen": v, "spina": s, "atomSpina": a })),
        }));
    }

    Ok(version_json(&vo, &vhash))
}

/// `show` with `atoms: true` (PROTOCOL §7.2): the citation listing,
/// plus the molecules where display bonds apply (SPEC §7.4 *Bonds*).
fn show_atoms(
    repo: &Repository,
    vo: &VersionObject,
    path: &str,
    volumen: &str,
    params: &Value,
) -> MethodResult {
    let bytes = repo
        .theca()
        .get("volumen", volumen)
        .map_err(|e| RpcError::new(-32603, e.to_string()))?
        .ok_or_else(|| RpcError::new(-32603, format!("volumen {volumen} missing from theca")))?;
    let attrs = repo
        .attributes_for_version(vo)
        .map_err(|e| RpcError::new(-32603, e.to_string()))?;
    let d = crate::derive::derive(&bytes, attrs.mode_for(path))
        .ok_or_else(|| RpcError::new(-32603, format!("stored volumen {volumen} does not derive")))?;

    let mut atoms: Vec<Value> = Vec::new();
    let mut k = 0usize;
    for (g, grex) in d.grexes.iter().enumerate() {
        for (a, atom) in grex.iter().enumerate() {
            k += 1;
            atoms.push(json!({
                "coordinate": format!("g{}a{}", g + 1, a + 1),
                "flat": format!("a{k}"),
                "hash": crate::derive::hash_bytes(atom.as_bytes()),
                "text": atom,
            }));
        }
    }
    let mut result = json!({ "path": path, "mode": d.mode, "atoms": atoms });

    let mut resolver = bonds_of(repo, params)?;
    if d.mode == "prose" {
        if let Some(b) = resolver.for_path(path).map_err(|e| RpcError::new(-32603, e.to_string()))? {
            let mut molecules: Vec<Value> = Vec::new();
            let mut k = 0usize;
            for (g, grex) in d.grexes.iter().enumerate() {
                let atoms: Vec<&str> = grex.iter().map(String::as_str).collect();
                for (first, last) in bonds::molecules(&atoms, Some(&b)) {
                    let k0 = k + 1;
                    k += last - first + 1;
                    molecules.push(json!({
                        "coordinate": bonds::coord_range(g, first, last),
                        "flat": bonds::flat_range(k0, k),
                        "text": bonds::molecule_text(&atoms, first, last),
                    }));
                }
            }
            result["molecules"] = Value::Array(molecules);
            result["bonds"] = json!(resolver.file_name_for(path));
        }
    }
    Ok(result)
}

/// The display-bonds choice of a request (PROTOCOL §7.2/§7.3):
/// `bonds: false` shows atoms; `desm: <path>` names one file
/// (repository-relative); otherwise the nearest `.desm` per document.
fn bonds_of(repo: &Repository, params: &Value) -> Result<Bonds, RpcError> {
    let choice = if params.get("bonds").and_then(Value::as_bool) == Some(false) {
        Choice::None
    } else if let Some(f) = params.get("desm").and_then(Value::as_str) {
        Choice::File(repo.root().join(f))
    } else {
        Choice::Discover
    };
    Ok(Bonds::new(repo.root(), choice))
}

fn document_json(vo: &VersionObject, path: &str) -> Value {
    let volumen = vo.document_at(path);
    match volumen.and_then(|v| vo.trio_for(v)) {
        Some([v, s, a]) => {
            json!({ "path": path, "volumen": v, "spina": s, "atomSpina": a })
        }
        None => json!({ "path": path, "volumen": volumen }),
    }
}

fn version_json(vo: &VersionObject, vhash: &str) -> Value {
    let documents: Vec<Value> = vo.paths().map(|p| document_json(vo, p)).collect();
    json!({
        "hash": vhash,
        "segmentation": vo.segmentation(),
        "documents": documents,
        "manifest": vo.manifest(),
    })
}

// ── resolution helpers (a subset of cli::resolve_version) ───────

fn resolve_history(repo: &Repository, name: &str) -> Result<History, RpcError> {
    repo.history(name)
        .ok_or_else(|| RpcError::new(UNKNOWN_HISTORY, format!("no history '{name}'")))
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// A subset of the CLI's `resolve_version` (cli.rs): a 64-hex
/// hash, a 6–63-hex unique prefix, or a history name (latest).
fn resolve_version(repo: &Repository, arg: &str) -> Result<String, RpcError> {
    if is_hex(arg) && arg.len() == 64 {
        let has = repo
            .theca()
            .has("version", arg)
            .map_err(|e| RpcError::new(-32603, e.to_string()))?;
        if !has {
            return Err(RpcError::new(UNKNOWN_VERSION, format!("no version {arg}")));
        }
        return Ok(arg.to_string());
    }
    if is_hex(arg) && arg.len() >= 6 && arg.len() <= 63 && repo.history(arg).is_none() {
        let hits: Vec<String> = repo
            .theca()
            .list("version")
            .map_err(|e| RpcError::new(-32603, e.to_string()))?
            .into_iter()
            .filter(|h| h.starts_with(arg))
            .collect();
        return match hits.len() {
            0 => Err(RpcError::new(UNKNOWN_VERSION, format!("no version matches '{arg}'"))),
            1 => Ok(hits.into_iter().next().unwrap()),
            _ => Err(RpcError::new(AMBIGUOUS_VERSION, format!("ambiguous prefix '{arg}'"))),
        };
    }
    let history = resolve_history(repo, arg)?;
    let latest = history
        .latest()
        .map_err(|e| RpcError::new(-32603, e.to_string()))?
        .ok_or_else(|| RpcError::new(UNKNOWN_HISTORY, format!("history '{arg}' has no versions")))?;
    Ok(latest.vhash)
}

// ── status & diff (build SideInputs via the public API) ─────────

fn side_of_version(
    repo: &Repository,
    vo: Option<&VersionObject>,
) -> Result<SideInput, RpcError> {
    match vo {
        None => Ok(SideInput { manifest: BTreeMap::new(), states: HashMap::new() }),
        Some(vo) => Ok(SideInput {
            manifest: vo.manifest().clone(),
            states: repo.doc_states(vo).map_err(|e| RpcError::new(-32603, e.to_string()))?,
        }),
    }
}

fn side_of_working_tree(repo: &Repository) -> Result<SideInput, RpcError> {
    // The tabula if present, else the live scan (SPEC §7.4 default
    // right side).
    if let Some(tab) = repo.read_tabula().map_err(|e| RpcError::new(-32603, e.to_string()))? {
        return side_of_version(repo, Some(&tab));
    }
    let scan = repo.scan().map_err(|e| RpcError::new(-32603, e.to_string()))?;
    let mut states = HashMap::new();
    for (path, vh) in &scan.manifest {
        states.insert(path.clone(), scan.docs[vh].clone());
    }
    Ok(SideInput { manifest: scan.manifest.clone(), states })
}

fn side_of_scan(repo: &Repository) -> Result<(crate::repository::Scan, SideInput), RpcError> {
    let scan = repo.scan().map_err(|e| RpcError::new(-32603, e.to_string()))?;
    let mut states = HashMap::new();
    for (path, vh) in &scan.manifest {
        states.insert(path.clone(), scan.docs[vh].clone());
    }
    let side = SideInput { manifest: scan.manifest.clone(), states };
    Ok((scan, side))
}

// ── write methods (conscribe/inscribe via the shared ops) ───────

fn method_add(session: &mut Session, params: Value) -> MethodResult {
    let root = session
        .root
        .clone()
        .ok_or_else(|| RpcError::new(NOT_INITIALIZED, "no workspace"))?;
    let mode = params.get("mode").and_then(Value::as_str).unwrap_or("prose");

    let (repo, created) = match Repository::discover(&root) {
        Some(found) => (
            Repository::new(found).map_err(|e| RpcError::new(NOT_A_REPOSITORY, e.to_string()))?,
            false,
        ),
        None => (
            Repository::create(&root, mode).map_err(|e| RpcError::new(-32603, e.to_string()))?,
            true,
        ),
    };
    repo.assert_segmentation().map_err(|e| RpcError::new(-33011, e.to_string()))?;

    let (scan, tree) = side_of_scan(&repo)?;
    let tip = repo.historia().latest().map_err(|e| RpcError::new(-32603, e.to_string()))?;
    let tip_vo = match &tip {
        Some(t) => repo.version(&t.vhash).map_err(|e| RpcError::new(-32603, e.to_string()))?,
        None => None,
    };
    let baseline = side_of_version(&repo, tip_vo.as_ref())?;
    if crate::cli::states_equal(&tree, &baseline) {
        return Err(RpcError::new(-33003, "nothing pending against historia"));
    }
    if let Some(tab) = repo.read_tabula().map_err(|e| RpcError::new(-32603, e.to_string()))? {
        if tab.manifest() == &tree.manifest {
            return Err(RpcError::new(-33003, "the tabula already holds this state"));
        }
    }

    let (trios, manifest) = repo.store_scan(&scan).map_err(|e| RpcError::new(-32603, e.to_string()))?;
    let vo = VersionObject::new(&repo.segmentation_version(), trios, manifest)
        .map_err(|e| RpcError::new(-32603, e.to_string()))?;
    repo.write_tabula(&vo).map_err(|e| RpcError::new(-32603, e.to_string()))?;

    Ok(json!({ "created": created, "conscribed": scan.manifest.len() }))
}

fn method_commit(session: &mut Session, params: Value) -> MethodResult {
    let repo = session.repo()?;
    let message = params
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, "commit: 'message' is required"))?
        .to_string();
    let history_name = params.get("history").and_then(Value::as_str).unwrap_or("historia");
    let target = resolve_history(&repo, history_name)?;

    // --fons: adopt a whole version (SPEC §8.3).
    if let Some(fons) = params.get("fons").and_then(Value::as_str) {
        let vhash = resolve_version(&repo, fons)?;
        target.append(&vhash, &message, None).map_err(|e| RpcError::new(-32603, e.to_string()))?;
        return Ok(json!({ "inscriptiones": [latest_inscriptio(&target, history_name)?] }));
    }

    repo.assert_segmentation().map_err(|e| RpcError::new(-33011, e.to_string()))?;
    let tip = target.latest().map_err(|e| RpcError::new(-32603, e.to_string()))?;
    let parent_vo = match &tip {
        Some(t) => repo.version(&t.vhash).map_err(|e| RpcError::new(-32603, e.to_string()))?,
        None => None,
    };
    let parent_side = side_of_version(&repo, parent_vo.as_ref())?;
    let tab = repo
        .read_tabula()
        .map_err(|e| RpcError::new(-32603, e.to_string()))?
        .ok_or_else(|| RpcError::new(-33003, "the tabula is empty; add first"))?;
    let tabula_side = side_of_version(&repo, Some(&tab))?;
    let pending = detect(&parent_side, &tabula_side).map_err(|e| RpcError::new(-32603, e.to_string()))?;
    if pending.revisions.is_empty() {
        return Err(RpcError::new(-33003, "no pending revisions"));
    }
    let subset: HashSet<String> = pending.revisions.iter().map(|r| r.id.clone()).collect();
    crate::cli::Cli::inscribe_subset(
        &repo, &target, parent_vo.as_ref(), &parent_side, &tabula_side, &pending, &subset, &message,
    )
    .map_err(|e| RpcError::new(-32603, e.to_string()))?;
    repo.clear_tabula().map_err(|e| RpcError::new(-32603, e.to_string()))?;
    repo.clear_contextio().map_err(|e| RpcError::new(-32603, e.to_string()))?;

    Ok(json!({ "inscriptiones": [latest_inscriptio(&target, history_name)?] }))
}

fn latest_inscriptio(target: &History, history: &str) -> Result<Value, RpcError> {
    let entries = target.entries().map_err(|e| RpcError::new(-32603, e.to_string()))?;
    let ordinal = entries.len();
    let e = entries.last().ok_or_else(|| RpcError::new(-32603, "history is empty after inscription"))?;
    Ok(inscriptio_json(e, history, ordinal))
}

fn method_status(session: &mut Session, params: Value) -> MethodResult {
    let repo = session.repo()?;
    let name = params.get("history").and_then(Value::as_str).unwrap_or("historia");
    let target = resolve_history(&repo, name)?;
    let tip = target.latest().map_err(|e| RpcError::new(-32603, e.to_string()))?;

    let tabula = repo.read_tabula().map_err(|e| RpcError::new(-32603, e.to_string()))?;
    let pending = match &tabula {
        None => Vec::new(),
        Some(tab) => {
            let parent_vo = match &tip {
                Some(t) => repo.version(&t.vhash).map_err(|e| RpcError::new(-32603, e.to_string()))?,
                None => None,
            };
            let parent_side = side_of_version(&repo, parent_vo.as_ref())?;
            let tabula_side = side_of_version(&repo, Some(tab))?;
            let p = detect(&parent_side, &tabula_side).map_err(|e| RpcError::new(-32603, e.to_string()))?;
            p.revisions
                .iter()
                .map(|r| {
                    json!({
                        "index": r.index,
                        "path": r.path,
                        "category": r.category.or(r.what),
                        "grexes": r.tabula_grexes,
                    })
                })
                .collect()
        }
    };

    Ok(json!({
        "repository": repo.root().to_string_lossy(),
        "history": name,
        "historiaTip": tip.map(|t| t.vhash),
        "tabula": tabula.is_some(),
        "pending": pending,
    }))
}

fn change_json(c: &crate::diff::Change) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("kind".into(), json!(c.kind.name()));
    o.insert("coord".into(), json!(c.coord));
    if let Some(t) = &c.to {
        o.insert("to".into(), json!(t));
    }
    if let Some(old) = &c.old {
        o.insert("old".into(), json!(old));
    }
    if let Some(new) = &c.new {
        o.insert("new".into(), json!(new));
    }
    Value::Object(o)
}

fn method_diff(session: &mut Session, params: Value) -> MethodResult {
    let repo = session.repo()?;
    let side_b = match params.get("right").and_then(Value::as_str) {
        Some(r) => {
            let vh = resolve_version(&repo, r)?;
            let vo = repo.version(&vh).map_err(|e| RpcError::new(-32603, e.to_string()))?;
            side_of_version(&repo, vo.as_ref())?
        }
        None => side_of_working_tree(&repo)?,
    };
    let side_a = match params.get("left").and_then(Value::as_str) {
        Some(l) => {
            let vh = resolve_version(&repo, l)?;
            let vo = repo.version(&vh).map_err(|e| RpcError::new(-32603, e.to_string()))?;
            side_of_version(&repo, vo.as_ref())?
        }
        None => {
            let tip = repo.historia().latest().map_err(|e| RpcError::new(-32603, e.to_string()))?;
            let vo = match &tip {
                Some(t) => repo.version(&t.vhash).map_err(|e| RpcError::new(-32603, e.to_string()))?,
                None => None,
            };
            side_of_version(&repo, vo.as_ref())?
        }
    };

    let path_filter = params.get("path").and_then(Value::as_str);
    // Display bonds (SPEC §7.4 *Bonds*): changes over molecules
    // where a .desm applies to a prose document.
    let mut resolver = bonds_of(&repo, &params)?;
    let diffs = crate::diff::collate_with(&side_a, &side_b, &mut |p| resolver.for_path(p))
        .map_err(|e| RpcError::new(-32603, e.to_string()))?;
    let documents: Vec<Value> = diffs
        .iter()
        .filter(|d| path_filter.is_none_or(|p| d.path == p))
        .map(|d| {
            let mut o = json!({
                "path": d.path,
                "category": d.category,
                "changes": d.changes.iter().map(change_json).collect::<Vec<_>>(),
            });
            let prose = side_b
                .states
                .get(&d.path)
                .or_else(|| side_a.states.get(&d.path))
                .is_some_and(|s| s.mode == "prose");
            if prose {
                if let Some(f) = resolver.file_name_for(&d.path) {
                    o["bonds"] = json!(f);
                }
            }
            o
        })
        .collect();
    Ok(json!({ "documents": documents }))
}

// ── framing (PROTOCOL §3.2) ─────────────────────────────────────

fn read_message(reader: &mut dyn BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None); // EOF
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // end of headers
        }
        if let Some(v) = trimmed.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok();
        }
    }
    let len = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

fn write_message(out: &mut dyn Write, message: &Value) {
    let body = serde_json::to_vec(message).expect("serialize response");
    let _ = write!(out, "Content-Length: {}\r\n\r\n", body.len());
    let _ = out.write_all(&body);
    let _ = out.flush();
}

fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
