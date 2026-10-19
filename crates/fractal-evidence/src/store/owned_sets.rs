//! The part of `store::owned_sets` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use super::*;
use crate::model::MemberSpec;
use serde::Deserialize;
use std::cell::RefCell;
pub use fractal_attention::store::owned_sets::*;
pub use fractal_attention::store::owned_sets::{Member, Options, Plan, Preview, Request, Revision, Source, View, claim, commit_tx, conflicts_at, desired_at};
pub use fractal_attention::store::owned_sets::{effective_members, guard_member, guard_mission_start, manual_member, manual_member_at, one_shot_retirement_owner, owner, owners_at, plan_tx, project_tx, refuse_unmanaged, selected};
pub use fractal_attention::store::owned_sets::{subject, validate_apply, validate_receipt};

pub const STAGED_SUBJECTS_QUERY: &str =
    "SELECT DISTINCT subject FROM claims WHERE json_extract(body,'$.owned_set') IS NOT NULL";

pub trait OwnedSetsStore {
    fn owned_sets(&self) -> Result<Vec<View>, St3Error>;
    /// Snapshot-bound retirement authority for client agent terminal projections.
    fn retired_owned_subjects_at(&self, at: u64) -> Result<BTreeSet<String>, St3Error>;
    /// Known replica names from graph membership, without sealing batches during a read.
    fn owned_set_replica_names(&self) -> Result<Vec<String>, St3Error>;
    fn owned_set_history(&self, set: &str) -> Result<Vec<View>, St3Error>;
    fn owned_set_effective_members(
            &self,
            view: &View,
        ) -> Result<Vec<(String, Member, bool)>, St3Error>;
    fn owned_set_preview(
            &self,
            intent: &NormalizedIntent,
            options: &Options,
        ) -> Result<Preview, St3Error>;
    fn owned_member_guard(&self, s: &str) -> Result<(), St3Error>;
    /// Select a pass's candidates from one authority snapshot. This is not an effect fence:
    /// callers must still check current authority before workspace, render or runtime effects.
    fn owned_desired_subjects(
            &self,
            desired: &[DesiredSubject],
        ) -> Result<BTreeSet<String>, St3Error>;
    fn apply_owned_set(
            &self,
            intent: &NormalizedIntent,
            options: &Options,
            key: &str,
            actor: &str,
        ) -> Result<ApplyResponse, St3Error>;
}

impl OwnedSetsStore for Store {

    fn owned_sets(&self) -> Result<Vec<View>, St3Error> {
        selected(&self.readers.get(), None)
    }

    fn retired_owned_subjects_at(&self, at: u64) -> Result<BTreeSet<String>, St3Error> {
        let connection = self.readers.get();
        let mut retired = BTreeSet::new();
        for view in selected(&connection, Some(at))? {
            if !view.blockers.is_empty() { continue; }
            for (subject, (_, is_retired, _)) in effective_members(&connection, &view, Some(at))? {
                if is_retired { retired.insert(subject); }
            }
        }
        Ok(retired)
    }

    fn owned_set_replica_names(&self) -> Result<Vec<String>, St3Error> {
        let membership = fleet_membership_tx(&self.readers.get()).map_err(internal)?;
        Ok(membership
            .incarnations()
            .filter(|m| m.end.is_none())
            .map(|m| m.name.clone())
            .collect())
    }

    fn owned_set_history(&self, set: &str) -> Result<Vec<View>, St3Error> {
        let subject = subject(set)?;
        Ok(rows(&self.readers.get(), None)?
            .into_iter()
            .filter(|v| v.id == subject)
            .collect())
    }

    fn owned_set_effective_members(
            &self,
            view: &View,
        ) -> Result<Vec<(String, Member, bool)>, St3Error> {
        Ok(effective_members(&self.readers.get(), view, None)?
            .into_iter()
            .map(|(s, (m, retired, _))| (s, m, retired))
            .collect())
    }

    fn owned_set_preview(
            &self,
            intent: &NormalizedIntent,
            options: &Options,
        ) -> Result<Preview, St3Error> {
        let mut preview = self.connection
            .batched(|tx| plan_tx(tx, intent, options).map(|p| p.preview))
            .map_err(|e| St3Error::new("internal", e))??;
        // Operational annotations belong to preview only. Publication keeps its existing
        // transaction reads and suspension follows the shared launch-lineage comparator.
        for (subject, effect) in &mut preview.effects {
            let Some(next) = intent.subjects.get(subject) else { continue; };
            if let Some(action) = crate::suspension::publication_action(
                self, next, true, || {
                    Ok(self.desired_subject_with_writer(subject)?
                        .and_then(|(desired, _)| desired.member))
                },
            ).map_err(internal)? {
                effect.push_str("; ");
                effect.push_str(&action.reason);
            }
        }
        Ok(preview)
    }

    fn owned_member_guard(&self, s: &str) -> Result<(), St3Error> {
        guard_member(&self.readers.get(), s)
    }

    fn owned_desired_subjects(
            &self,
            desired: &[DesiredSubject],
        ) -> Result<BTreeSet<String>, St3Error> {
        if desired.is_empty() {
            return Ok(BTreeSet::new());
        }
        for declaration in desired {
            smallclaims::touched::note_read(|| declaration.subject.clone());
        }
        self.read_snapshot(|_| {
            self.with_owned_set_snapshot_reads(|| {
                let connection = self.readers.get();
                let history = rows(&connection, None)?;
                let mut terminal = BTreeSet::new();
                for declaration in desired.iter().filter(|d| d.desired.get("retirement").is_some()) {
                    if retirements::terminal_selection(&connection, declaration)? == Some(true) {
                        terminal.insert(declaration.subject.clone());
                    }
                }
                let mut owners = BTreeMap::<String, BTreeSet<String>>::new();
                for view in &history {
                    for subject in view
                        .receipt
                        .members
                        .keys()
                        .chain(view.receipt.retired.keys())
                    {
                        owners
                            .entry(subject.clone())
                            .or_default()
                            .insert(view.id.clone());
                    }
                }
                // The partial index created at open contains only staged owned declarations.
                // Let the planner choose: a missing index must not fence otherwise valid work.
                // Empty ownership uses one receipt and one staged-subject query for any roster;
                // statement count alone does not bound rows or CPU for nonempty history.
                let staged = connection
                    .prepare(/* sqlite-large-value: allow-for-now lv-h99958058d413 owner=fractal_evidence::store::owned_sets TODO=trace-statement-and-value-representation */ STAGED_SUBJECTS_QUERY)
                    .map_err(internal)?
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(internal)?
                    .collect::<Result<BTreeSet<_>, _>>()
                    .map_err(internal)?;
                // Keep failures local to owned candidates, as the individual guard does.
                // An unowned subject does not depend on selected/effective membership.
                let selected = selected(&connection, None);
                let members = selected
                    .as_ref()
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter(|view| view.blockers.is_empty())
                    .map(|view| (view.id.clone(), effective_members(&connection, view, None)))
                    .collect::<BTreeMap<_, _>>();
                let eligible = desired
                    .iter()
                    .filter(|declaration| {
                        if declaration.desired.get("retirement").is_some() {
                            return terminal.contains(&declaration.subject);
                        }
                        let Some(sets) = owners.get(&declaration.subject) else {
                            return !staged.contains(&declaration.subject);
                        };
                        if sets.len() != 1 {
                            return false;
                        }
                        let set = sets.first().expect("one owner");
                        members.get(set).is_some_and(|members| {
                            members.as_ref().is_ok_and(|members| {
                                members
                                    .get(&declaration.subject)
                                    .is_some_and(|(member, _, _)| {
                                        member.revision == desired_revision(declaration)
                                    })
                            })
                        })
                    })
                    .map(|declaration| declaration.subject.clone())
                    .collect();
                Ok(eligible)
            })
        })
        .map_err(internal)
    }

    fn apply_owned_set(
            &self,
            intent: &NormalizedIntent,
            options: &Options,
            key: &str,
            actor: &str,
        ) -> Result<ApplyResponse, St3Error> {
        let key = format!("owned-set:{}:{key}", options.set);
        self.apply_as_impl(
            intent,
            &options.expected_subjects,
            &key,
            Some(actor),
            None,
            publication::CommitChecks {
                owned: Some(options),
                guard: None,
            },
        )
    }
}

