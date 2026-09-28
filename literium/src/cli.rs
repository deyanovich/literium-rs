//! The lit command surface. Verbs per SPEC §8–§9; provisional CLI
//! details per PROVISIONAL.md. Latin/Greek register aliases per
//! SPEC §0.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, Write};
use std::path::PathBuf;

use crate::attributes::Attributes;
use crate::bonds::{self, Bonds};
use crate::derive::derive;
use crate::diff;
use crate::elevate;
use crate::account;
use crate::remote;
use crate::exemplar;
use crate::error::{Error, Result};
use crate::fsck;
use crate::history::History;
use crate::repository::{Repository, Scan};
use crate::revision_detect::{detect, Pending, RevKind, Revision, SideInput};
use crate::splice::volumen_for;
use crate::theca::ObjectStore;
use crate::version_object::VersionObject;

pub struct Cli<'a> {
    pub cwd: PathBuf,
    pub input: &'a mut dyn BufRead,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

// Both registers are equal peers (HISTORY §15): the Latin/Greek
// human names and the plain-English git names resolve to the same
// handler. status/diff/gc/fsck/show/push/pull/clone already share
// the git name and need no entry.
fn alias(cmd: &str) -> &str {
    match cmd {
        // Latin / Greek human register.
        "conscribo" => "conscribe",
        "inscribo" | "epigrapho" => "inscribe",
        "exscribo" | "ekgrapho" => "exscribe",
        "contexo" | "synhyphaino" => "contexe",
        "experimentum" => "experiment",
        "confero" => "collate",
        // git peer register.
        "add" | "stage" => "conscribe",
        "commit" => "inscribe",
        "checkout" | "restore" => "exscribe",
        "merge" => "contexe",
        "branch" => "experiment",
        "log" => "show",
        "blame" => "stemma",
        other => other,
    }
}

/// Guidance for git verbs literium omits by design (HISTORY §15):
/// a one-line redirect that teaches the literium way, instead of a
/// blank "unknown command". `cmd` has already passed `alias()`, so
/// only the deliberately-absent verbs reach here.
fn git_hint(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "init" => "literium has no init: the first `lit conscribe` creates \
             the repository. (TERMINOLOGY: init.)",
        "revert" => "literium has no revert primitive. Re-inscribe the old \
             version: `lit inscribe --fons <version>`. (TERMINOLOGY: revert.)",
        "cherry-pick" => "literium has no cherry-pick primitive. Adopt a \
             version into a history: `lit inscribe --fons <version>`. \
             (TERMINOLOGY: cherry-pick.)",
        "rebase" => "literium has no rebase. Histories are linear event logs \
             — there is nothing to replay onto a new base. (TERMINOLOGY: rebase.)",
        "switch" => "literium has no branch-switch: there is no current-branch \
             pointer. Materialize a version with `lit exscribe <version>`; work \
             an alternate line with `lit experiment`. (TERMINOLOGY: HEAD.)",
        "stash" => "literium has no stash: the tabula is the single-slot \
             scratchpad. `lit conscribe` sets the working tree aside; `lit \
             tabula` inspects it, `lit tabula discard` empties it. \
             (TERMINOLOGY: stash.)",
        "fetch" => "literium has no fetch: `lit pull` fast-forwards the \
             historia (fetch-and-apply in one). (TERMINOLOGY: fetch.)",
        "tag" => "literium has no local tag in v1. A permanent named pointer \
             to a version is a Litorium permalink: `lit opus <slug> <version>`. \
             (TERMINOLOGY: tag.)",
        "remote" | "origin" => "literium has no remote command: a synced repo \
             records its project in `.lit/litorium`; `lit push` / `lit pull` / \
             `lit clone` are the remote verbs. (TERMINOLOGY: remote.)",
        _ => return None,
    })
}

const USAGE: &str = "usage: lit <command> [options]

Latin/Greek and git names are equal peers — use either.

  conscribe · add      snapshot the working tree onto the tabula
                       (creates the repository on first use)
  inscribe · commit    finalize the tabula (or a named version) onto
                       a history  [-<n> | --all] [-m <msg>] [--fons <src>]
  status               list pending revisions        [<target-history>]
  exscribe · checkout  copy a version back into the working tree
                       (also: restore)
  contexe · merge      weave a source into the working tree (combining
                       merge)   [<target>] [--base <version>] [--force]
  conflicts            list contextio conflicts; 'conflicts clear' discards
  tabula               inspect the tabula; 'tabula discard' empties it
  experiment · branch  create/delete/list an experiment history
  show · log           view a history or a version
                       [--atoms|<coord>] [--desm <file>|--no-desm]
  diff                 compare versions, the tabula, or the working tree
                       [--unified] [--desm <file>|--no-desm]
  stemma · blame       a version's life-path through the histories; with
                       a coordinate, an atom's first/last inscription
  gc                   delete unreachable objects        [--dry-run]
  fsck                 verify repository integrity
  exemplar             pack a version's documents for correspondence
                       (or repack an unpacked exemplar) [<source>] [-o <file>]
  collate              ingest a returned exemplar into the working
                       tree   <file> [--force]

litorium (the cloud host):
  push                 sync the historia (the first push creates the
                       project)   [--remote <url>]
  pull                 fast-forward the historia from litorium
  clone                fetch a project into a new directory
                       <project-id> [<dir>] [--remote <url>]
  publish              contribute the historia's atoms to the public
                       corpus (permanent)   [<version>] [--yes]
  share                'share' exports the historia; 'share <version>
                       [--expires <days>]' mints a link; 'shares' lists
  opus                 'opus <slug> <version>' permanent permalink;
                       'opera' lists them
  retract              'retract --link <token>' revokes a share;
                       'retract <version>' drops an inscriptio
  account              litorium sign-in: 'account login [--email <a>]',
                       'account handle [<name>]', 'account status', logout

Git verbs literium omits (init, revert, cherry-pick, rebase, switch,
stash, fetch, tag, remote) print a one-line redirect.
";

impl<'a> Cli<'a> {
    /// Exit status.
    pub fn run(&mut self, argv: &[String]) -> i32 {
        let mut argv: Vec<String> = argv.to_vec();
        let cmd = if argv.is_empty() {
            String::new()
        } else {
            argv.remove(0)
        };
        let cmd = alias(&cmd).to_string();

        if cmd.is_empty() || cmd == "help" || cmd == "--help" {
            self.say(USAGE);
            return if cmd.is_empty() { 1 } else { 0 };
        }

        let result = match cmd.as_str() {
            "conscribe" => self.cmd_conscribe(&mut argv),
            "inscribe" => self.cmd_inscribe(&mut argv),
            "status" => self.cmd_status(&mut argv),
            "exscribe" => self.cmd_exscribe(&mut argv),
            "contexe" => self.cmd_contexe(&mut argv),
            "conflicts" => self.cmd_conflicts(&mut argv),
            "tabula" => self.cmd_tabula(&mut argv),
            "experiment" => self.cmd_experiment(&mut argv),
            "show" => self.cmd_show(&mut argv),
            "diff" => self.cmd_diff(&mut argv),
            "stemma" => self.cmd_stemma(&mut argv),
            "gc" => self.cmd_gc(&mut argv),
            "fsck" => self.cmd_fsck(&mut argv),
            "exemplar" => self.cmd_exemplar(&mut argv),
            "collate" => self.cmd_collate(&mut argv),
            "push" => self.cmd_push(&mut argv),
            "pull" => self.cmd_pull(&mut argv),
            "clone" => self.cmd_clone(&mut argv),
            "publish" => self.cmd_publish(&mut argv),
            "share" => self.cmd_share(&mut argv),
            "shares" => self.cmd_shares(&mut argv),
            "opus" => self.cmd_opus(&mut argv),
            "opera" => self.cmd_opera(&mut argv),
            "retract" => self.cmd_retract(&mut argv),
            "account" => self.cmd_account(&mut argv),
            "serve" => return crate::serve::serve(self.input, self.out),
            _ => {
                if let Some(hint) = git_hint(&cmd) {
                    self.errsay(&format!("lit: {hint}\n"));
                } else {
                    self.errsay(&format!("lit: unknown command '{cmd}'\n{USAGE}"));
                }
                return 1;
            }
        };
        match result {
            Ok(()) => 0,
            Err(e) => {
                let mut msg = e.0;
                if !msg.ends_with('\n') {
                    msg.push('\n');
                }
                self.errsay(&format!("lit: {msg}"));
                1
            }
        }
    }


    // ── litorium sync (SYNC.md §6): push / pull / clone ─────────

    fn historia_vhashes(bytes: &[u8]) -> Vec<String> {
        String::from_utf8_lossy(bytes)
            .lines()
            .filter_map(|l| l.split('\t').next().map(String::from))
            .filter(|h| h.len() == 64)
            .collect()
    }

    fn cmd_push(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let mut remote_url: Option<String> = None;
        parse_opts(argv, |opt, take| match opt {
            "--remote" => {
                remote_url = Some(take()?);
                Ok(true)
            }
            _ => Ok(false),
        })
        .map_err(|_| Error::new("bad push options"))?;
        if !argv.is_empty() {
            err!("push takes no arguments");
        }
        let repo = self.repo()?;
        repo.assert_segmentation()?;
        let local_bytes = std::fs::read(repo.root().join(".lit/historia")).unwrap_or_default();
        if local_bytes.is_empty() {
            err!("nothing to push — inscribe something first");
        }

        let token = remote::token()?;
        let (record, first_push) = match remote::Remote::load(&repo)? {
            Some(existing) => {
                if remote_url.is_some() {
                    err!("this repository already has a remote (.lit/litorium)");
                }
                (existing, false)
            }
            None => (
                remote::Remote {
                    url: remote_url.unwrap_or_else(remote::gate_url),
                    project: remote::new_project_id()?,
                    watermark: 0,
                },
                true,
            ),
        };
        let gate = remote::Gate {
            url: record.url.clone(),
            project: record.project.clone(),
            token,
        };
        if first_push {
            gate.create_project()?;
            record.save(&repo)?;
            self.say(&format!(
                "created project {} on {}\n",
                record.project, record.url
            ));
        }

        let remote_pinax = gate.get_pinax("historia")?;
        let (remote_bytes, expected) = match &remote_pinax {
            Some((bytes, digest)) => (bytes.clone(), Some(digest.clone())),
            None => (Vec::new(), None),
        };
        if remote_bytes == local_bytes {
            self.say("Already up to date.\n");
            return Ok(());
        }
        if !local_bytes.starts_with(remote_bytes.as_slice()) {
            err!("the remote historia has inscriptiones you don't — pull first");
        }

        // Both closures computed locally: v1 is single-writer,
        // so everything the remote holds arrived via a prior
        // push of this historia (SYNC.md §6).
        let local_versions = Self::historia_vhashes(&local_bytes);
        let remote_versions = Self::historia_vhashes(&remote_bytes);
        let theca = repo.theca();
        let local_closure = remote::closure(&theca, &local_versions)?;
        let remote_closure = if remote_versions.is_empty() {
            remote::Closure::empty()
        } else {
            remote::closure(&theca, &remote_versions)?
        };
        let upload = remote::upload_set(&repo, &local_closure, &remote_closure)?;
        let objects = remote::push_records(&gate, upload.records)?;

        // Manifest pinakes: create, or replace (the manifest
        // region is the version's sole mutable-in-place part).
        for (vhash, manifest) in &upload.manifests {
            let name = format!("manifest%2F{vhash}");
            match gate.set_pinax(&name, manifest, None)? {
                Ok(()) => {}
                Err(actual) => {
                    if actual.as_deref() == Some(remote::digest(manifest).as_str()) {
                        continue;
                    }
                    if gate
                        .set_pinax(&name, manifest, actual.as_deref())?
                        .is_err()
                    {
                        err!("manifest for {vhash} keeps changing — try again");
                    }
                }
            }
        }

        // The commit point.
        if gate
            .set_pinax("historia", &local_bytes, expected.as_deref())?
            .is_err()
        {
            err!("the remote historia changed underneath — pull and retry");
        }
        let new_versions = local_versions.len() - remote_versions.len();
        self.say(&format!(
            "pushed {objects} object{}, {new_versions} version{}\n",
            if objects == 1 { "" } else { "s" },
            if new_versions == 1 { "" } else { "s" }
        ));
        Ok(())
    }

    fn cmd_pull(&mut self, argv: &mut Vec<String>) -> Result<()> {
        if !argv.is_empty() {
            err!("pull takes no arguments");
        }
        let repo = self.repo()?;
        let Some(mut record) = remote::Remote::load(&repo)? else {
            err!("no litorium remote here — `lit push` creates one, `lit clone` fetches one");
        };
        let gate = remote::Gate {
            url: record.url.clone(),
            project: record.project.clone(),
            token: remote::token()?,
        };

        let Some((remote_bytes, _)) = gate.get_pinax("historia")? else {
            err!("the remote has no historia yet");
        };
        let local_bytes = std::fs::read(repo.root().join(".lit/historia")).unwrap_or_default();
        if remote_bytes == local_bytes {
            self.say("Already up to date.\n");
            return Ok(());
        }
        if !remote_bytes.starts_with(local_bytes.as_slice()) {
            err!(
                "the histories diverged — v1 pulls are fast-forward only \
                 (local inscriptiones the remote lacks: push first)"
            );
        }

        let local_count = Self::historia_vhashes(&local_bytes).len();
        let remote_versions = Self::historia_vhashes(&remote_bytes);
        let new_versions: Vec<String> = remote_versions[local_count..].to_vec();

        let theca = repo.theca();
        let (mut staged, new_watermark) =
            remote::fetch_past_watermark(&gate, &theca, record.watermark)?;
        let stored = remote::ingest_versions(&gate, &repo, &mut staged, &new_versions)?;

        std::fs::write(repo.root().join(".lit/historia"), &remote_bytes)?;
        record.watermark = new_watermark;
        record.save(&repo)?;
        self.say(&format!(
            "pulled {} version{} ({stored} object{}); `lit exscribe historia` \
             materializes the latest\n",
            new_versions.len(),
            if new_versions.len() == 1 { "" } else { "s" },
            if stored == 1 { "" } else { "s" }
        ));
        Ok(())
    }

    fn cmd_clone(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let mut remote_url: Option<String> = None;
        parse_opts(argv, |opt, take| match opt {
            "--remote" => {
                remote_url = Some(take()?);
                Ok(true)
            }
            _ => Ok(false),
        })
        .map_err(|_| Error::new("bad clone options"))?;
        if argv.is_empty() || argv.len() > 2 {
            err!("usage: lit clone <project-id> [<dir>] [--remote <url>]");
        }
        let project = argv.remove(0);
        let dir = if argv.is_empty() {
            project.clone()
        } else {
            argv.remove(0)
        };
        let target = self.cwd.join(&dir);
        if target.exists() && std::fs::read_dir(&target)?.next().is_some() {
            err!("directory '{dir}' exists and is not empty");
        }

        let gate = remote::Gate {
            url: remote_url.unwrap_or_else(remote::gate_url),
            project: project.clone(),
            token: remote::token()?,
        };
        let Some((historia_bytes, _)) = gate.get_pinax("historia")? else {
            err!("no such project, no access, or nothing pushed yet");
        };
        let versions = Self::historia_vhashes(&historia_bytes);
        if versions.is_empty() {
            err!("the project's historia is empty");
        }

        std::fs::create_dir_all(&target)?;
        // The attributes file this writes is provisional; the
        // exscribe below replaces it with the version's own.
        let repo = Repository::create(&target, "prose")?;
        let mut record = remote::Remote {
            url: gate.url.clone(),
            project,
            watermark: 0,
        };
        record.save(&repo)?;

        let theca = repo.theca();
        let (mut staged, watermark) = remote::fetch_past_watermark(&gate, &theca, 0)?;
        let stored = remote::ingest_versions(&gate, &repo, &mut staged, &versions)?;
        std::fs::write(repo.root().join(".lit/historia"), &historia_bytes)?;
        record.watermark = watermark;
        record.save(&repo)?;

        // Materialize the latest version into the working tree.
        let latest = versions.last().expect("non-empty");
        let vo = repo
            .version(latest)?
            .ok_or_else(|| Error::new("cloned version missing from the theca"))?;
        for path in vo.paths().cloned().collect::<Vec<_>>() {
            let doc = vo.document_at(&path).expect("path from manifest");
            let bytes = repo.theca().get("volumen", doc)?;
            self.write_doc(&repo, &path, bytes)?;
        }
        self.say(&format!(
            "cloned {} version{} ({stored} object{}) into {dir}\n",
            versions.len(),
            if versions.len() == 1 { "" } else { "s" },
            if stored == 1 { "" } else { "s" }
        ));
        Ok(())
    }

    // ── account (litorium identity; account.rs) ─────────────────

    // ── litorium sharing & publication (SPEC §5–§6, §9.4–§9.7) ──

    fn cmd_publish(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let mut yes = false;
        let mut version: Option<String> = None;
        while !argv.is_empty() {
            let a = argv.remove(0);
            match a.as_str() {
                "--yes" | "-y" => yes = true,
                other if version.is_none() => version = Some(other.to_string()),
                other => err!("unexpected argument '{other}'"),
            }
        }
        let repo = self.repo()?;
        repo.assert_segmentation()?;
        let vhashes = match version {
            Some(v) => vec![self.resolve_version(&repo, &v)?],
            None => {
                let bytes = std::fs::read(repo.root().join(".lit/historia")).unwrap_or_default();
                let vs = Self::historia_vhashes(&bytes);
                if vs.is_empty() {
                    err!("nothing to publish — inscribe something first");
                }
                vs
            }
        };
        let closure = remote::closure(&repo.theca(), &vhashes)?;
        let atoms: Vec<String> = closure.atoms.iter().cloned().collect();
        if !self.confirm(
            &format!(
                "publish {} atom(s) to the public corpus? This is a permanent, \
                 consent-ful contribution",
                atoms.len()
            ),
            yes,
        )? {
            err!("not published");
        }

        let gate = remote::Gate {
            url: remote::gate_url(),
            project: "corpus".to_string(),
            token: remote::token()?,
        };
        // Negotiate absence in ≤512-name chunks, then batch-put
        // the missing atoms' bytes (named by their identity
        // hash; the corpus name_policy accepts them verbatim).
        let theca = repo.theca();
        let mut uploaded = 0usize;
        for chunk in atoms.chunks(512) {
            let missing = gate.absent(&chunk.to_vec())?;
            let mut records: Vec<(String, Vec<u8>)> = Vec::new();
            for h in &missing {
                let Some(bytes) = theca.get("atom", h)? else {
                    err!("atom {h} is not in the theca");
                };
                records.push((h.clone(), bytes));
            }
            if !records.is_empty() {
                uploaded += remote::push_records(&gate, records)?;
            }
        }
        self.say(&format!(
            "published {uploaded} new atom(s) to the corpus ({} already present)\n",
            atoms.len() - uploaded
        ));
        Ok(())
    }

    fn cmd_share(&mut self, argv: &mut Vec<String>) -> Result<()> {
        // `lit share` with no version: historia to stdout.
        if argv.is_empty() {
            let repo = self.repo()?;
            let bytes = std::fs::read(repo.root().join(".lit/historia")).unwrap_or_default();
            let _ = self.out.write_all(&bytes);
            return Ok(());
        }
        let mut expires: Option<u64> = None;
        let mut version: Option<String> = None;
        while !argv.is_empty() {
            let a = argv.remove(0);
            match a.as_str() {
                "--expires" => {
                    if argv.is_empty() {
                        err!("--expires needs a number of days");
                    }
                    expires = Some(
                        argv.remove(0)
                            .parse()
                            .map_err(|_| Error::new("--expires must be a number of days"))?,
                    );
                }
                other if version.is_none() => version = Some(other.to_string()),
                other => err!("unexpected argument '{other}'"),
            }
        }
        let version = version.expect("checked non-empty");
        let repo = self.repo()?;
        repo.assert_segmentation()?;
        let record = remote::Remote::load(&repo)?
            .ok_or_else(|| Error::new("push the project first — a share needs a remote project"))?;
        let vhash = self.resolve_version(&repo, &version)?;
        let (archive, page) = remote::share_artifacts(&repo, &vhash)?;
        let opera = remote::Opera { url: remote::opera_url(), token: remote::token()? };
        let ah = opera.put_artifact(&archive)?;
        let ph = opera.put_artifact(&page)?;
        let url = opera.create_share(&vhash, &record.project, expires, &ph, &ah)?;
        self.say(&format!("{url}\n"));
        Ok(())
    }

    fn cmd_shares(&mut self, argv: &mut Vec<String>) -> Result<()> {
        if !argv.is_empty() {
            err!("shares takes no arguments");
        }
        let opera = remote::Opera { url: remote::opera_url(), token: remote::token()? };
        let shares = opera.list_shares()?;
        if shares.is_empty() {
            self.say("No active shares.\n");
        }
        for (token, expires) in shares {
            self.say(&format!(
                "https://ref.litorium.org/{token}\texpires {expires}\n"
            ));
        }
        Ok(())
    }

    fn cmd_opus(&mut self, argv: &mut Vec<String>) -> Result<()> {
        if argv.len() != 2 {
            err!("usage: lit opus <slug> <version>");
        }
        let slug = argv.remove(0);
        let version = argv.remove(0);
        let repo = self.repo()?;
        repo.assert_segmentation()?;
        let record = remote::Remote::load(&repo)?
            .ok_or_else(|| Error::new("push the project first — an opus needs a remote project"))?;
        let vhash = self.resolve_version(&repo, &version)?;
        let (archive, page) = remote::share_artifacts(&repo, &vhash)?;
        let opera = remote::Opera { url: remote::opera_url(), token: remote::token()? };
        let ah = opera.put_artifact(&archive)?;
        let ph = opera.put_artifact(&page)?;
        let url = opera.register_opus(&slug, &vhash, &record.project, &ph, &ah)?;
        self.say(&format!("{url}\n"));
        Ok(())
    }

    fn cmd_opera(&mut self, argv: &mut Vec<String>) -> Result<()> {
        if !argv.is_empty() {
            err!("opera takes no arguments");
        }
        let opera = remote::Opera { url: remote::opera_url(), token: remote::token()? };
        let opera_list = opera.list_opera()?;
        if opera_list.is_empty() {
            self.say("No registered opera.\n");
        }
        for hs in opera_list {
            self.say(&format!("https://opus.litorium.org/{hs}\n"));
        }
        Ok(())
    }

    fn cmd_retract(&mut self, argv: &mut Vec<String>) -> Result<()> {
        if argv.first().map(String::as_str) == Some("--link") {
            argv.remove(0);
            if argv.len() != 1 {
                err!("usage: lit retract --link <token>");
            }
            let token = argv.remove(0);
            let opera = remote::Opera { url: remote::opera_url(), token: remote::token()? };
            opera.revoke_share(&token)?;
            self.say("Share revoked.\n");
            return Ok(());
        }
        // retract <version>: drop the inscriptio from the local
        // historia, then advance the remote by CAS — the
        // sanctioned non-fast-forward rewrite (SYNC.md §6).
        if argv.len() != 1 {
            err!("usage: lit retract <version> | lit retract --link <token>");
        }
        let version = argv.remove(0);
        let repo = self.repo()?;
        let vhash = self.resolve_version(&repo, &version)?;
        let path = repo.root().join(".lit/historia");
        let bytes = std::fs::read(&path).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let kept: Vec<&str> = text
            .lines()
            .filter(|l| l.split('\t').next() != Some(vhash.as_str()))
            .collect();
        let before = text.lines().count();
        if kept.len() == before {
            err!("no inscriptio for {} on the historia", &vhash[..12]);
        }
        if !self.confirm(
            &format!("retract version {} from the historia?", &vhash[..12]),
            false,
        )? {
            err!("not retracted");
        }
        let mut new_bytes = kept.join("\n");
        if !new_bytes.is_empty() {
            new_bytes.push('\n');
        }
        std::fs::write(&path, &new_bytes)?;

        // Advance the remote if this repo is synced.
        if let Some(record) = remote::Remote::load(&repo)? {
            let gate = remote::Gate {
                url: record.url.clone(),
                project: record.project.clone(),
                token: remote::token()?,
            };
            if let Some((remote_bytes, digest)) = gate.get_pinax("historia")? {
                // Only rewrite if the remote still carries the line.
                let remote_text = String::from_utf8_lossy(&remote_bytes).into_owned();
                if remote_text
                    .lines()
                    .any(|l| l.split('\t').next() == Some(vhash.as_str()))
                {
                    let remote_kept: Vec<&str> = remote_text
                        .lines()
                        .filter(|l| l.split('\t').next() != Some(vhash.as_str()))
                        .collect();
                    let mut rb = remote_kept.join("\n");
                    if !rb.is_empty() {
                        rb.push('\n');
                    }
                    if gate
                        .set_pinax("historia", rb.as_bytes(), Some(&digest))?
                        .is_err()
                    {
                        self.say(
                            "Local historia retracted; the remote changed underneath — \
                             pull and `lit retract` again to propagate.\n",
                        );
                        return Ok(());
                    }
                }
            }
        }
        self.say(&format!("Retracted {}.\n", &vhash[..12]));
        Ok(())
    }

    fn cmd_account(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let sub = if argv.is_empty() {
            "status".to_string()
        } else {
            argv.remove(0)
        };
        match sub.as_str() {
            "login" => self.cmd_account_login(argv),
            "handle" => self.cmd_account_handle(argv),
            "status" => self.cmd_account_status(argv),
            "logout" => self.cmd_account_logout(argv),
            other => {
                err!(
                    "unknown account subcommand '{other}' \
                     (login|handle|status|logout)"
                );
            }
        }
    }

    fn cmd_account_login(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let mut email: Option<String> = None;
        while !argv.is_empty() {
            let arg = argv.remove(0);
            match arg.as_str() {
                "--email" => {
                    if argv.is_empty() {
                        err!("--email needs an address");
                    }
                    email = Some(argv.remove(0));
                }
                other => err!("unknown login option '{other}'"),
            }
        }
        let email = match email {
            Some(e) => e,
            None => self.prompt("email: ")?,
        };
        if email.is_empty() || !email.contains('@') {
            err!("that does not look like an email address");
        }

        let grant = account::begin_login(&email)?;
        self.say(&format!(
            "A sign-in link is on its way to {email}.\n\
             Open it only if the code in the message is:\n\n\
             \x20   {}\n\n\
             Waiting for the link to be opened (Ctrl-C to give up)…\n",
            grant.user_code
        ));
        let _ = self.out.flush();
        let mut credentials = account::wait_for_approval(&grant)?;
        self.say(&format!(
            "Signed in as {} ({}).\n",
            credentials.email, credentials.issuer
        ));
        if account::whoami(&mut credentials)?.is_none() {
            self.choose_handle(&mut credentials)?;
        }
        Ok(())
    }

    /// Prompt for the account's handle — the namespace its work
    /// will live under. Skippable; retried on refusals so the
    /// reader sees the platform's reason (taken, reserved, too
    /// short for a free account) and can try again.
    fn choose_handle(
        &mut self,
        credentials: &mut account::Credentials,
    ) -> Result<()> {
        self.say(
            "\nYour account has no handle yet — the namespace your\n\
             work will live under. Choose one (or press enter to\n\
             decide later with `lit account handle`).\n",
        );
        for _ in 0..5 {
            let handle = self.prompt("handle: ")?;
            if handle.is_empty() {
                self.say("Skipped — set it later with `lit account handle`.\n");
                return Ok(());
            }
            match account::set_handle(credentials, &handle) {
                Ok(()) => {
                    self.say(&format!("Handle set: {handle}\n"));
                    return Ok(());
                }
                Err(e) => self.say(&format!("  {}\n", e.0)),
            }
        }
        self.say("Leaving the handle unset — `lit account handle` any time.\n");
        Ok(())
    }

    fn cmd_account_handle(&mut self, argv: &mut Vec<String>) -> Result<()> {
        if argv.len() > 1 {
            err!("usage: lit account handle [<name>]");
        }
        let mut credentials = match account::load()? {
            Some(c) => c,
            None => err!("not signed in — run `lit account login` first"),
        };
        match argv.pop() {
            Some(handle) => {
                account::set_handle(&mut credentials, &handle)?;
                self.say(&format!("Handle set: {handle}\n"));
            }
            None => match account::whoami(&mut credentials)? {
                Some(handle) => self.say(&format!("Handle: {handle}\n")),
                None => self.choose_handle(&mut credentials)?,
            },
        }
        Ok(())
    }

    fn cmd_account_status(&mut self, argv: &mut Vec<String>) -> Result<()> {
        if !argv.is_empty() {
            err!("account status takes no arguments");
        }
        match account::load()? {
            Some(credentials) => {
                let handle = match &credentials.handle {
                    Some(h) => format!(", handle {h}"),
                    None => ", no handle yet".to_string(),
                };
                self.say(&format!(
                    "Signed in as {}{} ({}).\n",
                    credentials.email, handle, credentials.issuer
                ));
            }
            None => self.say("Not signed in. Run `lit account login`.\n"),
        }
        Ok(())
    }

    fn cmd_account_logout(&mut self, argv: &mut Vec<String>) -> Result<()> {
        if !argv.is_empty() {
            err!("account logout takes no arguments");
        }
        match account::load()? {
            Some(credentials) => {
                // Best-effort server-side revocation; the local
                // erase is what logout means.
                account::revoke(&credentials);
                account::erase()?;
                self.say(&format!("Signed out {}.\n", credentials.email));
            }
            None => self.say("Not signed in.\n"),
        }
        Ok(())
    }

    // ── Plumbing ────────────────────────────────────────────────

    fn say(&mut self, text: &str) {
        let _ = self.out.write_all(text.as_bytes());
    }

    fn errsay(&mut self, text: &str) {
        let _ = self.err.write_all(text.as_bytes());
    }

    fn prompt(&mut self, question: &str) -> Result<String> {
        self.say(question);
        let _ = self.out.flush();
        let mut line = String::new();
        let n = self.input.read_line(&mut line)?;
        if n == 0 {
            err!("interactive input required and none available");
        }
        Ok(line
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .to_string())
    }

    pub(crate) fn confirm(&mut self, question: &str, yes: bool) -> Result<bool> {
        if yes {
            return Ok(true);
        }
        let answer = self.prompt(&format!("{question} [y/N] "))?;
        Ok(answer == "y" || answer == "Y" || answer == "yes" || answer == "Yes")
    }

    pub(crate) fn note(&mut self, text: &str) {
        self.say(text);
    }

    fn repo(&self) -> Result<Repository> {
        let Some(root) = Repository::discover(&self.cwd) else {
            err!("not inside a literium repository");
        };
        Repository::new(root)
    }

    // ── Side assembly (shapes for detect/diff) ──────────────────

    fn side_of_scan(&self, scan: &Scan) -> SideInput {
        let mut states = HashMap::new();
        for (path, vh) in &scan.manifest {
            states.insert(path.clone(), scan.docs[vh].clone());
        }
        SideInput {
            manifest: scan.manifest.clone(),
            states,
        }
    }

    fn side_of_version(&self, repo: &Repository, vo: Option<&VersionObject>) -> Result<SideInput> {
        let Some(vo) = vo else {
            return Ok(SideInput {
                manifest: BTreeMap::new(),
                states: HashMap::new(),
            });
        };
        Ok(SideInput {
            manifest: vo.manifest().clone(),
            states: repo.doc_states(vo)?,
        })
    }

    // Resolution per SPEC §8.3: syntactic. 64 lowercase hex =
    // version hash; 'historia' = the historia's latest; otherwise
    // an experiment's latest. Short unique version-hash prefixes
    // (>= 6 hex chars) are accepted (PROVISIONAL.md #8/#11) — but
    // an existing history name wins over the prefix reading.
    fn resolve_version(&self, repo: &Repository, arg: &str) -> Result<String> {
        if arg.is_empty() {
            err!("empty source");
        }
        if is_hex(arg) && arg.len() == 64 {
            if !repo.theca().has("version", arg)? {
                err!("no such version {arg}");
            }
            return Ok(arg.to_string());
        }
        if is_hex(arg) && arg.len() >= 6 && arg.len() <= 63 && repo.history(arg).is_none() {
            let hits: Vec<String> = repo
                .theca()
                .list("version")?
                .into_iter()
                .filter(|h| h.starts_with(arg))
                .collect();
            if hits.is_empty() {
                err!("no version matches prefix {arg}");
            }
            if hits.len() > 1 {
                err!("ambiguous version prefix {arg}");
            }
            return Ok(hits.into_iter().next().unwrap());
        }
        let Some(history) = repo.history(arg) else {
            err!("no history named '{arg}'");
        };
        let Some(latest) = history.latest()? else {
            err!("history '{arg}' has no inscriptiones");
        };
        Ok(latest.vhash)
    }

    fn target_history(&self, repo: &Repository, name: Option<&str>) -> Result<History> {
        let name = name.unwrap_or("historia");
        // SPEC §8.3: the target must already exist; inscribe never
        // creates an experiment as a side effect.
        let Some(history) = repo.history(name) else {
            err!(
                "no history named '{name}' (experiments are created with 'lit experiment create')"
            );
        };
        Ok(history)
    }

    // ── conscribe (SPEC §8.2, §1.3, §1.4) ───────────────────────

    fn cmd_conscribe(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let mut mode: Option<String> = None;
        let mut yes = false;
        parse_opts(argv, |opt, take| match opt {
            "--mode" => {
                mode = Some(take()?);
                Ok(true)
            }
            "--yes" => {
                yes = true;
                Ok(true)
            }
            _ => Ok(false),
        })
        .map_err(|_| Error::new("bad conscribe options"))?;
        if !argv.is_empty() {
            err!("conscribe takes no positional arguments");
        }

        let root = Repository::discover(&self.cwd);
        let repo = match root {
            None => match self.conscribe_dispatch_no_repo(mode.as_deref(), yes)? {
                Some(repo) => repo,
                None => return Ok(()), // elevation/merge path already reported
            },
            Some(root) => Repository::new(root)?,
        };

        repo.assert_segmentation()?;
        let scan = repo.scan()?;
        // SPEC §8.2: an empty document set is NOT refused here — a
        // conscribe recording the deletion of the repository's
        // last documents is valid (zero-trio version). The
        // no-documents refusal belongs only to the §1.3 creation
        // path; an empty tree with an empty baseline falls to the
        // no-pending-changes refusal below.

        let tree = self.side_of_scan(&scan);

        // SPEC §8.2 refusal 1: no pending changes against the
        // latest historia version (or the empty set on an empty
        // historia).
        let tip = repo.historia().latest()?;
        let tip_vo = match &tip {
            Some(t) => repo.version(&t.vhash)?,
            None => None,
        };
        let baseline = self.side_of_version(&repo, tip_vo.as_ref())?;
        if states_equal(&tree, &baseline) {
            err!("nothing to conscribe: no pending changes against historia");
        }

        // SPEC §8.2 refusal 2: tabula already holds this state.
        if let Some(tab) = repo.read_tabula()? {
            if tab.manifest() == &tree.manifest {
                err!("nothing to conscribe: the tabula already holds this state");
            }
            // SPEC §8.2: the previous tabula state is discarded,
            // never persisted; surface the replacement.
            self.say("replacing the previous tabula state\n");
        }

        let (trios, manifest) = repo.store_scan(&scan)?;
        repo.write_tabula(&VersionObject::new(
            &repo.segmentation_version(),
            trios,
            manifest,
        )?)?;
        let n = scan.manifest.len();
        self.say(&format!(
            "conscribed {n} document{} onto the tabula\n",
            if n == 1 { "" } else { "s" }
        ));
        Ok(())
    }

    fn conscribe_dispatch_no_repo(
        &mut self,
        mode: Option<&str>,
        yes: bool,
    ) -> Result<Option<Repository>> {
        let cwd = self.cwd.clone();

        // SPEC §1.4: scan the subtree for descendant repositories.
        let descendants = Repository::find_descendant_lits(&cwd);
        if descendants.len() >= 2 {
            err!(
                "found {} descendant repositories ({}); project merge is not implemented in this version",
                descendants.len(),
                descendants
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if descendants.len() == 1 {
            return elevate::elevate(self, &descendants[0], yes);
        }

        // SPEC §1.3: creation is refused when the tree contains no
        // documents — checked BEFORE anything is written (the
        // attributes file would otherwise make the tree non-empty
        // by itself).
        let probe = Repository::probe(&cwd)?;
        if probe.manifest.is_empty() {
            err!("refusing to create a repository: no documents here (SPEC §1.3)");
        }

        // SPEC §1.3 creation path: confirm the default mode for
        // the attributes file; --mode is the non-interactive
        // selector.
        let mode = match mode {
            Some(m) => m.to_string(),
            None => {
                let answer = self.prompt(
                    "no repository found; create one here?\ndefault mode for the attributes file [prose]: ",
                )?;
                let answer = if answer.is_empty()
                    || answer == "y"
                    || answer == "Y"
                    || answer == "yes"
                    || answer == "Yes"
                {
                    "prose".to_string()
                } else {
                    answer
                };
                if answer != "prose" && answer != "poetry" {
                    err!("mode must be 'prose' or 'poetry'");
                }
                answer
            }
        };
        if mode != "prose" && mode != "poetry" {
            err!("mode must be 'prose' or 'poetry'");
        }

        let repo = Repository::create(&cwd, &mode)?;
        self.say(&format!(
            "created literium repository in {}\n",
            cwd.display()
        ));
        Ok(Some(repo))
    }

    // ── status (SPEC §8.5, §10 #13) ─────────────────────────────

    fn cmd_status(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let target_name = if argv.is_empty() {
            None
        } else {
            Some(argv.remove(0))
        };
        if !argv.is_empty() {
            err!("status takes at most one target-history argument");
        }

        let repo = self.repo()?;
        repo.assert_segmentation()?;
        let target = self.target_history(&repo, target_name.as_deref())?;

        if let Some(ledger) = repo.read_contextio()? {
            self.say(&format!(
                "unresolved contextio conflicts: {} (see lit conflicts)\n",
                ledger.conflicts.len()
            ));
        }

        if repo.read_tabula()?.is_none() {
            self.say("tabula: empty (nothing conscribed)\n");
            return Ok(());
        }

        let (_, pending, _, _) = self.pending_against(&repo, &target)?;
        if pending.revisions.is_empty() {
            self.say(&format!("no pending revisions against {}\n", target.name()));
            return Ok(());
        }
        self.say(&format!("pending revisions against {}:\n", target.name()));
        for rev in &pending.revisions {
            self.say(&format!("  {}\n", revision_line(rev)));
        }
        Ok(())
    }

    // SPEC §8.5 parent binding: the latest version on the target.
    #[allow(clippy::type_complexity)]
    fn pending_against(
        &self,
        repo: &Repository,
        target: &History,
    ) -> Result<(Option<VersionObject>, Pending, SideInput, SideInput)> {
        let Some(tab) = repo.read_tabula()? else {
            err!("the tabula is empty; run lit conscribe first");
        };

        let tip = target.latest()?;
        let parent_vo = match &tip {
            Some(t) => repo.version(&t.vhash)?,
            None => None,
        };
        let parent_side = self.side_of_version(repo, parent_vo.as_ref())?;
        let tabula_side = self.side_of_version(repo, Some(&tab))?;

        let pending = detect(&parent_side, &tabula_side)?;
        Ok((parent_vo, pending, parent_side, tabula_side))
    }

    // ── inscribe (SPEC §8.3, §8.6, §8.7) ────────────────────────

    fn cmd_inscribe(&mut self, argv: &mut Vec<String>) -> Result<()> {
        // -<n> is not option-parser-friendly; extract it first.
        let mut revnum: Option<usize> = None;
        argv.retain(|a| {
            if let Some(n) = a
                .strip_prefix('-')
                .filter(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
            {
                revnum = Some(n.parse().unwrap());
                false
            } else {
                true
            }
        });

        let mut all = false;
        let mut message: Option<String> = None;
        let mut fons: Option<String> = None;
        parse_opts(argv, |opt, take| match opt {
            "--all" => {
                all = true;
                Ok(true)
            }
            "-m" | "--message" => {
                message = Some(take()?);
                Ok(true)
            }
            "-f" | "--fons" | "--from" => {
                fons = Some(take()?);
                Ok(true)
            }
            _ => Ok(false),
        })
        .map_err(|_| Error::new("bad inscribe options"))?;
        let target_name = if argv.is_empty() {
            None
        } else {
            Some(argv.remove(0))
        };
        if !argv.is_empty() {
            err!("inscribe takes at most one target argument");
        }

        let repo = self.repo()?;
        let target = self.target_history(&repo, target_name.as_deref())?;

        // SPEC §9.6: prompt before inscribing over unresolved
        // contextio conflicts.
        if let Some(ledger) = repo.read_contextio()? {
            if !self.confirm(
                &format!(
                    "the contextio ledger has {} unresolved conflicts; inscribe anyway?",
                    ledger.conflicts.len()
                ),
                false,
            )? {
                err!("inscription declined");
            }
        }

        // ── --fons: adopt a whole snapshot (SPEC §8.3, §9.4) ────
        if let Some(fons) = fons {
            if all {
                err!("--all does not apply with --fons (the source names one version)");
            }
            if revnum.is_some() {
                err!("-<n> does not apply with --fons");
            }
            let vhash = self.resolve_version(&repo, &fons)?;
            let src_label = if is_hex(&fons) && fons.len() >= 6 && fons.len() <= 64 {
                vhash[..12].to_string()
            } else {
                fons.clone()
            };
            self.say(&format!("inscribe: {src_label} -> {}\n", target.name()));
            let message = match message {
                Some(m) => m,
                None => self.prompt("message: ")?,
            };
            target.append(&vhash, &message, None)?;
            self.say(&format!(
                "inscribed {} onto {}\n",
                &vhash[..12],
                target.name()
            ));
            return Ok(()); // tabula unchanged (SPEC §8.3)
        }

        // ── tabula source ───────────────────────────────────────
        repo.assert_segmentation()?;
        let (parent_vo, pending, parent_side, tabula_side) =
            self.pending_against(&repo, &target)?;
        if pending.revisions.is_empty() {
            err!("no pending revisions against {}", target.name());
        }

        self.say(&format!("inscribe: tabula -> {}\n", target.name()));

        if all {
            // SPEC §8.6 opt-in: one version covering everything.
            let message = match message {
                Some(m) => m,
                None => self.prompt("message: ")?,
            };
            let subset: HashSet<String> = pending.revisions.iter().map(|r| r.id.clone()).collect();
            let vhash = Self::inscribe_subset(
                &repo,
                &target,
                parent_vo.as_ref(),
                &parent_side,
                &tabula_side,
                &pending,
                &subset,
                &message,
            )?;
            repo.clear_tabula()?; // consumed the last pending revision
            repo.clear_contextio()?; // SPEC §9.6 ledger lifecycle
            self.say(&format!(
                "inscribed {} onto {} (all pending revisions)\n",
                &vhash[..12],
                target.name()
            ));
            return Ok(());
        }

        let mut revnum = revnum;
        if revnum.is_none() && message.is_some() {
            // SPEC §8.7: bare -m is accepted only with exactly one
            // pending revision.
            if pending.revisions.len() != 1 {
                err!("several revisions are pending; use -<n>, --all, or the editor form");
            }
            revnum = Some(1);
        }

        if let Some(revnum) = revnum {
            let Some(rev) = pending.revisions.iter().find(|r| r.index == revnum) else {
                err!(
                    "no pending revision {revnum} (1-{})",
                    pending.revisions.len()
                );
            };
            // PROVISIONAL.md #5: echo the resolved revision before
            // committing, so stale indices are caught.
            self.say(&format!("revision {}\n", revision_line(rev)));
            let message = match message {
                Some(m) => m,
                None => self.prompt("message: ")?,
            };
            let subset: HashSet<String> = HashSet::from([rev.id.clone()]);
            let vhash = Self::inscribe_subset(
                &repo,
                &target,
                parent_vo.as_ref(),
                &parent_side,
                &tabula_side,
                &pending,
                &subset,
                &message,
            )?;
            if pending.revisions.len() == 1 {
                repo.clear_tabula()?; // SPEC §8.3
                repo.clear_contextio()?; // SPEC §9.6 ledger lifecycle
            }
            self.say(&format!(
                "inscribed {} onto {}\n",
                &vhash[..12],
                target.name()
            ));
            return Ok(());
        }

        // Editor form (SPEC §8.7): all pending revisions, one
        // message line each; unmessaged revisions stay pending.
        let selection = self.editor_messages(&pending.revisions)?;
        if selection.is_empty() {
            err!("no messages supplied; nothing inscribed");
        }
        let mut subset: HashSet<String> = HashSet::new();
        for (rev_index, msg) in &selection {
            let rev = pending
                .revisions
                .iter()
                .find(|r| r.index == *rev_index)
                .expect("selection index from revisions");
            subset.insert(rev.id.clone());
            let vhash = Self::inscribe_subset(
                &repo,
                &target,
                parent_vo.as_ref(),
                &parent_side,
                &tabula_side,
                &pending,
                &subset,
                msg,
            )?;
            self.say(&format!(
                "inscribed {} (revision {})\n",
                &vhash[..12],
                rev.index
            ));
        }
        if selection.len() == pending.revisions.len() {
            repo.clear_tabula()?;
            repo.clear_contextio()?; // SPEC §9.6 ledger lifecycle
        }
        Ok(())
    }

    // The cumulative builder: construct (and inscribe) the version
    // for a subset of the pending revisions. Per-revision versions
    // are cumulative (SPEC §8.6); the subset-based splice makes
    // the construction order-independent, so each call passes the
    // full subset inscribed so far.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn inscribe_subset(
        repo: &Repository,
        target: &History,
        parent_vo: Option<&VersionObject>,
        parent_side: &SideInput,
        tabula_side: &SideInput,
        pending: &Pending,
        subset: &HashSet<String>,
        message: &str,
    ) -> Result<String> {
        let mut manifest: BTreeMap<String, String> = match parent_vo {
            Some(vo) => vo.manifest().clone(),
            None => BTreeMap::new(),
        };
        let mut bytes: HashMap<String, Vec<u8>> = HashMap::new();
        let mut renamed: BTreeMap<String, String> = BTreeMap::new();
        let mut spliced: HashSet<String> = HashSet::new();

        for rev in pending.revisions.iter().filter(|r| subset.contains(&r.id)) {
            match rev.kind {
                RevKind::Edit | RevKind::Formatting => {
                    spliced.insert(rev.doc.clone().expect("edit revision has doc"));
                }
                RevKind::Removal => {
                    manifest.remove(&rev.path);
                }
                RevKind::FirstVersion => {
                    manifest.insert(rev.path.clone(), "pending".to_string());
                    bytes.insert(
                        rev.path.clone(),
                        tabula_side.states[&rev.path].volumen.clone(),
                    );
                }
                RevKind::Rename => {
                    let new_path = rev.new_path.clone().expect("rename has new_path");
                    if let Some(vh) = manifest.remove(&rev.path) {
                        manifest.insert(new_path.clone(), vh);
                    }
                    renamed.insert(rev.path.clone(), new_path);
                }
            }
        }
        for path in &spliced {
            bytes.insert(path.clone(), volumen_for(&pending.docs[path], subset));
        }
        let renamed_from: HashMap<&String, &String> = renamed.iter().map(|(o, n)| (n, o)).collect();
        for path in manifest.keys() {
            if bytes.contains_key(path) {
                continue;
            }
            // A renamed path's bytes live under the former path.
            let src = if parent_side.states.contains_key(path) {
                Some(path)
            } else {
                renamed_from.get(path).copied()
            };
            let Some(state) = src.and_then(|s| parent_side.states.get(s)) else {
                err!("internal: no bytes for '{path}'");
            };
            bytes.insert(path.clone(), state.volumen.clone());
        }

        // Re-derive every document with modes from the NEW state's
        // own attributes document (SPEC §1.1) — this is what makes
        // a mode change re-segment other documents with unchanged
        // bytes.
        let attr_content = match bytes.get(crate::ATTRIBUTES_FILE) {
            Some(b) => String::from_utf8(b.clone())
                .map_err(|_| Error::new("attributes bytes are not UTF-8"))?,
            None => String::new(),
        };
        let attrs = Attributes::parse(&attr_content)?;

        let mut trios: BTreeMap<String, (String, String)> = BTreeMap::new();
        let mut new_manifest: BTreeMap<String, String> = BTreeMap::new();
        let theca = repo.theca();
        for path in manifest.keys() {
            let Some(d) = derive(&bytes[path], attrs.mode_for(path)) else {
                err!("internal: spliced volumen for '{path}' is not a document");
            };
            new_manifest.insert(path.clone(), d.volumen_hash.clone());
            trios.insert(
                d.volumen_hash.clone(),
                (d.spina_hash.clone(), d.atom_spina_hash.clone()),
            );
            theca.put("volumen", &d.volumen)?;
            for b in d.atoms.values() {
                theca.put("atom", b)?;
            }
            for b in d.fasciculi.values() {
                theca.put("fasciculus", b)?;
            }
            theca.put("spina", &d.spina)?;
            theca.put("atom-spina", &d.atom_spina)?;
        }

        let vo = VersionObject::new(&repo.segmentation_version(), trios, new_manifest.clone())?;

        let mut message = message.to_string();
        if !renamed.is_empty() {
            let note = renamed
                .iter()
                .map(|(o, n)| format!("{o} -> {n}"))
                .collect::<Vec<_>>()
                .join(", ");
            message = if message.is_empty() {
                format!("renamed: {note}")
            } else {
                format!("{message}\n(renamed: {note})")
            };
        }

        // SPEC §8.5: a pure rename re-inscribes the SAME version
        // hash with the manifest updated in place.
        if let Some(parent_vo) = parent_vo {
            if vo.vhash() == parent_vo.vhash() {
                let mut pvo = parent_vo.clone();
                pvo.rewrite_manifest(&theca, new_manifest)?;
                target.append(&pvo.vhash(), &message, None)?;
                return Ok(pvo.vhash());
            }
        }

        let vhash = vo.store(&theca)?;
        target.append(&vhash, &message, None)?;
        Ok(vhash)
    }

    // Editor form: compose the buffer, run $EDITOR, parse
    // messages. Returns (revision index, message) pairs in
    // document order.
    fn editor_messages(&mut self, revs: &[Revision]) -> Result<Vec<(usize, String)>> {
        let editor = std::env::var("EDITOR")
            .or_else(|_| std::env::var("VISUAL"))
            .map_err(|_| Error::new("no $EDITOR set; use -<n>, --all, or -m"))?;

        let mut buffer = String::from(
            "# inscribe: each revision below becomes one version.\n\
             # Write a message on its 'message:' line; revisions left\n\
             # without a message stay pending. '#' lines are ignored.\n",
        );
        for rev in revs {
            buffer.push_str(&format!("\n[{}] {}\n", rev.index, revision_line(rev)));
            buffer.push_str("message:\n");
        }

        let tmp = std::env::temp_dir().join(format!(
            "lit-inscribe-{}-{}.txt",
            std::process::id(),
            revs.len()
        ));
        std::fs::write(&tmp, buffer.as_bytes())?;

        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{editor} '{}'", tmp.display()))
            .status()
            .map_err(|e| Error::new(format!("editor '{editor}' failed: {e}")))?;
        if !status.success() {
            let _ = std::fs::remove_file(&tmp);
            err!("editor '{editor}' failed");
        }

        let edited = std::fs::read(&tmp)
            .map_err(|e| Error::new(format!("cannot reread editor buffer: {e}")))?;
        let _ = std::fs::remove_file(&tmp);
        let edited =
            String::from_utf8(edited).map_err(|_| Error::new("editor buffer is not UTF-8"))?;

        let mut msg: HashMap<usize, String> = HashMap::new();
        let mut current: Option<usize> = None;
        for line in edited.split('\n') {
            if line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                if let Some(end) = rest.find(']') {
                    if let Ok(n) = rest[..end].parse::<usize>() {
                        current = Some(n);
                        continue;
                    }
                }
            }
            if let Some(rest) = line.strip_prefix("message:") {
                if let Some(cur) = current {
                    let text = rest.trim_start();
                    if !text.is_empty() {
                        msg.insert(cur, text.to_string());
                    }
                    current = None;
                }
            }
        }
        Ok(revs
            .iter()
            .filter_map(|r| msg.get(&r.index).map(|m| (r.index, m.clone())))
            .collect())
    }

    // ── exscribe (SPEC §8.9) ────────────────────────────────────

    fn cmd_exscribe(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let mut force = false;
        parse_opts(argv, |opt, _take| match opt {
            "--force" => {
                force = true;
                Ok(true)
            }
            _ => Ok(false),
        })
        .map_err(|_| Error::new("bad exscribe options"))?;
        if argv.is_empty() {
            err!("exscribe needs a source (version hash or history name)");
        }
        let fons = argv.remove(0);
        let only_path = if argv.is_empty() {
            None
        } else {
            Some(argv.remove(0))
        };
        if !argv.is_empty() {
            err!("exscribe takes at most a source and one path");
        }

        let repo = self.repo()?;
        let vhash = self.resolve_version(&repo, &fons)?;
        let Some(vo) = repo.version(&vhash)? else {
            err!("no such version {vhash}");
        };

        // Single-document restore: only that document's bytes.
        if let Some(only_path) = only_path {
            let Some(doc) = vo.document_at(&only_path) else {
                err!("version {} has no document at '{only_path}'", &vhash[..12]);
            };
            let bytes = repo.theca().get("volumen", doc)?;
            self.write_doc(&repo, &only_path, bytes)?;
            self.say(&format!("exscribed {only_path} from {}\n", &vhash[..12]));
            return Ok(());
        }

        // Dirty-tree guard (SPEC §8.9), shared with collate.
        let scan = self.dirty_tree_guard(&repo, force)?;
        let tree = self.side_of_scan(&scan);

        // Materialize; then document-set fidelity: documents
        // present in the tree but absent from the version are
        // deleted.
        for path in vo.paths().cloned().collect::<Vec<_>>() {
            let doc = vo.document_at(&path).expect("path from manifest");
            let bytes = repo.theca().get("volumen", doc)?;
            self.write_doc(&repo, &path, bytes)?;
        }
        let mut deleted = 0;
        for path in tree.manifest.keys() {
            if vo.document_at(path).is_some() {
                continue;
            }
            if std::fs::remove_file(repo.root().join(path)).is_ok() {
                deleted += 1;
            }
        }
        let n = vo.paths().count();
        self.say(&format!(
            "exscribed {n} document{} from {}{}\n",
            if n == 1 { "" } else { "s" },
            &vhash[..12],
            if deleted > 0 {
                format!(" ({deleted} deleted)")
            } else {
                String::new()
            }
        ));
        Ok(())
    }

    /// SPEC §8.9 dirty-tree guard, shared by exscribe and
    /// collate: prompt when the tree differs from BOTH the
    /// latest historia version and the tabula state. Returns
    /// the scan for the caller's fidelity pass.
    fn dirty_tree_guard(&mut self, repo: &Repository, force: bool) -> Result<Scan> {
        let scan = repo.scan()?;
        let tree = self.side_of_scan(&scan);
        let mut dirty = true;
        match repo.historia().latest()? {
            Some(tip) => {
                let tip_vo = repo.version(&tip.vhash)?;
                if states_equal(&tree, &self.side_of_version(repo, tip_vo.as_ref())?) {
                    dirty = false;
                }
            }
            None => {
                if tree.manifest.is_empty() {
                    dirty = false;
                }
            }
        }
        if dirty {
            if let Some(tab) = repo.read_tabula()? {
                if tab.manifest() == &tree.manifest {
                    dirty = false;
                }
            }
        }
        if dirty && !self.confirm("the working tree has unrecorded changes; overwrite?", force)? {
            err!("working tree has unrecorded changes; not overwriting");
        }
        Ok(scan)
    }

    // ── the exemplar exchange (SPEC §9.7) ───────────────────────

    fn cmd_exemplar(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let mut out: Option<String> = None;
        parse_opts(argv, |opt, take| match opt {
            "-o" | "--out" => {
                out = Some(take()?);
                Ok(true)
            }
            _ => Ok(false),
        })
        .map_err(|_| Error::new("bad exemplar options"))?;
        if argv.len() > 1 {
            err!("usage: lit exemplar [<source>] [-o <file>]");
        }
        if argv.is_empty() {
            return self.exemplar_repack(out);
        }

        let fons = argv.remove(0);
        let repo = self.repo()?;
        repo.assert_segmentation()?;
        let vhash = self.resolve_version(&repo, &fons)?;
        let Some(vo) = repo.version(&vhash)? else {
            err!("no such version {vhash}");
        };
        let mut docs = Vec::new();
        for path in vo.paths().cloned().collect::<Vec<_>>() {
            let doc = vo.document_at(&path).expect("path from manifest");
            let Some(bytes) = repo.theca().get("volumen", doc)? else {
                err!("missing volumen for {path}");
            };
            docs.push((path, bytes));
        }
        let metadata = exemplar::Metadata {
            basis: vhash.clone(),
            segmentatio: repo.segmentation_version(),
        };
        let out = out.unwrap_or_else(|| format!("exemplar-{}.tgz", &vhash[..12]));
        exemplar::write_archive(&self.cwd.join(&out), &metadata, &docs)?;
        self.say(&format!(
            "packed {} document{} into {out} (basis {})\n",
            docs.len(),
            if docs.len() == 1 { "" } else { "s" },
            &vhash[..12]
        ));
        Ok(())
    }

    /// The recipient's return trip: repack the current directory
    /// (an unpacked exemplar) with its `.exemplar` verbatim.
    /// Only `.exemplar` itself, the output archive, and a
    /// `.lit/` are excluded — dot-led documents such as
    /// `.litattributes` are ordinary manifest entries and must
    /// ship.
    fn exemplar_repack(&mut self, out: Option<String>) -> Result<()> {
        let meta_path = self.cwd.join(exemplar::METADATA_NAME);
        let Ok(text) = std::fs::read_to_string(&meta_path) else {
            err!("no <source> given and no .exemplar here — nothing to pack");
        };
        let metadata = exemplar::Metadata::parse(&text)?;
        let out = out.unwrap_or_else(|| format!("exemplar-{}.tgz", &metadata.basis[..12]));
        let mut docs = Vec::new();
        collect_files(&self.cwd, "", &mut docs)?;
        docs.retain(|(path, _)| path != &out && path != exemplar::METADATA_NAME);
        exemplar::write_archive(&self.cwd.join(&out), &metadata, &docs)?;
        self.say(&format!(
            "repacked {} document{} into {out} (basis {})\n",
            docs.len(),
            if docs.len() == 1 { "" } else { "s" },
            &metadata.basis[..12]
        ));
        Ok(())
    }

    fn cmd_collate(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let mut force = false;
        parse_opts(argv, |opt, _take| match opt {
            "--force" => {
                force = true;
                Ok(true)
            }
            _ => Ok(false),
        })
        .map_err(|_| Error::new("bad collate options"))?;
        if argv.len() != 1 {
            err!("usage: lit collate <file> [--force]");
        }
        let file = argv.remove(0);

        let repo = self.repo()?;
        let (metadata, docs) = exemplar::read_archive(&self.cwd.join(&file))?;
        if metadata.segmentatio != repo.segmentation_version() {
            self.say(&format!(
                "note: exemplar segmentation '{}' differs from this repository's '{}'\n",
                metadata.segmentatio,
                repo.segmentation_version()
            ));
        }

        let scan = self.dirty_tree_guard(&repo, force)?;
        let tree = self.side_of_scan(&scan);

        let mut names = std::collections::BTreeSet::new();
        for (path, bytes) in &docs {
            names.insert(path.clone());
            self.write_doc(&repo, path, Some(bytes.clone()))?;
        }
        // Document-set fidelity, as exscribe (§8.11): tree
        // documents absent from the archive are deleted.
        let mut deleted = 0;
        for path in tree.manifest.keys() {
            if names.contains(path) {
                continue;
            }
            if std::fs::remove_file(repo.root().join(path)).is_ok() {
                deleted += 1;
            }
        }

        let basis12 = &metadata.basis[..12];
        self.say(&format!(
            "collated {} document{}{} (basis {basis12})\n\
             review:  lit diff {basis12}\n\
             adopt:   lit conscribe, then lit inscribe — or weave with\n\
             \x20        lit contexe historia --base {basis12} if historia has moved\n",
            docs.len(),
            if docs.len() == 1 { "" } else { "s" },
            if deleted > 0 {
                format!(", {deleted} deleted")
            } else {
                String::new()
            }
        ));
        Ok(())
    }

    fn write_doc(&mut self, repo: &Repository, path: &str, bytes: Option<Vec<u8>>) -> Result<()> {
        let Some(bytes) = bytes else {
            err!("missing volumen for {path}");
        };
        let fs = repo.root().join(path);
        if let Some(dir) = fs.parent() {
            if !dir.is_dir() {
                std::fs::create_dir_all(dir)?;
            }
        }
        std::fs::write(&fs, bytes).map_err(|e| Error::new(format!("cannot write {path}: {e}")))
    }

    // ── contexe (SPEC §9.6): the combining merge ────────────────

    fn cmd_contexe(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let mut base_arg: Option<String> = None;
        let mut force = false;
        parse_opts(argv, |opt, take| match opt {
            "--base" => {
                base_arg = Some(take()?);
                Ok(true)
            }
            "--force" => {
                force = true;
                Ok(true)
            }
            _ => Ok(false),
        })
        .map_err(|_| Error::new("bad contexe options"))?;
        if argv.is_empty() {
            err!("contexe needs a source (version hash or history name)");
        }
        let source_arg = argv.remove(0);
        let target_name = if argv.is_empty() {
            None
        } else {
            Some(argv.remove(0))
        };
        if !argv.is_empty() {
            err!("contexe takes at most a source and a target");
        }

        let repo = self.repo()?;
        repo.assert_segmentation()?;
        let target = self.target_history(&repo, target_name.as_deref())?;

        // Resolve the source: an existing history name resolves to
        // its tip (and enables auto-base); otherwise a version.
        let source_history = repo.history(&source_arg);
        let (b_vhash, src_label) = match &source_history {
            Some(h) => {
                let Some(latest) = h.latest()? else {
                    err!("history '{source_arg}' has no inscriptiones");
                };
                (latest.vhash, source_arg.clone())
            }
            None => {
                let vhash = self.resolve_version(&repo, &source_arg)?;
                let label = vhash[..12].to_string();
                (vhash, label)
            }
        };

        // Base selection (SPEC §9.6): --base, else the latest
        // common version by target entry order, else baseless.
        let base_vhash: Option<String> = match &base_arg {
            Some(b) => Some(self.resolve_version(&repo, b)?),
            None => match &source_history {
                Some(sh) => {
                    let source_set: std::collections::HashSet<String> =
                        sh.entries()?.into_iter().map(|e| e.vhash).collect();
                    let mut best: Option<String> = None;
                    for e in target.entries()? {
                        if source_set.contains(&e.vhash) {
                            best = Some(e.vhash); // last occurrence wins
                        }
                    }
                    best
                }
                None => None,
            },
        };

        // Sides.
        let a_tip = target.latest()?;
        let a_vo = match &a_tip {
            Some(t) => repo.version(&t.vhash)?,
            None => None,
        };
        let a_side = self.side_of_version(&repo, a_vo.as_ref())?;
        let Some(b_vo) = repo.version(&b_vhash)? else {
            err!("no such version {b_vhash}");
        };
        let b_side = self.side_of_version(&repo, Some(&b_vo))?;
        let o_side = match &base_vhash {
            Some(vh) => {
                let Some(o_vo) = repo.version(vh)? else {
                    err!("no such base version {vh}");
                };
                Some(self.side_of_version(&repo, Some(&o_vo))?)
            }
            None => None,
        };

        // Dirty-tree guard, exactly as in exscribe (SPEC §8.9).
        let scan = repo.scan()?;
        let tree = self.side_of_scan(&scan);
        let mut dirty = true;
        match repo.historia().latest()? {
            Some(tip) => {
                let tip_vo = repo.version(&tip.vhash)?;
                if states_equal(&tree, &self.side_of_version(&repo, tip_vo.as_ref())?) {
                    dirty = false;
                }
            }
            None => {
                if tree.manifest.is_empty() {
                    dirty = false;
                }
            }
        }
        if dirty {
            if let Some(tab) = repo.read_tabula()? {
                if tab.manifest() == &tree.manifest {
                    dirty = false;
                }
            }
        }
        if dirty && !self.confirm("the working tree has unrecorded changes; overwrite?", force)? {
            err!("working tree has unrecorded changes; not overwriting");
        }

        // Combine and materialize.
        let outcome = crate::merge::combine(&a_side, &b_side, o_side.as_ref())?;
        for (path, bytes) in &outcome.documents {
            self.write_doc(&repo, path, Some(bytes.clone()))?;
        }
        let mut deleted = 0;
        for path in tree.manifest.keys() {
            if outcome.documents.contains_key(path) {
                continue;
            }
            if std::fs::remove_file(repo.root().join(path)).is_ok() {
                deleted += 1;
            }
        }

        // Ledger (SPEC §9.6): a fresh contexe replaces any prior
        // ledger; a clean one clears it.
        if outcome.conflicts.is_empty() {
            repo.clear_contextio()?;
        } else {
            repo.write_contextio(&crate::merge::Ledger {
                target_name: target.name().to_string(),
                target_vhash: a_tip.map(|t| t.vhash),
                source_label: src_label.clone(),
                source_vhash: b_vhash.clone(),
                base: base_vhash.clone(),
                conflicts: outcome.conflicts.clone(),
            })?;
        }

        // Summary.
        let base_label = match &base_vhash {
            Some(vh) => vh[..12].to_string(),
            None => "none".to_string(),
        };
        self.say(&format!(
            "contexe: {src_label} + {} (base {base_label})\n",
            target.name()
        ));
        let n = outcome.documents.len();
        self.say(&format!(
            "materialized {n} document{}{}\n",
            if n == 1 { "" } else { "s" },
            if deleted > 0 {
                format!(" ({deleted} deleted)")
            } else {
                String::new()
            }
        ));
        if !outcome.merged_paths.is_empty() {
            self.say(&format!(
                "content-merged: {}\n",
                outcome.merged_paths.join(", ")
            ));
        }
        if !outcome.conflicts.is_empty() {
            self.say(&format!(
                "{} conflict{} recorded in the contextio ledger (see lit conflicts)\n",
                outcome.conflicts.len(),
                if outcome.conflicts.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        Ok(())
    }

    // ── conflicts (SPEC §9.6 ledger surface) ────────────────────

    fn cmd_conflicts(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let sub = if argv.is_empty() {
            "list".to_string()
        } else {
            argv.remove(0)
        };
        if !argv.is_empty() || (sub != "list" && sub != "clear") {
            err!("usage: lit conflicts [clear]");
        }

        let repo = self.repo()?;
        let ledger = repo.read_contextio()?;
        if sub == "clear" {
            if ledger.is_none() {
                err!("the contextio ledger is already empty");
            }
            repo.clear_contextio()?;
            self.say("contextio ledger cleared\n");
            return Ok(());
        }
        let Some(ledger) = ledger else {
            self.say("no contextio conflicts\n");
            return Ok(());
        };
        let base_label = match &ledger.base {
            Some(vh) => vh[..12].to_string(),
            None => "none".to_string(),
        };
        self.say(&format!(
            "contexe {} -> {} (base {base_label}): {} unresolved conflict{}\n",
            ledger.source_label,
            ledger.target_name,
            ledger.conflicts.len(),
            if ledger.conflicts.len() == 1 { "" } else { "s" }
        ));
        let theca = repo.theca();
        let atom_text = |hash: &str| -> String {
            match theca.get("atom", hash) {
                Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
                _ => format!("<atom {hash} missing>"),
            }
        };
        for (i, c) in ledger.conflicts.iter().enumerate() {
            self.say(&format!(
                "{}. {} {} ({})\n",
                i + 1,
                c.path,
                c.coords,
                c.kind
            ));
            match c.kind.as_str() {
                "modify-delete" => self.say(&format!(
                    "   kept from {}; removed in {}\n",
                    ledger.target_name, ledger.source_label
                )),
                "delete-modify" => self.say(&format!(
                    "   removed in {}; kept from {}\n",
                    ledger.target_name, ledger.source_label
                )),
                _ => {
                    for h in &c.a_atoms {
                        self.say(&format!("   [{}] {}\n", ledger.target_name, atom_text(h)));
                    }
                    for h in &c.b_atoms {
                        self.say(&format!("   [{}] {}\n", ledger.source_label, atom_text(h)));
                    }
                }
            }
        }
        Ok(())
    }

    // ── tabula (SPEC §8.1 lifecycle) ────────────────────────────

    fn cmd_tabula(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let sub = if argv.is_empty() {
            "show".to_string()
        } else {
            argv.remove(0)
        };
        if !argv.is_empty() || (sub != "show" && sub != "discard") {
            err!("usage: lit tabula [discard]");
        }

        let repo = self.repo()?;
        let tab = repo.read_tabula()?;
        if sub == "discard" {
            if tab.is_none() {
                err!("the tabula is already empty");
            }
            repo.clear_tabula()?;
            self.say("tabula discarded\n");
            return Ok(());
        }
        let Some(tab) = tab else {
            self.say("tabula: empty\n");
            return Ok(());
        };
        let paths: Vec<&String> = tab.paths().collect();
        self.say(&format!(
            "tabula: {} document{}\n",
            paths.len(),
            if paths.len() == 1 { "" } else { "s" }
        ));
        for p in paths {
            self.say(&format!("  {p}\n"));
        }
        Ok(())
    }

    // ── experiment (SPEC §9.2, §10 #14) ─────────────────────────

    fn cmd_experiment(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let sub = if argv.is_empty() {
            String::new()
        } else {
            argv.remove(0)
        };
        let name = if argv.is_empty() {
            None
        } else {
            Some(argv.remove(0))
        };
        if !argv.is_empty() || !matches!(sub.as_str(), "create" | "delete" | "list") {
            err!("usage: lit experiment create|delete <name>  (or: lit experiment list)");
        }

        let repo = self.repo()?;
        if sub == "list" {
            for name in repo.experiments() {
                self.say(&format!("{name}\n"));
            }
            return Ok(());
        }
        let Some(name) = name else {
            err!("experiment {sub} needs a name");
        };
        if sub == "create" {
            repo.create_experiment(&name)?;
            self.say(&format!("created experiment '{name}'\n"));
        } else {
            repo.delete_experiment(&name)?;
            self.say(&format!(
                "deleted experiment '{name}' (its versions remain in the theca)\n"
            ));
        }
        Ok(())
    }

    // ── show (PROVISIONAL.md #8/#11 minimal read surface) ───────

    fn cmd_show(&mut self, argv: &mut Vec<String>) -> Result<()> {
        // Display bonds (SPEC §7.4 *Bonds*): --desm <file> / --no-desm.
        let bonds_choice = bonds::choice_from_args(argv, &self.cwd)?;
        let mut atoms_flag = false;
        let mut hash_flag = false;
        parse_opts(argv, |opt, _take| match opt {
            "--atoms" => {
                atoms_flag = true;
                Ok(true)
            }
            "--hash" => {
                hash_flag = true;
                Ok(true)
            }
            _ => Ok(false),
        })
        .map_err(|_| Error::new("bad show options"))?;
        let arg = if argv.is_empty() {
            None
        } else {
            Some(argv.remove(0))
        };
        let path = if argv.is_empty() {
            None
        } else {
            Some(argv.remove(0))
        };
        let coord_arg = if argv.is_empty() {
            None
        } else {
            Some(argv.remove(0))
        };
        if !argv.is_empty() {
            err!("usage: lit show [<history>|<version> [<path> [--atoms|<coord>]]]");
        }

        let repo = self.repo()?;

        let is_history = match &arg {
            None => true,
            Some(a) => path.is_none() && !atoms_flag && !hash_flag && repo.history(a).is_some(),
        };
        if is_history {
            let history = self.target_history(&repo, arg.as_deref())?;
            let entries = history.entries()?;
            if entries.is_empty() {
                self.say(&format!("{}: no inscriptiones yet\n", history.name()));
                return Ok(());
            }
            for (i, e) in entries.iter().enumerate() {
                let first = e.message.split('\n').next().unwrap_or("");
                self.say(&format!(
                    "{:3}  {}  {}  {}\n",
                    i + 1,
                    &e.vhash[..12],
                    e.timestamp,
                    first
                ));
            }
            return Ok(());
        }

        let arg = arg.expect("non-history branch has an argument");
        let vhash = self.resolve_version(&repo, &arg)?;
        let Some(vo) = repo.version(&vhash)? else {
            err!("no such version {vhash}");
        };

        if let Some(path) = path {
            let Some(doc) = vo.document_at(&path) else {
                err!("version {} has no document at '{path}'", &vhash[..12]);
            };
            let Some(bytes) = repo.theca().get("volumen", doc)? else {
                err!("volumen {doc} missing from theca");
            };

            if !atoms_flag && coord_arg.is_none() {
                let _ = self.out.write_all(&bytes); // raw volumen bytes
                return Ok(());
            }

            // SPEC §8.11: the citation surface needs the document's
            // derivation under its historical mode (SPEC §1.1).
            let attrs = repo.attributes_for_version(&vo)?;
            let Some(d) = crate::derive::derive(&bytes, attrs.mode_for(&path)) else {
                err!("stored volumen {doc} does not derive as a document");
            };

            // Display bonds (SPEC §7.4 *Bonds*): the working tree's
            // nearest .desm, prose documents only; the listing then
            // shows molecules with coordinate ranges.
            let mut bonds_of = Bonds::new(repo.root(), bonds_choice);
            let bonds = if d.mode == "prose" {
                bonds_of.for_path(&path)?
            } else {
                None
            };

            if atoms_flag {
                if coord_arg.is_some() {
                    err!("--atoms does not take a coordinate");
                }
                // aK gNaM  <content form>, one atom (or molecule:
                // aK..aL gNaM..gNaP) per line.
                let mut k = 0usize;
                for (g, grex) in d.grexes.iter().enumerate() {
                    let atoms: Vec<&str> = grex.iter().map(String::as_str).collect();
                    for (first, last) in bonds::molecules(&atoms, bonds.as_deref()) {
                        let k0 = k + 1;
                        k += last - first + 1;
                        self.say(&format!(
                            "{} {}  {}\n",
                            bonds::flat_range(k0, k),
                            bonds::coord_range(g, first, last),
                            bonds::molecule_text(&atoms, first, last)
                        ));
                    }
                }
                return Ok(());
            }

            let coord = coord_arg.expect("coord branch");
            match parse_coord(&coord) {
                Some(Coord::Grex(gn)) => {
                    if gn == 0 || gn > d.grexes.len() {
                        err!(
                            "no grex g{gn} in {path} (document has {} grexes)",
                            d.grexes.len()
                        );
                    }
                    if hash_flag {
                        self.say(&format!("{}\n", d.fasc_hashes[gn - 1]));
                    } else {
                        let atoms: Vec<&str> = d.grexes[gn - 1].iter().map(String::as_str).collect();
                        for (first, last) in bonds::molecules(&atoms, bonds.as_deref()) {
                            self.say(&format!("{}\n", bonds::molecule_text(&atoms, first, last)));
                        }
                    }
                }
                Some(Coord::GrexAtom(gn, an)) => {
                    if gn == 0 || gn > d.grexes.len() {
                        err!(
                            "no grex g{gn} in {path} (document has {} grexes)",
                            d.grexes.len()
                        );
                    }
                    let grex = &d.grexes[gn - 1];
                    if an == 0 || an > grex.len() {
                        err!(
                            "no atom g{gn}a{an} in {path} (grex g{gn} has {} atoms)",
                            grex.len()
                        );
                    }
                    let atom = &grex[an - 1];
                    if hash_flag {
                        self.say(&format!("{}\n", crate::derive::hash_bytes(atom.as_bytes())));
                    } else {
                        self.say(&format!("{atom}\n"));
                    }
                }
                Some(Coord::Flat(k)) => {
                    let flat: Vec<&String> = d.grexes.iter().flatten().collect();
                    if k == 0 || k > flat.len() {
                        err!("no atom a{k} in {path} (document has {} atoms)", flat.len());
                    }
                    let atom = flat[k - 1];
                    if hash_flag {
                        self.say(&format!("{}\n", crate::derive::hash_bytes(atom.as_bytes())));
                    } else {
                        self.say(&format!("{atom}\n"));
                    }
                }
                None => err!("bad coordinate '{coord}' (use gN, gNaM, or aK)"),
            }
            return Ok(());
        }
        if atoms_flag || hash_flag || coord_arg.is_some() {
            err!("--atoms, --hash, and coordinates need a <path>");
        }

        self.say(&format!("version {vhash}\n"));
        self.say(&format!("segmentation {}\n", vo.segmentation()));
        for p in vo.paths() {
            let doc = vo.document_at(p).expect("path from manifest");
            self.say(&format!("  {}  {p}\n", &doc[..12]));
        }
        Ok(())
    }

    // ── diff (PROVISIONAL.md #8) ────────────────────────────────

    fn cmd_diff(&mut self, argv: &mut Vec<String>) -> Result<()> {
        // Rendering: the apparatus (editorial change list) is the
        // default; --unified is the legacy git-style -/+ form
        // (SPEC §7.4; HISTORY §17).
        let unified = argv.iter().any(|a| a == "--unified");
        argv.retain(|a| a != "--unified");
        // Display bonds (SPEC §7.4 *Bonds*): the apparatus shows
        // molecules where a .desm applies; --unified never does.
        let bonds_choice = bonds::choice_from_args(argv, &self.cwd)?;
        if argv.len() > 2 {
            err!("usage: lit diff [--unified] [<version> [<version>]]");
        }
        let a = argv.first().cloned();
        let b = argv.get(1).cloned();

        let repo = self.repo()?;
        repo.assert_segmentation()?;

        let side_b = match &b {
            Some(b) => {
                let vo = repo.version(&self.resolve_version(&repo, b)?)?;
                self.side_of_version(&repo, vo.as_ref())?
            }
            None => match repo.read_tabula()? {
                Some(tab) => self.side_of_version(&repo, Some(&tab))?,
                None => self.side_of_scan(&repo.scan()?),
            },
        };

        let side_a = match &a {
            Some(a) => {
                let vo = repo.version(&self.resolve_version(&repo, a)?)?;
                self.side_of_version(&repo, vo.as_ref())?
            }
            None => {
                let tip = repo.historia().latest()?;
                let vo = match &tip {
                    Some(t) => repo.version(&t.vhash)?,
                    None => None,
                };
                self.side_of_version(&repo, vo.as_ref())?
            }
        };

        let text = if unified {
            diff::render(&side_a, &side_b)?
        } else {
            let mut bonds_of = Bonds::new(repo.root(), bonds_choice);
            let diffs = diff::collate_with(&side_a, &side_b, &mut |p| bonds_of.for_path(p))?;
            diff::render_apparatus(&diffs)
        };
        self.say(if text.is_empty() {
            "no differences\n"
        } else {
            &text
        });
        Ok(())
    }

    // ── stemma (SPEC §9.5): provenance ──────────────────────────

    fn cmd_stemma(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let arg = if argv.is_empty() {
            None
        } else {
            Some(argv.remove(0))
        };
        let path = if argv.is_empty() {
            None
        } else {
            Some(argv.remove(0))
        };
        let coord_arg = if argv.is_empty() {
            None
        } else {
            Some(argv.remove(0))
        };
        if !argv.is_empty() {
            err!("usage: lit stemma [<version> [<path> <coord>]]");
        }
        if path.is_some() && coord_arg.is_none() {
            err!("stemma with a path needs an atom coordinate (gNaM or aK)");
        }

        let repo = self.repo()?;
        let vhash = match &arg {
            Some(a) => self.resolve_version(&repo, a)?,
            None => {
                let Some(tip) = repo.historia().latest()? else {
                    err!("historia has no inscriptiones yet");
                };
                tip.vhash
            }
        };

        // Every inscriptio across every history, with the §9.5
        // sort key: (timestamp, historia-first, name, entry order).
        struct Entry {
            name: String,
            rank: u8,
            ordinal: usize,
            ts: String,
            msg: String,
            vhash: String,
        }
        let mut entries: Vec<Entry> = Vec::new();
        let mut histories = vec![repo.historia()];
        for name in repo.experiments() {
            if let Some(h) = repo.experiment(&name)? {
                histories.push(h);
            }
        }
        let mut history_count = 0usize;
        for h in &histories {
            let es = h.entries()?;
            if !es.is_empty() {
                history_count += 1;
            }
            for (i, e) in es.into_iter().enumerate() {
                entries.push(Entry {
                    name: h.name().to_string(),
                    rank: if h.name() == "historia" { 0 } else { 1 },
                    ordinal: i + 1,
                    ts: e.timestamp,
                    msg: e.message.split('\n').next().unwrap_or("").to_string(),
                    vhash: e.vhash,
                });
            }
        }
        entries.sort_by(|a, b| {
            a.ts.cmp(&b.ts)
                .then(a.rank.cmp(&b.rank))
                .then(a.name.cmp(&b.name))
                .then(a.ordinal.cmp(&b.ordinal))
        });

        // ── Atom provenance ─────────────────────────────────────
        if let Some(path) = path {
            let Some(vo) = repo.version(&vhash)? else {
                err!("no such version {vhash}");
            };
            let Some(doc) = vo.document_at(&path) else {
                err!("version {} has no document at '{path}'", &vhash[..12]);
            };
            let Some(bytes) = repo.theca().get("volumen", doc)? else {
                err!("volumen {doc} missing from theca");
            };
            let attrs = repo.attributes_for_version(&vo)?;
            let Some(d) = crate::derive::derive(&bytes, attrs.mode_for(&path)) else {
                err!("stored volumen {doc} does not derive as a document");
            };
            let coord = coord_arg.expect("checked above");
            let atom: &str = match parse_coord(&coord) {
                Some(Coord::GrexAtom(gn, an)) => {
                    if gn == 0 || gn > d.grexes.len() {
                        err!(
                            "no grex g{gn} in {path} (document has {} grexes)",
                            d.grexes.len()
                        );
                    }
                    let grex = &d.grexes[gn - 1];
                    if an == 0 || an > grex.len() {
                        err!(
                            "no atom g{gn}a{an} in {path} (grex g{gn} has {} atoms)",
                            grex.len()
                        );
                    }
                    &grex[an - 1]
                }
                Some(Coord::Flat(k)) => {
                    let flat: Vec<&String> = d.grexes.iter().flatten().collect();
                    if k == 0 || k > flat.len() {
                        err!("no atom a{k} in {path} (document has {} atoms)", flat.len());
                    }
                    flat[k - 1]
                }
                _ => err!("stemma needs an atom coordinate (gNaM or aK)"),
            };
            let atom_hash = crate::derive::hash_bytes(atom.as_bytes());
            self.say(&format!("atom {atom_hash}  \"{}\"\n", snippet(atom)));

            // Which inscribed versions contain the atom? Exact
            // atom-spina membership, cached per version.
            let theca = repo.theca();
            let mut cache: HashMap<String, bool> = HashMap::new();
            let mut containing: Vec<&Entry> = Vec::new();
            for e in &entries {
                let has = match cache.get(&e.vhash) {
                    Some(&v) => v,
                    None => {
                        let v = version_contains_atom(&repo, &theca, &e.vhash, &atom_hash)?;
                        cache.insert(e.vhash.clone(), v);
                        v
                    }
                };
                if has {
                    containing.push(e);
                }
            }
            if containing.is_empty() {
                self.say("not inscribed in any history\n");
                return Ok(());
            }
            let first = containing[0];
            let last = containing[containing.len() - 1];
            self.say(&format!(
                "first: {} #{}  {}  {}\n",
                first.name, first.ordinal, first.ts, first.msg
            ));
            self.say(&format!(
                "last: {} #{}  {}  {}\n",
                last.name, last.ordinal, last.ts, last.msg
            ));
            self.say(&format!(
                "present in {} of {} inscriptiones across {} histories\n",
                containing.len(),
                entries.len(),
                history_count
            ));
            return Ok(());
        }

        // ── Version life-path ───────────────────────────────────
        self.say(&format!("version {vhash}\n"));
        let mine: Vec<&Entry> = entries.iter().filter(|e| e.vhash == vhash).collect();
        if mine.is_empty() {
            self.say("not inscribed in any history\n");
            return Ok(());
        }
        for (i, e) in mine.iter().enumerate() {
            self.say(&format!(
                "  {} #{}  {}  {}{}\n",
                e.name,
                e.ordinal,
                e.ts,
                e.msg,
                if i == 0 { " (origin)" } else { "" }
            ));
        }
        Ok(())
    }

    // ── gc (SPEC §6.10) ─────────────────────────────────────────

    fn cmd_gc(&mut self, argv: &mut Vec<String>) -> Result<()> {
        let mut dry_run = false;
        parse_opts(argv, |opt, _take| match opt {
            "--dry-run" => {
                dry_run = true;
                Ok(true)
            }
            _ => Ok(false),
        })
        .map_err(|_| Error::new("bad gc options"))?;
        if !argv.is_empty() {
            err!("gc takes no positional arguments");
        }

        let repo = self.repo()?;
        let report = crate::gc::collect(&repo, dry_run)?;
        if report.total() == 0 {
            self.say("nothing to collect\n");
            return Ok(());
        }
        let detail = report
            .counts
            .iter()
            .map(|(t, n)| format!("{t} {n}"))
            .collect::<Vec<_>>()
            .join(", ");
        self.say(&format!(
            "{} {} objects ({detail})\n",
            if dry_run {
                "would collect"
            } else {
                "collected"
            },
            report.total()
        ));
        Ok(())
    }

    // ── fsck (SPEC §10 #7) ──────────────────────────────────────

    fn cmd_fsck(&mut self, argv: &mut Vec<String>) -> Result<()> {
        if !argv.is_empty() {
            err!("fsck takes no arguments");
        }
        let repo = self.repo()?;
        repo.assert_segmentation()?;
        let problems = fsck::check(&repo);
        if !problems.is_empty() {
            for p in &problems {
                self.errsay(&format!("{p}\n"));
            }
            err!(
                "{} problem{} found",
                problems.len(),
                if problems.len() == 1 { "" } else { "s" }
            );
        }
        self.say("fsck: repository is sound\n");
        Ok(())
    }
}

enum Coord {
    Grex(usize),
    GrexAtom(usize, usize),
    Flat(usize),
}

/// Parses the SPEC §7.4 display coordinates: gN, gNaM, aK.
fn parse_coord(s: &str) -> Option<Coord> {
    fn num(s: &str) -> Option<usize> {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    }
    if let Some(rest) = s.strip_prefix('g') {
        if let Some((g, a)) = rest.split_once('a') {
            return Some(Coord::GrexAtom(num(g)?, num(a)?));
        }
        return Some(Coord::Grex(num(rest)?));
    }
    if let Some(rest) = s.strip_prefix('a') {
        return Some(Coord::Flat(num(rest)?));
    }
    None
}

/// Status-style snippet: whitespace runs collapse to single
/// spaces; longer than 60 chars truncates to 57 + "...".
fn snippet(atom: &str) -> String {
    let mut out = String::with_capacity(atom.len());
    let mut in_run = false;
    for c in atom.chars() {
        if c.is_whitespace() {
            if !in_run {
                out.push(' ');
                in_run = true;
            }
        } else {
            out.push(c);
            in_run = false;
        }
    }
    let chars: Vec<char> = out.chars().collect();
    if chars.len() > 60 {
        let head: String = chars[..57].iter().collect();
        format!("{head}...")
    } else {
        out
    }
}

/// Exact atom-spina membership: does any document of the version
/// contain the atom hash?
fn version_contains_atom(
    repo: &Repository,
    theca: &crate::theca::Theca,
    vhash: &str,
    atom_hash: &str,
) -> Result<bool> {
    let Some(vo) = repo.version(vhash)? else {
        return Ok(false);
    };
    for doc in vo.documents() {
        let trio = vo.trio_for(doc).expect("trio for document");
        let Some(aspina) = theca.get("atom-spina", trio[2])? else {
            continue;
        };
        let text = String::from_utf8_lossy(&aspina);
        if text.split('\n').any(|l| l == atom_hash) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_hex(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Recursively collect regular files under `root` as sorted
/// slash-relative (path, bytes) pairs (exemplar repack). A
/// `.lit/` directory never ships; everything else does —
/// dot-led documents are legitimate manifest entries.
fn collect_files(
    root: &std::path::Path,
    prefix: &str,
    docs: &mut Vec<(String, Vec<u8>)>,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(root)
        .map_err(|e| Error::new(format!("read {}: {e}", root.display())))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".lit" {
            continue;
        }
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, &rel, docs)?;
        } else if path.is_file() {
            let bytes = std::fs::read(&path)
                .map_err(|e| Error::new(format!("read {rel}: {e}")))?;
            docs.push((rel, bytes));
        }
    }
    Ok(())
}

pub(crate) fn states_equal(a: &SideInput, b: &SideInput) -> bool {
    a.manifest == b.manifest
}

fn revision_line(rev: &Revision) -> String {
    let mut line = format!("{}. {}", rev.index, rev.path);
    match rev.kind {
        RevKind::Edit | RevKind::Formatting => {
            let g = if !rev.tabula_grexes.is_empty() {
                &rev.tabula_grexes
            } else {
                &rev.parent_grexes
            };
            let g_label = if g.len() == 1 {
                format!("g{}", g[0] + 1)
            } else {
                format!("g{}-g{}", g[0] + 1, g[g.len() - 1] + 1)
            };
            let category = rev.category.unwrap_or("-");
            let what_note = match rev.what {
                Some(w) if w != "modified" => format!(": {w}"),
                _ => String::new(),
            };
            line.push_str(&format!(" {g_label} ({category}{what_note})"));
        }
        RevKind::Rename => {
            line.push_str(&format!(
                " renamed -> {}",
                rev.new_path.as_deref().unwrap_or("")
            ));
        }
        RevKind::Removal => line.push_str(" removed"),
        RevKind::FirstVersion => line.push_str(" added"),
    }
    if !rev.snippet.is_empty() && rev.kind != RevKind::Rename && rev.kind != RevKind::Removal {
        line.push_str(&format!(" \"{}\"", rev.snippet));
    }
    line
}

/// Minimal option parser: `--opt`, `--opt value`, `--opt=value`,
/// `-x value`. The callback returns Ok(true) when it consumed the
/// option, Ok(false) for an unknown option (an error, matching
/// Getopt::Long's behavior), and non-option arguments stay in
/// argv.
fn parse_opts(
    argv: &mut Vec<String>,
    mut handle: impl FnMut(&str, &mut dyn FnMut() -> Result<String>) -> Result<bool>,
) -> Result<()> {
    let mut rest: Vec<String> = Vec::new();
    let mut items = std::mem::take(argv).into_iter().peekable();
    while let Some(item) = items.next() {
        if !item.starts_with('-') || item == "-" {
            rest.push(item);
            continue;
        }
        let (opt, inline_value) = match item.split_once('=') {
            Some((o, v)) => (o.to_string(), Some(v.to_string())),
            None => (item.clone(), None),
        };
        let mut used_inline = false;
        let consumed = {
            let mut take = || -> Result<String> {
                if let Some(v) = &inline_value {
                    used_inline = true;
                    return Ok(v.clone());
                }
                match items.next() {
                    Some(v) => Ok(v),
                    None => Err(Error::new(format!("option {opt} needs a value"))),
                }
            };
            handle(&opt, &mut take)?
        };
        if !consumed {
            err!("unknown option {opt}");
        }
        if inline_value.is_some() && !used_inline {
            err!("option {opt} does not take a value");
        }
    }
    *argv = rest;
    Ok(())
}

#[cfg(test)]
mod alias_tests {
    use super::{alias, git_hint};

    #[test]
    fn git_peers_resolve_to_handlers() {
        for (git, canonical) in [
            ("add", "conscribe"),
            ("stage", "conscribe"),
            ("commit", "inscribe"),
            ("checkout", "exscribe"),
            ("restore", "exscribe"),
            ("merge", "contexe"),
            ("branch", "experiment"),
            ("log", "show"),
            ("blame", "stemma"),
        ] {
            assert_eq!(alias(git), canonical, "git peer {git}");
        }
    }

    #[test]
    fn latin_greek_peers_still_resolve() {
        assert_eq!(alias("inscribo"), "inscribe");
        assert_eq!(alias("epigrapho"), "inscribe");
        assert_eq!(alias("synhyphaino"), "contexe");
        assert_eq!(alias("confero"), "collate");
    }

    #[test]
    fn canonical_names_pass_through() {
        for c in ["conscribe", "inscribe", "push", "publish", "opus", "account"] {
            assert_eq!(alias(c), c);
        }
    }

    #[test]
    fn omitted_git_verbs_have_hints() {
        for v in [
            "init", "revert", "cherry-pick", "rebase", "switch", "stash", "fetch",
            "tag", "remote", "origin",
        ] {
            let hint = git_hint(v).unwrap_or_else(|| panic!("no hint for {v}"));
            assert!(hint.contains("TERMINOLOGY"), "hint for {v} cites TERMINOLOGY");
        }
    }

    #[test]
    fn real_commands_have_no_hint() {
        for c in ["conscribe", "commit", "push", "unknownzzz"] {
            assert!(git_hint(c).is_none(), "{c} must not be a git hint");
        }
    }
}
