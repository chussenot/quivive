//! The ledger reader: pact's `.pact/events.jsonl`, streamed forward from a
//! cursor.
//!
//! This is the only streamed input vigil has, and the only one with a cursor.
//! The other two surfaces are small and mutable, so a cursor over them would be
//! a cache with nothing to gain (`docs/adr/0001-stream-first-tile.md`).

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::cursor::{self, Cursor};

pub const LEDGER_FILE: &str = "events.jsonl";

/// One row of pact's lease event log, narrowed to the three fields vigil reads.
///
/// serde ignores the rest — `path`, `detail`, `ttl_secs`, `covers_lines`,
/// `actor`, `displaced`, `chain` — and that is deliberate rather than lazy:
/// pact's schema has grown roughly one optional field per kind for years, and a
/// reader that enumerated them all would decline a line for carrying a field it
/// had not heard of. All three fields here are required, so a row missing any of
/// them is a decline and gets counted.
#[derive(Deserialize)]
struct Row {
    at: String,
    agent: String,
    kind: String,
}

/// Does a row of this kind mean *the agent it names was alive at that moment*?
///
/// Almost all of them do, and three do not. This is not a detail that could be
/// guessed from the field names; it comes from pact's own schema documentation in
/// `src/events.rs`, and getting it wrong would make vigil report dead agents as
/// alive — the single most misleading thing it could say.
///
/// * `expired` — a TTL ran out and *the sweeper* wrote the row. The `agent` field
///   names the holder whose claim ended, who by definition did nothing. Counting
///   it as evidence would resurrect exactly the agent that just went quiet.
/// * `displaced` — same shape: the row belongs to the holder whose live claim was
///   overridden, not to whoever overrode it. (The overriding agent gets its own
///   `stolen` row immediately after, which does count.)
/// * `annotation` — a correction pointing at earlier lines, and pact records its
///   author in `actor` rather than `agent`. A human annotating last week's
///   history is not an agent working now.
///
/// Anything else, including a kind this version of vigil has never heard of,
/// counts. That direction is the safe one: a new pact event kind starts working
/// here on the day pact ships it, and the cost of being wrong is reporting an
/// agent as alive one tick longer than it was.
fn counts_as_evidence(kind: &str) -> bool {
    !matches!(kind, "expired" | "displaced" | "annotation")
}

pub struct Reading {
    /// Newest ledger evidence per agent — the folded accumulator.
    pub agents: BTreeMap<String, DateTime<Utc>>,
    /// The cursor to persist for the next tick.
    pub cursor: Cursor,
    /// Lines that could not be parsed into a row. Reported in the tile's
    /// `degraded` list when non-zero, because a decline count nobody knows about
    /// is the defect this reader is most likely to ship.
    pub declined: usize,
    /// True when this was a full re-read rather than a resume.
    pub cold: bool,
    /// False when there is no ledger at all — which is a normal repository, not
    /// a broken one.
    pub present: bool,
}

impl Reading {
    fn absent() -> Self {
        Self {
            agents: BTreeMap::new(),
            cursor: Cursor::empty(),
            declined: 0,
            cold: true,
            present: false,
        }
    }
}

/// Fold the ledger, resuming from `prior` when it still describes the file.
///
/// `prior: None` forces a cold read, which is what `--no-cursor` does and what
/// the control leg of `scripts/fleet-sim.sh` ticks with.
pub fn read(state_dir: &Path, prior: Option<Cursor>) -> Reading {
    let path = state_dir.join(LEDGER_FILE);
    let Ok(mut file) = std::fs::File::open(&path) else {
        return Reading::absent();
    };
    let Ok(meta) = file.metadata() else {
        return Reading::absent();
    };
    // A directory named events.jsonl opens successfully and reads as an error
    // later; deciding here keeps the read loop free of the special case. pact's
    // own test suite creates exactly this shape.
    if !meta.is_file() {
        return Reading::absent();
    }
    let file_len = meta.len();

    // Decide resume-or-cold BEFORE reading anything, and verify rather than
    // trust: the bytes of the last line we claim to have consumed must still be
    // there. See `Cursor::still_describes`.
    let resumed = prior.and_then(|c| {
        if c.offset > file_len || c.tail_len == 0 || c.tail_len > c.offset {
            return None;
        }
        let start = c.offset - c.tail_len;
        let mut buf = vec![0u8; c.tail_len as usize];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut buf).ok()?;
        c.still_describes(file_len, &buf).then_some(c)
    });

    // The prior tail is kept, not just the offset: on a tick that consumes
    // nothing — an idle fleet, which is most ticks — the cursor written below
    // must carry the SAME tail forward. Dropping it there left the next tick
    // unable to verify its own resume point, so every tick after the first quiet
    // one degraded to a full re-read: a cursor that silently stops working
    // exactly when the fleet is calm.
    let (mut agents, start_at, prior_tail, cold) = match resumed {
        Some(c) => (c.agents, c.offset, (c.tail_len, c.tail_hash), false),
        None => (BTreeMap::new(), 0, (0, 0), true),
    };

    if file.seek(SeekFrom::Start(start_at)).is_err() {
        return Reading::absent();
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return Reading::absent();
    }

    // Consume only up to the LAST newline. The ledger is written by another
    // process, so a tick can arrive mid-append and see half a line; advancing
    // the cursor over it would drop the rest of that event forever, and the tile
    // would be wrong from then until the next rewrite. This is the single most
    // likely bug in the crate and the reason the cursor stores a tail at all.
    let consumable = match buf.iter().rposition(|b| *b == b'\n') {
        Some(i) => i + 1,
        None => 0,
    };

    let mut declined = 0usize;
    let mut last_line: &[u8] = &[];
    for line in buf[..consumable].split_inclusive(|b| *b == b'\n') {
        last_line = line;
        let text = line.trim_ascii();
        // A blank line is not a decline. pact's rewrite can leave one, and
        // counting it as damage would report corruption in a healthy repository.
        if text.is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_slice::<Row>(text) else {
            declined += 1;
            continue;
        };
        if !counts_as_evidence(&row.kind) {
            continue;
        }
        let Ok(at) = DateTime::parse_from_rfc3339(&row.at) else {
            declined += 1;
            continue;
        };
        let at = at.to_utc();
        // Newest wins, decided by timestamp and never by position in the file.
        // The fold must not care what order it sees events in, or the cold and
        // warm paths disagree the moment a ledger is written out of order — and
        // pact does not promise it never is.
        agents
            .entry(row.agent)
            .and_modify(|seen| {
                if at > *seen {
                    *seen = at;
                }
            })
            .or_insert(at);
    }

    let (offset, tail_len, tail_hash) = if last_line.is_empty() {
        // Nothing new was consumed. Carry the prior position AND the prior tail
        // forward unchanged, rather than inventing a tail we did not read or
        // discarding the one that is still true.
        (start_at, prior_tail.0, prior_tail.1)
    } else {
        (
            start_at + consumable as u64,
            last_line.len() as u64,
            cursor::hash(last_line),
        )
    };
    let cursor = Cursor {
        v: 1,
        offset,
        tail_len,
        tail_hash,
        agents: agents.clone(),
    };

    Reading {
        agents,
        cursor,
        declined,
        cold,
        present: true,
    }
}
