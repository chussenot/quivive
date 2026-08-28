//! `quivive watch`: `notify-send` on transitions only, debounced per
//! (repo, event), every notification carrying its S20 follow-up command —
//! S14-S20 of `docs/spec.md`.
//!
//! The loop below is two layers over `state`'s pure seam:
//!
//! 1. **S14** is not this file's job at all — `state::transitions` already
//!    IS "becomes true, not while it stays true", comparing one tick's
//!    [`state::RepoAssessment`] to the one before it. This module just keeps
//!    the previous assessment per repo and calls it once per tick per repo.
//! 2. **S15** *is* this file's job: `transitions` only ever compares two
//!    *consecutive* ticks, so a condition that drops out of `attention` for
//!    even one tick — an agent's evidence flickering, a bead's `type` field
//!    toggled and toggled back — reads as "new" again the moment it returns,
//!    and would notify on every flap. [`Debouncer`] is the second, coarser
//!    filter that actually satisfies "a flapping condition... notifies
//!    once."
//!
//! **The tick stays file-reads-only (S3).** Everything up through building a
//! [`state::RepoAssessment`] and diffing it is pure computation over what
//! [`reader::read`] returned; [`send`] — spawning `notify-send` — is the
//! *only* place in this file that starts a subprocess, and it runs strictly
//! after a tick has already been judged, never as part of judging it.
//!
//! **Known gap: `S18`'s `GateOrderViolation` cannot fire from this loop
//! today.** [`to_snapshot`] always sets `plan: None` — no reader anywhere in
//! this crate yet folds `.pact/plan.json`'s wave/gate shape together with
//! the events tail's "what started, in which wave" evidence into
//! [`state::PlanSnapshot`], and inventing that fold here would duplicate
//! whichever bead builds it for real (`why` or `tile`) rather than reuse it.
//! `state::gate_order_violations` only ever sees `snapshot.plan`, so with it
//! always `None` the violation simply never appears in `attention` — quietly
//! correct, not silently wrong. The formatting and S20 follow-up for
//! `GateOrderViolation` are implemented and tested against hand-built
//! fixtures below regardless, ready the moment a real plan reader lands.
//!
//! That same gap decides S20's follow-up for `GateOrderViolation`: the
//! payload [`state::AttentionItem::GateOrderViolation`] carries has no
//! `events.jsonl` line number (nothing in `reader::Readings` exposes one for
//! the ledger tail — only the sidecar's rows carry a line, via
//! `reader::sidecar::Row::line`), so `recount explain --event-line N` is not
//! answerable here without leasing and extending the ledger reader itself,
//! which is out of this bead's scope. `pact audit --check gate-order` — the
//! family's own auditor for exactly this condition — stands in instead. See
//! [`format_event`]'s `GateOrderViolation` arm.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::reader::{self, sidecar};
use crate::state::{
    self, AttentionItem, RepoAssessment, RepoSnapshot, Thresholds, TransitionEvent,
};
use crate::{dur, registry};

/// Default poll interval between passes over the registry.
///
/// 2s — a human reaction path, not a render path. It is neither
/// `docs/tile-contract.md`'s ~1s bar-respawn cadence (this loop draws
/// nothing) nor S6's 10ms per-repo tick ceiling (that bounds one read, not
/// how often to repeat it). Two seconds is fast enough that "a bead just got
/// flagged" reads as near-instant to a person watching for a desktop
/// notification, and slow enough that a registry of dozens of repos — each
/// mtime-pruned by S5 into a handful of `stat` calls once quiet — never
/// becomes something worth tuning for its own sake. A starting point, hence
/// `--interval` rather than a promise.
pub const INTERVAL_DEFAULT: &str = "2s";

/// Default debounce window (S15): how long a (repo, event) stays suppressed
/// after it last actually notified.
///
/// 5 minutes, matching `state::IDLE_DEFAULT` — not a coincidence. That
/// constant is already this crate's answer to "how long is an ordinary quiet
/// moment for an agent," and a condition flapping inside that same window —
/// an agent's evidence bouncing dead/active, a bead's `type` field getting
/// corrected and corrected back — is exactly the flapping S15 exists to
/// silence rather than re-report as a fresh human-needed event every time it
/// flickers back. Shorter buys back little: `transitions` (S14) already
/// suppresses a condition that stays continuously true, so this window only
/// ever matters for one that drops out and returns. Longer would sit on a
/// genuinely recurring, still-unresolved problem for uncomfortably long.
pub const DEBOUNCE_DEFAULT: &str = "5m";

/// The two knobs `quivive watch` takes: how often to look, and how long to
/// stay quiet about a repeat of the same standing story.
#[derive(Debug, Clone, Copy)]
pub struct WatchOptions {
    pub interval: Duration,
    pub debounce: Duration,
}

/// One fully-formatted notification: exactly what a fake notifier in tests
/// asserts, and exactly what [`send`] hands to `notify-send` (or prints, if
/// `notify-send` is unavailable).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Notification {
    title: String,
    body: String,
    /// S20: the follow-up command a human runs to act on this notification.
    command: String,
}

/// The notify-send edge, abstracted so a test can inject a recorder and
/// assert exact notification bodies without spawning anything real.
trait Notifier {
    /// `Err` propagates only a genuine I/O failure writing the stdout
    /// fallback (see [`send`]) — never a `notify-send` problem, which
    /// [`send`] always degrades instead of erroring on.
    fn notify(&self, n: &Notification) -> std::io::Result<()>;
}

/// The real notifier: `notify-send` via a subprocess, falling back to
/// stdout. The only [`Notifier`] wired into [`run`].
struct RealNotifier;

impl Notifier for RealNotifier {
    fn notify(&self, n: &Notification) -> std::io::Result<()> {
        send(n, &mut std::io::stdout())
    }
}

/// The line printed when `notify-send` could not be spawned — kept as its
/// own pure function so a test can assert it without capturing real stdout.
fn fallback_line(n: &Notification) -> String {
    format!("{}: {}  $ {}", n.title, n.body, n.command)
}

/// Spawn `notify-send`; on any failure to spawn it — the binary absent (it
/// is, in this container), or any other spawn error — degrade to printing
/// the notification on `out` instead of failing. A watcher that dies because
/// libnotify is missing watches nothing, so this function has no error path
/// of its own beyond `out` itself refusing the write (SIGPIPE — `run`
/// treats that as "stop watching cleanly," not a crash).
///
/// This is the ONLY subprocess spawn in this file, and it happens strictly
/// after a tick has already been judged — see the module doc's S3 note.
fn send(n: &Notification, out: &mut dyn Write) -> std::io::Result<()> {
    let full_body = format!("{}\n\n$ {}", n.body, n.command);
    match ProcessCommand::new("notify-send")
        .arg(&n.title)
        .arg(&full_body)
        .status()
    {
        Ok(_) => Ok(()),
        Err(_) => writeln!(out, "{}", fallback_line(n)),
    }
}

/// S16-S20: turn one transition into exactly what a human needs — what
/// happened, and the S20 follow-up command that answers it. Pure and
/// deterministic, which is what makes the four bodies below exactly what
/// the tests assert.
fn format_event(repo: &str, event: &TransitionEvent) -> Notification {
    match event {
        TransitionEvent::Attention(AttentionItem::DeadHoldingPaths {
            agent,
            paths,
            remaining_ttl,
        }) => Notification {
            title: format!("quivive: {repo} — dead agent holding paths"),
            // S16 verbatim: names the agent, the paths, and the remaining TTL.
            body: format!(
                "{agent} is dead and still holds {n} path{plural}: {joined} (TTL {ttl})",
                n = paths.len(),
                plural = if paths.len() == 1 { "" } else { "s" },
                joined = paths.join(", "),
                ttl = dur::human(*remaining_ttl),
            ),
            // S20: the lease-facing follow-up.
            command: "pact lease ls".to_string(),
        },
        TransitionEvent::Attention(AttentionItem::NeedsDecision { bead_id }) => Notification {
            title: format!("quivive: {repo} — needs-decision bead filed"),
            // S17 verbatim: a needs-decision bead is filed.
            body: format!("{bead_id} was filed as needs-decision"),
            // S20: the bead-facing follow-up.
            command: format!("bd show {bead_id}"),
        },
        TransitionEvent::Attention(AttentionItem::GateOrderViolation {
            started_id,
            started_wave,
            open_gate_id,
            gate_wave,
        }) => Notification {
            title: format!("quivive: {repo} — gate-order violation"),
            // S18 verbatim: work in a later wave started before an earlier
            // wave's declared gate closed.
            body: format!(
                "{started_id} started in wave {started_wave} before gate {open_gate_id} \
                 (wave {gate_wave}) closed"
            ),
            // S20's third option, not `bd show <id>`: see the module doc's
            // "known gap" note for why there is no events.jsonl line number
            // to cite here, and why the family's own gate-order auditor is
            // the honest follow-up instead of a bare bead lookup that would
            // say nothing about the gate itself.
            command: "pact audit --check gate-order".to_string(),
        },
        TransitionEvent::FleetDrained => Notification {
            title: format!("quivive: {repo} — fleet drained"),
            // S19 / S8's own wording for `drained`.
            body: "the fleet drained: a plan or recent fleet evidence exists but no live agent \
                   remains"
                .to_string(),
            // S20: `bd ready` over `pact log` — the question a human has the
            // instant a fleet drains is "what still needs doing" (the
            // unblocked work `bd ready` lists), not a chronological replay
            // of who leased what, which is what `pact log` would show
            // instead.
            command: "bd ready".to_string(),
        },
    }
}

/// The identity [`Debouncer`] groups by — its own concept, not
/// `state::AttentionItem`'s private `identity()`, but built for the same
/// reason: one bucket per (event-shape, primary id[, secondary id]),
/// independent of any field that counts down or otherwise drifts tick to
/// tick (`remaining_ttl`, `started_wave`, `gate_wave`), so a standing
/// condition's *content* moving does not look like a new event to debounce
/// against.
type EventKey = (u8, String, String);

fn event_key(event: &TransitionEvent) -> EventKey {
    match event {
        TransitionEvent::Attention(AttentionItem::DeadHoldingPaths { agent, .. }) => {
            (0, agent.clone(), String::new())
        }
        TransitionEvent::Attention(AttentionItem::NeedsDecision { bead_id }) => {
            (1, bead_id.clone(), String::new())
        }
        TransitionEvent::Attention(AttentionItem::GateOrderViolation {
            started_id,
            open_gate_id,
            ..
        }) => (2, started_id.clone(), open_gate_id.clone()),
        TransitionEvent::FleetDrained => (3, String::new(), String::new()),
    }
}

/// S15's per-(repo, event) cooldown, layered on top of S14's becoming-true
/// edge — see the module doc for why both layers are needed.
struct Debouncer {
    window_secs: i64,
    /// Epoch seconds of the last time each (repo, event) actually notified.
    /// Epoch `i64`, not a `DateTime` delta: two timestamps this crate reads
    /// can legitimately be very far apart (a lease's `expires_at`, a clock
    /// that jumped), and `DateTime` subtraction panics past i64
    /// milliseconds — the same reasoning `reader::lease::Lease::expired_for`
    /// and `tile::Tile::build`'s age math already apply.
    last_sent: HashMap<(PathBuf, EventKey), i64>,
}

impl Debouncer {
    fn new(window: Duration) -> Self {
        Self {
            window_secs: window.as_secs() as i64,
            last_sent: HashMap::new(),
        }
    }

    /// True the first time (repo, event) is ever seen, or once `window_secs`
    /// has elapsed since it last actually notified. Never mutates — call
    /// [`Self::record`] only after a real send, so a *suppressed* event does
    /// not reset its own cooldown.
    fn should_notify(&self, repo: &Path, event: &TransitionEvent, now: DateTime<Utc>) -> bool {
        let key = (repo.to_path_buf(), event_key(event));
        match self.last_sent.get(&key) {
            None => true,
            Some(&last) => now.timestamp() - last >= self.window_secs,
        }
    }

    fn record(&mut self, repo: &Path, event: &TransitionEvent, now: DateTime<Utc>) {
        let key = (repo.to_path_buf(), event_key(event));
        self.last_sent.insert(key, now.timestamp());
    }
}

/// Diff `prev` against `now_assessment` (S14, via `state::transitions`), and
/// notify every transition that survives the debounce window (S15). `repo`
/// is used both to key the debounce and, via `.display()`, as the label in
/// the notification title — no filesystem access happens here, so a test can
/// pass a bare, non-existent path.
fn notify_transitions(
    repo: &Path,
    prev: &RepoAssessment,
    now_assessment: &RepoAssessment,
    now: DateTime<Utc>,
    debouncer: &mut Debouncer,
    notifier: &dyn Notifier,
) -> std::io::Result<()> {
    let repo_label = repo.display().to_string();
    for event in state::transitions(prev, now_assessment) {
        if !debouncer.should_notify(repo, &event, now) {
            continue;
        }
        notifier.notify(&format_event(&repo_label, &event))?;
        debouncer.record(repo, &event, now);
    }
    Ok(())
}

/// S17: which sidecar rows mean "this bead needs a human decision".
///
/// bd's committed sidecar only ever emits `kind: "field_change"` rows —
/// there is no `created`/`filed` kind to key off (see `reader::sidecar`'s
/// module doc) — so a bead being flagged needs-decision shows up the same
/// way any other field edit does: `field: "type", new_value:
/// "needs-decision"` (`reader::sidecar`'s own test fixture uses exactly this
/// shape). Folded newest-row-wins per bead — `reader::Readings::interactions`
/// is documented oldest-first, so a later insert for the same `issue_id`
/// simply overwrites — the same "latest evidence, not first" rule every
/// other reader in this crate applies to its own fold (`reader::read`'s
/// agent merge, `reader::ledger`'s per-agent newest-wins). A bead whose
/// `type` later moves away from `needs-decision` must stop being reported,
/// not stay flagged forever because of a row from months ago.
///
/// Unlike S16/S19 (see [`tick_repo`]'s doc), S17 cannot freeze under an
/// unchanged pass: it can only ever become true by a *write* to `.beads/`'s
/// committed sidecar, and `reader::unchanged`'s watermark stats that exact
/// file (`reader::newest_source_mtime`) — so the write that makes a bead
/// newly needs-decision is itself what trips S5's gate and forces the next
/// pass to read for real. Confirmed by reading `reader::newest_source_mtime`,
/// not assumed.
fn needs_decision_from(rows: &[sidecar::Row]) -> Vec<String> {
    let mut latest_type: BTreeMap<&str, &str> = BTreeMap::new();
    for row in rows {
        if row.field.as_deref() == Some("type")
            && let Some(new_value) = row.new_value.as_deref()
        {
            latest_type.insert(row.issue_id.as_str(), new_value);
        }
    }
    latest_type
        .into_iter()
        .filter(|(_, value)| *value == "needs-decision")
        .map(|(id, _)| id.to_string())
        .collect()
}

/// Build the pure-judgment [`RepoSnapshot`] this tick's `state::assess`
/// needs, from what [`reader::read`] actually gathered. See the module doc
/// for why `plan` is always `None` here.
fn to_snapshot(readings: &reader::Readings, repo_root: &Path) -> RepoSnapshot {
    let leases = readings
        .leases
        .iter()
        .map(|l| state::LeaseSnapshot {
            agent: l.agent.clone(),
            path: l.path.clone(),
            acquired_at: l.acquired_at,
            expires_at: l.expires_at,
        })
        .collect();

    RepoSnapshot {
        agents: readings.agents.clone(),
        leases,
        plan: None,
        needs_decision: needs_decision_from(&readings.interactions),
        // `.pact/` existing at all is what separates S8's `all-quiet` from
        // `no-fleet`; `reader::read` never reports this directly (it reports
        // whether the *ledger* is present, which is narrower — a fresh pact
        // with no events yet still has a `.pact/` directory).
        pact_present: reader::state_dir(repo_root).is_dir(),
        degraded: readings.degraded.clone(),
    }
}

/// One repo's worth of one pass: mtime-prune (S5, `reader::unchanged`),
/// read-or-reassess, diff against the previous tick's assessment for that
/// repo, and notify what survives debounce (S15).
///
/// On an **unchanged** pass (S5 licenses skipping the re-READ): the re-read
/// is skipped, but `state::assess` is re-run over `snapshot_cache`'s last
/// built [`RepoSnapshot`] for this repo, at the new `now`. `state::assess` is
/// a pure function of `(snapshot, now, thresholds)` — nothing in it needs a
/// fresh read, only a fresh clock — so this is what keeps S16 (a DEAD agent)
/// and S19 (the fleet drained) from freezing on a repo nobody is writing to:
/// both become true by clock passage alone, with no file write to trip
/// `reader::unchanged`'s watermark. A repo with no cached snapshot yet (the
/// very first pass a watermark could exist for) cannot be true here — the
/// watermark and the cache are only ever written together, in the read
/// branch below — so that arm only exists to keep the lookup total.
///
/// `prev` — and therefore `notify_transitions`, and therefore the debounce
/// and S14's becoming-true edge — is fed by this reassessed judgment exactly
/// like a freshly-read one: this function inserts into `prev` on every call
/// that reaches the bottom, whichever branch produced the assessment. That is
/// what makes a state that decayed while pruned fire exactly once the first
/// time it is assessed as true, the same "two *consecutive* ticks" contract
/// `state::transitions` already keeps for a freshly-read repo — there is no
/// separate rule here, only the same one applied to a `now` that moved
/// without a `readings` to go with it.
#[allow(clippy::too_many_arguments)]
fn tick_repo(
    repo: &Path,
    now: DateTime<Utc>,
    thresholds: &Thresholds,
    prune_watermark: &mut HashMap<PathBuf, DateTime<Utc>>,
    snapshot_cache: &mut HashMap<PathBuf, RepoSnapshot>,
    prev: &mut HashMap<PathBuf, RepoAssessment>,
    debouncer: &mut Debouncer,
    notifier: &dyn Notifier,
) -> std::io::Result<()> {
    if let Some(watermark) = prune_watermark.get(repo)
        && reader::unchanged(repo, *watermark)
    {
        continue_unchanged(
            repo,
            now,
            thresholds,
            snapshot_cache,
            prev,
            debouncer,
            notifier,
        )
    } else {
        read_and_assess(
            repo,
            now,
            thresholds,
            prune_watermark,
            snapshot_cache,
            prev,
            debouncer,
            notifier,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn continue_unchanged(
    repo: &Path,
    now: DateTime<Utc>,
    thresholds: &Thresholds,
    snapshot_cache: &mut HashMap<PathBuf, RepoSnapshot>,
    prev: &mut HashMap<PathBuf, RepoAssessment>,
    debouncer: &mut Debouncer,
    notifier: &dyn Notifier,
) -> std::io::Result<()> {
    // S5 licenses skipping the re-READ, not the re-assessment: `assess` is a
    // pure function of `(snapshot, now, thresholds)`, so re-running it over
    // the last snapshot this repo actually read still lets S16/S19 decay
    // with the clock even though nothing on disk moved. No re-read, no
    // cursor commit, no watermark update — none of those belong to a pass
    // that read nothing.
    let Some(cached) = snapshot_cache.get(repo) else {
        // `unchanged` can only be true once `prune_watermark` holds an entry
        // for this repo, and that entry is only ever written in
        // `read_and_assess` alongside the matching cache entry — so this arm
        // keeps the lookup total, not because it is reachable.
        return Ok(());
    };
    let assessment = state::assess(cached, now, thresholds);

    if let Some(prior_assessment) = prev.get(repo) {
        notify_transitions(
            repo,
            prior_assessment,
            &assessment,
            now,
            debouncer,
            notifier,
        )?;
    }
    prev.insert(repo.to_path_buf(), assessment);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_and_assess(
    repo: &Path,
    now: DateTime<Utc>,
    thresholds: &Thresholds,
    prune_watermark: &mut HashMap<PathBuf, DateTime<Utc>>,
    snapshot_cache: &mut HashMap<PathBuf, RepoSnapshot>,
    prev: &mut HashMap<PathBuf, RepoAssessment>,
    debouncer: &mut Debouncer,
    notifier: &dyn Notifier,
) -> std::io::Result<()> {
    let read_opts = reader::Options {
        repo_root: repo.to_path_buf(),
        use_cursor: true,
    };
    // A registry entry that cannot be read this pass — moved, unmounted, a
    // typo'd line — degrades like every other bad input in this crate: skip
    // it and try again next pass, rather than taking the whole watcher down
    // for one bad line (the same call `registry::read` itself makes about a
    // malformed registry).
    let Ok(readings) = reader::read(&read_opts) else {
        return Ok(());
    };

    let snapshot = to_snapshot(&readings, repo);
    let assessment = state::assess(&snapshot, now, thresholds);

    // Cursor committed before any notification is attempted, exactly like
    // `main.rs`'s `tick_once`: a panic or a broken pipe below must not leave
    // the cursor advanced past events this pass never got to report.
    reader::commit(repo, &readings);
    prune_watermark.insert(repo.to_path_buf(), now);
    snapshot_cache.insert(repo.to_path_buf(), snapshot);

    if let Some(prior_assessment) = prev.get(repo) {
        notify_transitions(
            repo,
            prior_assessment,
            &assessment,
            now,
            debouncer,
            notifier,
        )?;
    }
    prev.insert(repo.to_path_buf(), assessment);
    Ok(())
}

/// Watch the registry and notify on transitions, until interrupted or
/// stdout closes.
///
/// Per pass, per repo: [`tick_repo`]. A repo with no previous assessment yet
/// — every repo, on the very first pass — produces no notifications:
/// `state::transitions` needs two ticks to define "becomes true," and
/// treating an already-messy fleet's entire pre-existing state as "new" the
/// moment `quivive watch` starts would turn every startup into a
/// notification storm rather than into a baseline.
pub fn run(opts: &WatchOptions) -> anyhow::Result<()> {
    let repos = registry::read()?;
    let thresholds = Thresholds::default();
    let notifier = RealNotifier;
    let mut debouncer = Debouncer::new(opts.debounce);
    let mut prune_watermark: HashMap<PathBuf, DateTime<Utc>> = HashMap::new();
    let mut snapshot_cache: HashMap<PathBuf, RepoSnapshot> = HashMap::new();
    let mut prev: HashMap<PathBuf, RepoAssessment> = HashMap::new();

    loop {
        for repo in &repos {
            // One clock read per repo per pass, used both as the mtime-prune
            // watermark and as the tick clock — see `crate::now`'s doc on
            // why a tick reads the clock exactly once.
            let now = crate::now()?;

            match tick_repo(
                repo,
                now,
                &thresholds,
                &mut prune_watermark,
                &mut snapshot_cache,
                &mut prev,
                &mut debouncer,
                &notifier,
            ) {
                Ok(()) => {}
                // stdout closed underneath us (`quivive watch | head -1`, or
                // a caller that just stopped reading): this is the intended
                // way to stop watching, not a fault.
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }

        std::thread::sleep(opts.interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::Mutex;

    fn clock() -> DateTime<Utc> {
        "2026-08-28T12:00:00Z".parse().unwrap()
    }

    /// Collects every notification handed to it, in order — the "fake
    /// notifier" the acceptance criteria calls for: no test in this module
    /// spawns a real `notify-send`.
    #[derive(Default)]
    struct RecordingNotifier {
        sent: RefCell<Vec<Notification>>,
    }

    impl Notifier for RecordingNotifier {
        fn notify(&self, n: &Notification) -> std::io::Result<()> {
            self.sent.borrow_mut().push(n.clone());
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // S16-S20: the four exact notification bodies.
    // -----------------------------------------------------------------------

    #[test]
    fn s16_dead_holding_paths_names_agent_paths_and_ttl_with_the_lease_followup() {
        let event = TransitionEvent::Attention(AttentionItem::DeadHoldingPaths {
            agent: "agent-3".to_string(),
            paths: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
            remaining_ttl: 90,
        });
        let n = format_event("myrepo", &event);
        assert_eq!(n.title, "quivive: myrepo — dead agent holding paths");
        assert_eq!(
            n.body,
            "agent-3 is dead and still holds 2 paths: src/a.rs, src/b.rs (TTL 1m30s)"
        );
        assert_eq!(n.command, "pact lease ls");
    }

    #[test]
    fn s16_singular_path_is_not_pluralized() {
        let event = TransitionEvent::Attention(AttentionItem::DeadHoldingPaths {
            agent: "agent-9".to_string(),
            paths: vec!["src/c.rs".to_string()],
            remaining_ttl: 0,
        });
        let n = format_event("myrepo", &event);
        assert_eq!(
            n.body,
            "agent-9 is dead and still holds 1 path: src/c.rs (TTL 0s)"
        );
    }

    #[test]
    fn s17_needs_decision_names_the_bead_with_the_bd_show_followup() {
        let event = TransitionEvent::Attention(AttentionItem::NeedsDecision {
            bead_id: "bd-42".to_string(),
        });
        let n = format_event("myrepo", &event);
        assert_eq!(n.title, "quivive: myrepo — needs-decision bead filed");
        assert_eq!(n.body, "bd-42 was filed as needs-decision");
        assert_eq!(n.command, "bd show bd-42");
    }

    #[test]
    fn s18_gate_order_violation_names_started_and_gate_with_the_audit_followup() {
        let event = TransitionEvent::Attention(AttentionItem::GateOrderViolation {
            started_id: "bead-9".to_string(),
            started_wave: 2,
            open_gate_id: "gate-1".to_string(),
            gate_wave: 1,
        });
        let n = format_event("myrepo", &event);
        assert_eq!(n.title, "quivive: myrepo — gate-order violation");
        assert_eq!(
            n.body,
            "bead-9 started in wave 2 before gate gate-1 (wave 1) closed"
        );
        // The documented tradeoff: no event-line number is available, so
        // the family's own auditor is the follow-up rather than `bd show`.
        assert_eq!(n.command, "pact audit --check gate-order");
    }

    #[test]
    fn s19_fleet_drained_has_the_bd_ready_followup() {
        let n = format_event("myrepo", &TransitionEvent::FleetDrained);
        assert_eq!(n.title, "quivive: myrepo — fleet drained");
        assert_eq!(
            n.body,
            "the fleet drained: a plan or recent fleet evidence exists but no live agent remains"
        );
        assert_eq!(n.command, "bd ready");
    }

    // -----------------------------------------------------------------------
    // S14: becoming-true-only, driven end to end through notify_transitions.
    // -----------------------------------------------------------------------

    #[test]
    fn a_new_attention_item_notifies_exactly_once_and_a_standing_one_does_not_repeat() {
        let empty = state::assess(&RepoSnapshot::default(), clock(), &Thresholds::default());
        let mut snap = RepoSnapshot::default();
        snap.needs_decision.push("bd-7".to_string());
        let filed = state::assess(&snap, clock(), &Thresholds::default());

        let notifier = RecordingNotifier::default();
        let mut debouncer = Debouncer::new(Duration::from_secs(300));
        let repo = Path::new("repoA");

        notify_transitions(repo, &empty, &filed, clock(), &mut debouncer, &notifier).unwrap();
        assert_eq!(notifier.sent.borrow().len(), 1);
        assert_eq!(
            notifier.sent.borrow()[0].body,
            "bd-7 was filed as needs-decision"
        );

        // Next tick, still filed: transitions() itself reports nothing new.
        notify_transitions(repo, &filed, &filed, clock(), &mut debouncer, &notifier).unwrap();
        assert_eq!(
            notifier.sent.borrow().len(),
            1,
            "must not repeat while standing true"
        );
    }

    // -----------------------------------------------------------------------
    // S15: the debounce window.
    // -----------------------------------------------------------------------

    #[test]
    fn a_flapping_condition_notifies_once_inside_the_debounce_window() {
        // Two independent assessments transitions() sees as "new" each time
        // — the exact flap S15 exists to silence, simulated the way
        // `state.rs`'s own tests build fixtures (no filesystem needed).
        let empty = state::assess(&RepoSnapshot::default(), clock(), &Thresholds::default());
        let mut snap = RepoSnapshot::default();
        snap.needs_decision.push("bd-1".to_string());
        let filed = state::assess(&snap, clock(), &Thresholds::default());

        let notifier = RecordingNotifier::default();
        let mut debouncer = Debouncer::new(Duration::from_secs(300));
        let repo = Path::new("repoB");

        // Appears...
        notify_transitions(repo, &empty, &filed, clock(), &mut debouncer, &notifier).unwrap();
        // ...disappears (a real tick's transitions() would report nothing
        // here since it only fires on becoming-true; the interesting case is
        // the NEXT time it becomes true again, inside the window)...
        let t2 = clock() + chrono::TimeDelta::seconds(60);
        notify_transitions(repo, &filed, &empty, t2, &mut debouncer, &notifier).unwrap();
        // ...and becomes true again, still well inside the 5-minute window.
        let t3 = clock() + chrono::TimeDelta::seconds(120);
        notify_transitions(repo, &empty, &filed, t3, &mut debouncer, &notifier).unwrap();

        assert_eq!(
            notifier.sent.borrow().len(),
            1,
            "a flap within the debounce window must notify once"
        );
    }

    #[test]
    fn the_same_condition_notifies_again_once_the_debounce_window_has_elapsed() {
        let empty = state::assess(&RepoSnapshot::default(), clock(), &Thresholds::default());
        let mut snap = RepoSnapshot::default();
        snap.needs_decision.push("bd-1".to_string());
        let filed = state::assess(&snap, clock(), &Thresholds::default());

        let notifier = RecordingNotifier::default();
        let mut debouncer = Debouncer::new(Duration::from_secs(300));
        let repo = Path::new("repoC");

        notify_transitions(repo, &empty, &filed, clock(), &mut debouncer, &notifier).unwrap();
        let t2 = clock() + chrono::TimeDelta::seconds(60);
        notify_transitions(repo, &filed, &empty, t2, &mut debouncer, &notifier).unwrap();
        // Past the window this time.
        let t3 = clock() + chrono::TimeDelta::seconds(301);
        notify_transitions(repo, &empty, &filed, t3, &mut debouncer, &notifier).unwrap();

        assert_eq!(
            notifier.sent.borrow().len(),
            2,
            "past the window, the same condition must notify again"
        );
    }

    #[test]
    fn debounce_is_keyed_per_repo_not_globally() {
        let empty = state::assess(&RepoSnapshot::default(), clock(), &Thresholds::default());
        let mut snap = RepoSnapshot::default();
        snap.needs_decision.push("bd-1".to_string());
        let filed = state::assess(&snap, clock(), &Thresholds::default());

        let notifier = RecordingNotifier::default();
        let mut debouncer = Debouncer::new(Duration::from_secs(300));

        notify_transitions(
            Path::new("repoD"),
            &empty,
            &filed,
            clock(),
            &mut debouncer,
            &notifier,
        )
        .unwrap();
        notify_transitions(
            Path::new("repoE"),
            &empty,
            &filed,
            clock(),
            &mut debouncer,
            &notifier,
        )
        .unwrap();

        assert_eq!(
            notifier.sent.borrow().len(),
            2,
            "the same event identity in two different repos must both notify"
        );
    }

    // -----------------------------------------------------------------------
    // S3 / the notify-send edge: absent binary degrades to stdout, never to
    // an error.
    // -----------------------------------------------------------------------

    /// Guards PATH mutation across tests in this module the same way
    /// `registry.rs`'s own `EnvGuard` does: `cargo test`'s default harness
    /// runs tests in parallel threads of one process, and env vars are
    /// process-global.
    static PATH_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn absent_notify_send_degrades_to_stdout_and_never_errors() {
        let _guard = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let empty_path = tempfile::tempdir().unwrap();
        let prior = std::env::var_os("PATH");
        // An empty directory as the whole PATH guarantees `notify-send`
        // cannot be found, regardless of whether this host actually has it
        // installed — deterministic rather than relying on the container's
        // real absence, which the module doc notes but tests should not
        // depend on.
        unsafe { std::env::set_var("PATH", empty_path.path()) };

        let n = Notification {
            title: "quivive: myrepo — fleet drained".to_string(),
            body: "the fleet drained".to_string(),
            command: "bd ready".to_string(),
        };
        let mut out = Vec::new();
        let result = send(&n, &mut out);

        match prior {
            Some(p) => unsafe { std::env::set_var("PATH", p) },
            None => unsafe { std::env::remove_var("PATH") },
        }

        result.expect("a stdout fallback write must not itself error");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("{}\n", fallback_line(&n)),
            "an unspawnable notify-send must degrade to exactly the fallback line, not panic \
             or bail"
        );
    }

    // -----------------------------------------------------------------------
    // S17 derivation: needs-decision from the sidecar's field-change rows.
    // -----------------------------------------------------------------------

    fn row(issue_id: &str, field: &str, new_value: &str) -> sidecar::Row {
        sidecar::Row {
            issue_id: issue_id.to_string(),
            kind: "field_change".to_string(),
            actor: "someone".to_string(),
            at: clock(),
            field: Some(field.to_string()),
            new_value: Some(new_value.to_string()),
            old_value: None,
            line: 1,
        }
    }

    #[test]
    fn a_type_change_to_needs_decision_flags_the_bead() {
        let rows = vec![row("bd-1", "type", "needs-decision")];
        assert_eq!(needs_decision_from(&rows), vec!["bd-1".to_string()]);
    }

    #[test]
    fn a_later_type_change_away_from_needs_decision_clears_the_flag() {
        let rows = vec![
            row("bd-1", "type", "needs-decision"),
            row("bd-1", "type", "task"),
        ];
        assert!(needs_decision_from(&rows).is_empty());
    }

    #[test]
    fn field_changes_unrelated_to_type_are_ignored() {
        let rows = vec![row("bd-1", "status", "closed")];
        assert!(needs_decision_from(&rows).is_empty());
    }

    // -----------------------------------------------------------------------
    // to_snapshot: reader::Readings -> state::RepoSnapshot, over a minimal
    // real fixture (tempdir + raw writes — tests/support is leased
    // elsewhere this wave, see the bead's setup notes).
    // -----------------------------------------------------------------------

    #[test]
    fn to_snapshot_maps_leases_agents_and_pact_presence_and_leaves_plan_none() {
        let dir = tempfile::tempdir().unwrap();
        let pact = dir.path().join(".pact");
        std::fs::create_dir_all(pact.join("leases")).unwrap();
        std::fs::write(pact.join("events.jsonl"), "").unwrap();
        std::fs::write(
            pact.join("leases").join("src-a.rs.lock"),
            r#"{"agent":"agent-1","path":"src/a.rs","acquired_at":"2026-08-28T11:59:00Z","ttl_secs":3600}"#,
        )
        .unwrap();

        let readings = reader::read(&reader::Options {
            repo_root: dir.path().to_path_buf(),
            use_cursor: false,
        })
        .unwrap();

        let snapshot = to_snapshot(&readings, dir.path());
        assert!(snapshot.pact_present);
        assert!(
            snapshot.plan.is_none(),
            "no plan reader exists yet — see the module doc"
        );
        assert_eq!(snapshot.leases.len(), 1);
        assert_eq!(snapshot.leases[0].agent, "agent-1");
        assert_eq!(snapshot.leases[0].path, "src/a.rs");
    }

    #[test]
    fn a_repository_with_no_pact_at_all_is_not_pact_present() {
        let dir = tempfile::tempdir().unwrap();
        let readings = reader::read(&reader::Options {
            repo_root: dir.path().to_path_buf(),
            use_cursor: false,
        })
        .unwrap();
        assert!(!to_snapshot(&readings, dir.path()).pact_present);
    }

    // -----------------------------------------------------------------------
    // Debouncer, directly.
    // -----------------------------------------------------------------------

    #[test]
    fn debouncer_allows_the_first_occurrence_of_any_event() {
        let d = Debouncer::new(Duration::from_secs(60));
        let event = TransitionEvent::FleetDrained;
        assert!(d.should_notify(Path::new("r"), &event, clock()));
    }

    #[test]
    fn debouncer_blocks_within_the_window_and_allows_at_the_boundary() {
        let mut d = Debouncer::new(Duration::from_secs(60));
        let event = TransitionEvent::FleetDrained;
        let repo = Path::new("r");
        d.record(repo, &event, clock());

        assert!(!d.should_notify(repo, &event, clock() + chrono::TimeDelta::seconds(59)));
        assert!(d.should_notify(repo, &event, clock() + chrono::TimeDelta::seconds(60)));
    }

    // -----------------------------------------------------------------------
    // quivive-trx: an unchanged pass must still re-assess the cached
    // snapshot at the new clock (S5 licenses skipping the re-READ, not the
    // re-assessment) — a repo nobody writes to after the crash must not
    // freeze DEAD/drained forever. Both drive the clock via an injected
    // `now` passed straight to `tick_repo`, never a real sleep.
    // -----------------------------------------------------------------------

    /// A repo with `.pact/leases/` holding one lock for `agent-1`, whose
    /// `acquired_at` (also the lease reader's only source of agent evidence
    /// here) is real wall-clock time at the moment of the write. The caller
    /// captures its own `t0` with `Utc::now()` *after* this returns, so
    /// `t0` is guaranteed to be at or after every mtime this leaves on disk
    /// — the ordering `reader::unchanged`'s own tests rely on too.
    fn repo_with_one_leased_agent() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let pact = dir.path().join(".pact");
        std::fs::create_dir_all(pact.join("leases")).unwrap();
        std::fs::write(pact.join("events.jsonl"), "").unwrap();
        std::fs::write(
            pact.join("leases").join("src-a.rs.lock"),
            format!(
                r#"{{"agent":"agent-1","path":"src/a.rs","acquired_at":"{}","ttl_secs":3600}}"#,
                Utc::now().to_rfc3339(),
            ),
        )
        .unwrap();
        dir
    }

    /// A repo with only `.pact/activity/agent-1`, no lease — so the agent
    /// going DEAD later cannot itself produce a `DeadHoldingPaths` item (S16
    /// needs a held lease) and the only transition a decaying clock can
    /// produce is S19's fleet-drained. See [`repo_with_one_leased_agent`] for
    /// why the timestamp is real wall-clock time rather than a caller-passed
    /// `now`.
    fn repo_with_one_unleased_agent() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let pact = dir.path().join(".pact");
        std::fs::create_dir_all(pact.join("activity")).unwrap();
        std::fs::write(pact.join("events.jsonl"), "").unwrap();
        std::fs::write(
            pact.join("activity").join("agent-1"),
            Utc::now().to_rfc3339(),
        )
        .unwrap();
        dir
    }

    #[test]
    fn a_dead_agent_still_holding_paths_fires_on_a_later_unchanged_pass() {
        let dir = repo_with_one_leased_agent();
        let repo = dir.path().to_path_buf();
        // Captured after every write the fixture makes, so every mtime it
        // left on disk is `<= t0` — the precondition `reader::unchanged`
        // needs to report true below.
        let t0 = Utc::now();

        let thresholds = Thresholds::default();
        let notifier = RecordingNotifier::default();
        let mut debouncer = Debouncer::new(Duration::from_secs(300));
        let mut prune_watermark = HashMap::new();
        let mut snapshot_cache = HashMap::new();
        let mut prev = HashMap::new();

        // Pass 1: establishes the baseline. Fresh evidence -> Active, no
        // attention, so nothing notifies yet (no `prev` to diff against).
        tick_repo(
            &repo,
            t0,
            &thresholds,
            &mut prune_watermark,
            &mut snapshot_cache,
            &mut prev,
            &mut debouncer,
            &notifier,
        )
        .unwrap();
        assert!(
            notifier.sent.borrow().is_empty(),
            "the first pass over any repo must never notify"
        );

        // Sanity-check the fixture: nothing on disk has changed since pass
        // 1, so S5's gate really would skip a re-read here.
        assert!(reader::unchanged(&repo, t0));

        // Pass 2, far past `--dead-window` (30m default), with NO write to
        // the repo in between — the clock alone crosses the boundary.
        let t1 = t0 + chrono::TimeDelta::seconds(3600);
        tick_repo(
            &repo,
            t1,
            &thresholds,
            &mut prune_watermark,
            &mut snapshot_cache,
            &mut prev,
            &mut debouncer,
            &notifier,
        )
        .unwrap();

        // The unchanged branch must not have taken the real-read path: that
        // path is the only place `prune_watermark` is written, so if S5's
        // skip fired, the watermark is still `t0`.
        assert_eq!(
            prune_watermark.get(&repo),
            Some(&t0),
            "an unchanged pass must not re-read (S5) — the watermark only moves on a real read"
        );

        let sent = notifier.sent.borrow();
        assert_eq!(
            sent.len(),
            1,
            "a dead agent still holding a path must notify exactly once, from clock passage alone"
        );
        assert!(sent[0].title.ends_with("— dead agent holding paths"));
        assert!(
            sent[0]
                .body
                .starts_with("agent-1 is dead and still holds 1 path: src/a.rs")
        );
    }

    #[test]
    fn a_drained_fleet_fires_on_a_later_unchanged_pass() {
        let dir = repo_with_one_unleased_agent();
        let repo = dir.path().to_path_buf();
        let t0 = Utc::now();

        let thresholds = Thresholds::default();
        let notifier = RecordingNotifier::default();
        let mut debouncer = Debouncer::new(Duration::from_secs(300));
        let mut prune_watermark = HashMap::new();
        let mut snapshot_cache = HashMap::new();
        let mut prev = HashMap::new();

        tick_repo(
            &repo,
            t0,
            &thresholds,
            &mut prune_watermark,
            &mut snapshot_cache,
            &mut prev,
            &mut debouncer,
            &notifier,
        )
        .unwrap();
        assert!(notifier.sent.borrow().is_empty());
        assert!(reader::unchanged(&repo, t0));

        let t1 = t0 + chrono::TimeDelta::seconds(3600);
        tick_repo(
            &repo,
            t1,
            &thresholds,
            &mut prune_watermark,
            &mut snapshot_cache,
            &mut prev,
            &mut debouncer,
            &notifier,
        )
        .unwrap();

        assert_eq!(
            prune_watermark.get(&repo),
            Some(&t0),
            "an unchanged pass must not re-read (S5) — the watermark only moves on a real read"
        );

        let sent = notifier.sent.borrow();
        assert_eq!(
            sent.len(),
            1,
            "the fleet draining must notify exactly once, from clock passage alone"
        );
        assert_eq!(sent[0].command, "bd ready");
        assert!(sent[0].body.contains("the fleet drained"));
    }
}
