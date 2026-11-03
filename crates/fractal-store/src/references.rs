//! References that do not resolve: a pinned mission revision that is not stored, or a mission,
//! agent or observer that nothing declares. Publication refuses them, and `st doctor` lists the
//! ones already in the graph.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{
    DesiredSubject, MissionInputKind, MissionSpec, NormalizedIntent, St3Error, UsedMissionSpec,
    WorkSelector,
};

/// What the graph holds, as far as the reference checks need it.
pub trait Graph {
    /// Whether `subject` has a current declaration.
    fn declared(&self, subject: &str) -> Result<bool, St3Error>;
    /// Whether any revision of the mission is published.
    fn mission_published(&self, mission: &str) -> Result<bool, St3Error>;
    /// The stored revision of the mission.
    fn mission_revision(
        &self,
        mission: &str,
        revision: &str,
    ) -> Result<Option<MissionSpec>, St3Error>;
    /// Why `mission/MISSION@REVISION` does not resolve.
    fn missing_revision(&self, mission: &str, revision: &str) -> Result<String, St3Error>;
}

/// The run a mission's declarations are checked under, so a step can select an agent that another
/// step or the mission declares. Each mission is checked with only its own run's declarations.
const PROOF_RUN: &str = "migration-proof";

/// What a variable whose value only a run knows is replaced with. A reference that contains it
/// cannot be checked before the run and is skipped.
const RUN_VALUE: &str = "st-run-value";

/// Every reference in the graph's ready missions and current declarations that no longer
/// resolves. A mission's loop rounds are checked with it, since they share its run's
/// declarations. Messages are history and are not checked.
pub fn graph(
    missions: &[MissionSpec],
    desired: &[DesiredSubject],
    graph: &impl Graph,
    host: &str,
) -> Result<Vec<String>, St3Error> {
    let mut refusals = Vec::new();
    for run in runs(&missions.iter().collect::<Vec<_>>()) {
        let scope = Scope {
            run: run_declarations(&run, host),
            intent: None,
        };
        for mission in run {
            mission_references(mission, &scope, graph, host, &mut refusals)?;
        }
    }
    let scope = Scope {
        run: BTreeSet::new(),
        intent: None,
    };
    for desired in desired.iter().filter(|desired| desired.kind != "message") {
        subject_references(desired, None, &scope, graph, host, &mut refusals)?;
    }
    refusals.sort();
    refusals.dedup();
    Ok(refusals)
}

/// The missions whose declarations one run shares: each mission with the loop rounds embedded in
/// it. Separate missions start separate runs, so one never resolves another's run-scoped
/// declarations. An embedded mission without its owner is checked alone.
pub fn runs<'a>(missions: &[&'a MissionSpec]) -> Vec<Vec<&'a MissionSpec>> {
    let mut grouped = BTreeSet::new();
    let mut runs = Vec::new();
    for owner in missions
        .iter()
        .filter(|mission| !mission.id.starts_with("__st3/"))
    {
        let embedded = format!("__st3/{}/loop/", owner.id);
        let run = missions
            .iter()
            .copied()
            .filter(|mission| mission.id == owner.id || mission.id.contains(&embedded))
            .collect::<Vec<_>>();
        grouped.extend(run.iter().map(|mission| mission.id.as_str()));
        runs.push(run);
    }
    runs.extend(
        missions
            .iter()
            .copied()
            .filter(|mission| !grouped.contains(mission.id.as_str()))
            .map(|mission| vec![mission]),
    );
    runs
}

pub struct Scope<'a> {
    /// The subjects the missions' own declarations create in a run, under [`PROOF_RUN`].
    pub run: BTreeSet<String>,
    /// The publication being checked, whose subjects and missions resolve references too.
    pub intent: Option<&'a NormalizedIntent>,
}

impl<'a> Scope<'a> {
    pub fn empty(intent: &'a NormalizedIntent) -> Self {
        Self {
            run: BTreeSet::new(),
            intent: Some(intent),
        }
    }

    fn subject(&self, graph: &impl Graph, subject: &str) -> Result<bool, St3Error> {
        Ok(subject.contains(RUN_VALUE)
            || self.run.contains(subject)
            || self
                .intent
                .is_some_and(|intent| intent.subjects.contains_key(subject))
            || graph.declared(subject)?)
    }

    fn mission(&self, graph: &impl Graph, mission: &str) -> Result<bool, St3Error> {
        Ok(self
            .intent
            .is_some_and(|intent| intent.missions.contains_key(mission))
            || graph.mission_published(mission)?)
    }

    fn revision(
        &self,
        graph: &impl Graph,
        mission: &str,
        revision: &str,
    ) -> Result<Option<MissionSpec>, St3Error> {
        if let Some(published) = self
            .intent
            .and_then(|intent| intent.missions.get(mission))
            .filter(|published| published.revision == revision)
        {
            return Ok(Some(published.clone()));
        }
        graph.mission_revision(mission, revision)
    }
}

/// The variables a mission's declarations are checked with. The run's identity is fixed, so
/// references between its declarations resolve; every other value is only known to a run.
fn proof_variables(mission: &MissionSpec) -> BTreeMap<String, String> {
    const RUN_IDENTITY: &[&str] = &[
        "ST_MISSION",
        "ST_MISSION_REVISION",
        "ST_MISSION_RUN",
        "ST_RUN_GENERATION",
        "ST_ROOT_MISSION_RUN",
        "ST_ROOT_MISSION_RUN_ID",
        "PATH",
    ];
    crate::graph::runtime_proof_variables(mission)
        .into_iter()
        .map(|(name, value)| {
            if RUN_IDENTITY.contains(&name.as_str()) {
                (name, value)
            } else {
                let value = value.replace("migration", RUN_VALUE);
                (name, value)
            }
        })
        .collect()
}

/// Each declarations block of the mission and its nested missions, with the mission it belongs
/// to.
fn declaration_sources(mission: &MissionSpec) -> Vec<(&MissionSpec, &str)> {
    let mut sources = mission
        .declarations_kdl
        .iter()
        .map(|source| (mission, source.as_str()))
        .collect::<Vec<_>>();
    for step in mission.steps.values() {
        if let Some(source) = &step.declarations_kdl {
            sources.push((mission, source));
        }
        if let Some(nested) = &step.nested_mission {
            sources.extend(declaration_sources(nested));
        }
    }
    sources
}

/// One declarations block as a run would read it, or `None` when it only parses with values a
/// run supplies. Publication already refused a block that cannot parse at all.
fn run_intent(mission: &MissionSpec, source: &str, host: &str) -> Option<NormalizedIntent> {
    let source = crate::mission::interpolate_kdl(source, &proof_variables(mission)).ok()?;
    crate::graph::parse_execution_intent(&source, host, PROOF_RUN).ok()
}

pub fn run_declarations(missions: &[&MissionSpec], host: &str) -> BTreeSet<String> {
    missions
        .iter()
        .flat_map(|mission| declaration_sources(mission))
        .filter_map(|(mission, source)| run_intent(mission, source, host))
        .flat_map(|intent| intent.subjects.into_keys())
        .collect()
}

/// A subject as its author wrote it inside a mission.
fn authored(subject: &str) -> String {
    subject.replace(PROOF_RUN, "${ST_MISSION_RUN}")
}

pub fn mission_references(
    mission: &MissionSpec,
    scope: &Scope<'_>,
    graph: &impl Graph,
    host: &str,
    refusals: &mut Vec<String>,
) -> Result<(), St3Error> {
    let variables = proof_variables(mission);
    let mut selectors = mission.work_selector.iter().collect::<Vec<_>>();
    let mut steps = mission.steps.values().collect::<Vec<_>>();
    while let Some(step) = steps.pop() {
        selectors.extend(step.work_selector.iter());
        if let Some(UsedMissionSpec::Revision {
            mission: used,
            revision,
        }) = &step.uses_mission
            && scope.revision(graph, used, revision)?.is_none()
        {
            refusals.push(format!(
                "step `{}` of mission `{}` uses a mission that is not stored: {}",
                step.path,
                mission.subject,
                graph.missing_revision(used, revision)?
            ));
        }
        if let Some(nested) = &step.nested_mission {
            selectors.extend(nested.work_selector.iter());
            steps.extend(nested.steps.values());
        }
    }
    for selector in selectors {
        let agents = match selector {
            WorkSelector::Assigned { agent } => std::slice::from_ref(agent),
            WorkSelector::Available { agents } => agents.as_slice(),
            WorkSelector::Agentless => &[],
        };
        for agent in agents {
            // A selector that names a variable a run supplies cannot be checked before the run.
            let Ok(resolved) = crate::mission::interpolate(agent, &variables) else {
                continue;
            };
            if !resolved.starts_with("person/")
                && !mission.revision_owners.contains(agent)
                && !scope.subject(graph, &resolved)?
            {
                refusals.push(format!(
                    "mission `{}` references missing eligible agent `{agent}`",
                    mission.subject
                ));
            }
        }
    }
    for (owner, source) in declaration_sources(mission) {
        let Some(run) = run_intent(owner, source, host) else {
            continue;
        };
        for desired in run.subjects.values() {
            subject_references(desired, Some(mission), scope, graph, host, refusals)?;
        }
    }
    Ok(())
}

/// The references one declaration names. `mission` is the mission whose declarations hold it,
/// when it belongs to a run.
pub fn subject_references(
    desired: &DesiredSubject,
    mission: Option<&MissionSpec>,
    scope: &Scope<'_>,
    graph: &impl Graph,
    host: &str,
    refusals: &mut Vec<String>,
) -> Result<(), St3Error> {
    let owner = mission.map_or(String::new(), |mission| {
        format!("mission `{}` declares ", mission.subject)
    });
    let subject = authored(&desired.subject);
    match desired.kind.as_str() {
        "agent" => {
            for under in crate::graph::agent_under(&desired.desired) {
                if under.agent != desired.subject && !scope.subject(graph, &under.agent)? {
                    refusals.push(format!(
                        "{owner}agent `{subject}` is grouped under missing agent `{}`",
                        authored(&under.agent)
                    ));
                }
            }
        }
        "message" => {
            if let Some(to) = crate::model::canonical_child_string(&desired.desired, "to") {
                let recipient = if to == "requester" || to.contains('/') {
                    to
                } else {
                    format!("agent/{to}")
                };
                if recipient != "requester"
                    && !recipient.starts_with("person/")
                    && !scope.subject(graph, &recipient)?
                {
                    refusals.push(format!(
                        "{owner}message `{subject}` references undeclared recipient `{}`",
                        authored(&recipient)
                    ));
                }
            }
        }
        "subscription" => {
            let Some(subscription) = crate::graph::subscription_spec(&desired.desired) else {
                return Ok(());
            };
            if subscription.stopped {
                return Ok(());
            }
            // st declares a repository's standing observer for the first subscription to it.
            if crate::github_watch::standing_locator(&subscription.observer).is_none()
                && !scope.subject(graph, &subscription.observer)?
            {
                let scoped = subscription
                    .observer
                    .strip_prefix("observer/")
                    .map(|local| format!("observer/{PROOF_RUN}/{local}"))
                    .filter(|scoped| scope.run.contains(scoped));
                refusals.push(match scoped {
                    // A declarations block names its own observers by their short name; another
                    // block's observer needs the run in its name.
                    Some(scoped) => format!(
                        "{owner}subscription `{subject}` references observer `{}`, which another block declares; name it `{}`",
                        subscription.observer,
                        authored(&scoped)
                    ),
                    None => format!(
                        "{owner}subscription `{subject}` references missing observer `{}`",
                        authored(&subscription.observer)
                    ),
                });
            }
            if subscription.delivery == "message"
                && !subscription.to.is_empty()
                && subscription.to != "requester"
                && !subscription.to.starts_with("person/")
                && !scope.subject(graph, &subscription.to)?
            {
                refusals.push(format!(
                    "{owner}subscription `{subject}` has missing delivery target `{}`",
                    authored(&subscription.to)
                ));
            }
            if subscription.delivery == "mission"
                && let Some(target) = subscription.mission.as_deref()
            {
                match subscription.revision.as_deref() {
                    Some(revision) => match scope.revision(graph, target, revision)? {
                        None => refusals.push(format!(
                            "{owner}subscription `{subject}` delivers to a mission that is not stored: {}",
                            graph.missing_revision(target, revision)?
                        )),
                        Some(published) => {
                            if let Some(input) = subscription.resource_input.as_deref()
                                && published.inputs.get(input).is_none_or(|declaration| {
                                    declaration.kind != MissionInputKind::Resource
                                })
                            {
                                refusals.push(format!(
                                    "{owner}subscription `{subject}` uses undeclared resource input `{input}` in mission/{target}@{revision}"
                                ));
                            }
                        }
                    },
                    None => {
                        if !scope.mission(graph, target)? {
                            refusals.push(format!(
                                "{owner}subscription `{subject}` references unpublished mission `mission/{target}`"
                            ));
                        }
                    }
                }
            }
        }
        "schedule" => {
            let Some(schedule) = crate::graph::schedule_spec(&desired.desired, host) else {
                return Ok(());
            };
            if let Some(work) = schedule.work.filter(|_| !schedule.stopped) {
                match work.revision.as_deref() {
                    Some(revision) if scope.revision(graph, &work.mission, revision)?.is_none() => {
                        refusals.push(format!(
                            "{owner}schedule `{subject}` runs a mission that is not stored: {}",
                            graph.missing_revision(&work.mission, revision)?
                        ));
                    }
                    None if !scope.mission(graph, &work.mission)? => {
                        refusals.push(format!(
                            "{owner}schedule `{subject}` references unpublished mission `mission/{}`",
                            work.mission
                        ));
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    Ok(())
}


#[cfg(any(test, feature = "test-support"))]
pub mod seats_shared_tests;
