//! The sidecar reader: bd's committed `.beads/interactions.jsonl`, when the
//! audit export is enabled.
//!
//! Same shape of file as `.pact/events.jsonl` — append-only, one JSON object
//! per line — and pact's own reader of it (`beads::interaction_actors` et al.)
//! is deliberately parse-*tolerant* where its own `plan lint` manifest reader is
//! not, because this file is somebody else's export and a partial answer beats
//! none. This reader follows the same rule, for the same file.
//!
//! ## Why no cursor
//!
//! It is append-only like the ledger, so a cursor is *possible* here in a way
//! it is not for leases or activity — but it is not *needed*. bd's audit
//! sidecar is opt-in and, measured against real repositories, small: a few
//! hundred rows for a fleet's whole history, not the tens of thousands
//! `docs/adr/0001-stream-first-tile.md` built a cursor to avoid re-parsing
//! every second. Reading it whole every tick is the "read the whole ledger
//! every tick" alternative that ADR rejected for the *events* log on cost —
//! rejected there, not here, because the file this reader reads is two orders
//! of magnitude smaller. Revisit if a fleet ever makes that stop being true.
//!
//! ## What counts as a "row" here
//!
//! bd exports every field change as `kind: "field_change"` — there is no
//! `created` or `closed` kind, a close is `field=status, new_value=closed`
//! the same way a bead being filed shows up as whichever change the caller's
//! `bd` version emits on creation. This reader does not decide which kind or
//! field means "a needs-decision bead was filed" (`S17`) — that judgement
//! belongs to whichever bead consumes `Readings.interactions` — so every row
//! that parses is surfaced, narrowed to the fields common to all of them.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;

pub const SIDECAR_FILE: &str = "interactions.jsonl";

/// bd's shape, narrowed to what vigil reads. `id` (bd's own row id) is not
/// carried forward: the 1-based line number in [`Row::line`] is what `why`
/// (`S21`) cites as the evidence location, in the same "file plus line" form
/// the ledger's declines are reported in, and it is vigil's own coordinate
/// rather than a fact bd recorded.
#[derive(Deserialize)]
struct Raw {
    issue_id: String,
    kind: String,
    actor: String,
    created_at: String,
    #[serde(default)]
    extra: Extra,
}

/// `extra` is a grab-bag in bd's export — `field`/`new_value`/`old_value` for a
/// `field_change`, and potentially other keys for a kind this reader has never
/// seen. All three are optional and unknown keys are ignored, the same
/// tolerance `ledger::Row` extends to pact's own event shape.
#[derive(Deserialize, Default)]
struct Extra {
    field: Option<String>,
    new_value: Option<String>,
    old_value: Option<String>,
}

/// One parsed row of the sidecar, naming a bead and what changed.
pub struct Row {
    pub issue_id: String,
    pub kind: String,
    pub actor: String,
    pub at: DateTime<Utc>,
    pub field: Option<String>,
    pub new_value: Option<String>,
    pub old_value: Option<String>,
    /// 1-based line in `.beads/interactions.jsonl`, for citing this row as
    /// evidence the way `S21` requires.
    pub line: usize,
}

pub struct Reading {
    pub rows: Vec<Row>,
    /// Lines that were not blank but did not parse into a [`Row`] — a
    /// malformed line, or one missing a required field. Counted, never fatal,
    /// matching every other reader's rule that a decline nobody knows about is
    /// the defect most likely to ship silently.
    pub declined: usize,
    /// False when there is no sidecar at all: bd's audit export is opt-in and
    /// off by default (`BD_AUDIT_ENABLED`/`bd config set audit.enabled`), so
    /// most repositories never have this file. Normal, not a fault — `S17`
    /// simply has nothing to feed from.
    pub present: bool,
}

pub fn read(repo_root: &Path) -> Reading {
    let path = repo_root.join(".beads").join(SIDECAR_FILE);
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Reading {
            rows: Vec::new(),
            declined: 0,
            present: false,
        };
    };

    let mut rows = Vec::new();
    let mut declined = 0usize;
    for (i, line) in contents.lines().enumerate() {
        let text = line.trim();
        // A blank line is not a decline — the same allowance `ledger::read`
        // makes for pact's own compaction leaving one.
        if text.is_empty() {
            continue;
        }
        let Ok(raw) = serde_json::from_str::<Raw>(text) else {
            declined += 1;
            continue;
        };
        let Ok(at) = DateTime::parse_from_rfc3339(&raw.created_at) else {
            declined += 1;
            continue;
        };
        rows.push(Row {
            issue_id: raw.issue_id,
            kind: raw.kind,
            actor: raw.actor,
            at: at.to_utc(),
            field: raw.extra.field,
            new_value: raw.extra.new_value,
            old_value: raw.extra.old_value,
            line: i + 1,
        });
    }

    Reading {
        rows,
        declined,
        present: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beads_dir(dir: &Path) -> std::path::PathBuf {
        let d = dir.join(".beads");
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_missing_sidecar_is_absent_not_declined() {
        let dir = tempfile::tempdir().unwrap();
        let r = read(dir.path());
        assert!(!r.present);
        assert_eq!(r.declined, 0);
        assert!(r.rows.is_empty());
    }

    const ROW_1: &str = r#"{"id":"int-1","kind":"field_change","created_at":"2026-08-25T00:00:00Z","actor":"a","issue_id":"proj-1","extra":{"field":"status","new_value":"closed","old_value":"open"}}"#;
    const ROW_2: &str = r#"{"id":"int-2","kind":"field_change","created_at":"2026-08-25T00:01:00Z","actor":"b","issue_id":"proj-2","extra":{"field":"type","new_value":"needs-decision"}}"#;

    #[test]
    fn a_field_change_row_parses_with_its_line_number() {
        let dir = tempfile::tempdir().unwrap();
        let d = beads_dir(dir.path());
        std::fs::write(d.join(SIDECAR_FILE), format!("{ROW_1}\n{ROW_2}\n")).unwrap();
        let r = read(dir.path());
        assert!(r.present);
        assert_eq!(r.declined, 0);
        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.rows[0].issue_id, "proj-1");
        assert_eq!(r.rows[0].line, 1);
        assert_eq!(r.rows[1].field.as_deref(), Some("type"));
        assert_eq!(r.rows[1].new_value.as_deref(), Some("needs-decision"));
        assert_eq!(r.rows[1].line, 2);
    }

    #[test]
    fn a_blank_line_is_not_a_decline() {
        let dir = tempfile::tempdir().unwrap();
        let d = beads_dir(dir.path());
        std::fs::write(d.join(SIDECAR_FILE), format!("\n{ROW_1}\n\n")).unwrap();
        let r = read(dir.path());
        assert_eq!(r.declined, 0);
        assert_eq!(r.rows.len(), 1);
    }

    #[test]
    fn garbage_lines_are_declined_and_the_rest_still_reads() {
        let dir = tempfile::tempdir().unwrap();
        let d = beads_dir(dir.path());
        let bad_timestamp = r#"{"issue_id":"proj-3","kind":"field_change","actor":"c","created_at":"not a date","extra":{}}"#;
        std::fs::write(
            d.join(SIDECAR_FILE),
            format!("{{ not json\n{ROW_1}\n{bad_timestamp}\n"),
        )
        .unwrap();
        let r = read(dir.path());
        assert_eq!(r.declined, 2, "one unparsable line, one bad timestamp");
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0].issue_id, "proj-1");
    }
}
