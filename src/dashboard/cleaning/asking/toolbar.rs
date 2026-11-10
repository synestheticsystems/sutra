//! What happens when the user clicks the toolbar's clean: working out on a thread of its own what
//! it offers, then one question about every idle folder and, apart from it and after it, one about
//! the folders kept only by sessions at rest, each answered on its own. A session keeps its folders
//! from everything Sutra does by itself, so that second question is the one way they go.

use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use super::super::queue::By;
use super::super::{Cleaning, OFFER, Report, words};
use super::Asked;
use crate::dashboard::state::{Alert, Proposal};
use crate::map::Snapshot;
use crate::map::clean::{self, AtRestClean, CargoEnv, IdleClean, LeftOut, Offered};

impl Cleaning {
    /// The user clicked the toolbar's clean. Work out the clean of every idle folder, and apart from
    /// them of every folder kept only by sessions at rest, on a thread of its own, as `map` shows
    /// them: `asking_idle` takes it from there.
    pub(in crate::dashboard) fn start_idle(
        &mut self,
        map: Arc<Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        // A second click while the first is worked out or asked about would only ask again.
        let asking = [Asked::CleanIdle, Asked::CleanAtRest];
        if self.preparing_idle || asking.iter().any(|asked| self.questions.is_open(asked)) {
            return;
        }
        // The toolbar's offer leaves out each folder whose card shows a clean of its own, and so
        // do the questions that follow it.
        let being_cleaned = self.being_cleaned();
        let env = CargoEnv::of_this_process(home);
        let preparing = self.threads.run(
            "sutra-clean-prepare",
            move || clean::prepare_offer(&map, now, OFFER, &being_cleaned, &env),
            Report::OfferPrepared,
        );
        if let Err(err) = preparing {
            let stopped = Alert::notice(words::IDLE_SEARCH_STOPPED, err.to_string());
            self.alerts.push(stopped);
            return;
        }
        self.preparing_idle = true;
    }

    /// What the toolbar's clean offers has been worked out, or its thread ended with no word
    /// (`None`). Ask about the idle folders, naming how many and how much, then apart about those
    /// kept only by sessions at rest; `answered_idle` and `answered_at_rest` take the answers. With
    /// nothing for either, say so, and why.
    pub(in crate::dashboard::cleaning) fn asking_idle(&mut self, found: Option<Offered>) {
        self.preparing_idle = false;
        let Some(Offered {
            mut idle,
            mut at_rest,
        }) = found
        else {
            let (title, detail) = words::search_stopped();
            self.alerts.push(Alert::notice(title, detail));
            return;
        };
        // A question opened while these were worked out, a card's or the first one here, keeps the
        // folders it asks about.
        idle.cleans.retain(|clean| {
            (self.questions)
                .asked_about_in(&clean.clicked, &clean.prepared)
                .is_none()
        });
        let asks_idle = !idle.cleans.is_empty();
        if asks_idle {
            let question = words::idle_question(&idle.cleans, &idle.left_out, OFFER.quiet);
            for clean in &idle.cleans {
                (self.questions).open(&Asked::CleanIdle, &clean.clicked, &clean.prepared);
            }
            let proposal = Proposal::CleanIdle(idle.cleans);
            self.alerts.push(Alert::question(question, proposal));
        }
        at_rest.cleans.retain(|clean| {
            (self.questions)
                .asked_about_in(&clean.clicked, &clean.prepared)
                .is_none()
        });
        let asks_at_rest = !at_rest.cleans.is_empty();
        if asks_at_rest {
            let question =
                words::at_rest_question(&at_rest.cleans, &at_rest.left_out, OFFER.at_rest);
            for clean in &at_rest.cleans {
                (self.questions).open(&Asked::CleanAtRest, &clean.clicked, &clean.prepared);
            }
            let proposal = Proposal::CleanAtRest(at_rest.cleans);
            self.alerts.push(Alert::question(question, proposal));
        }
        if !asks_idle && !asks_at_rest {
            let left_out: Vec<LeftOut> =
                idle.left_out.into_iter().chain(at_rest.left_out).collect();
            let (title, detail) = words::nothing_to_clean(&left_out);
            self.alerts.push(Alert::notice(title, detail));
        }
    }

    /// The user answered the question about every idle folder, with `cleans` to go on a yes. Either
    /// way the question lets its folders go. Each clean looks at whether its project is still idle
    /// when its turn comes, as `map` shows it then.
    pub(in crate::dashboard) fn answered_idle(
        &mut self,
        cleans: Option<Vec<IdleClean>>,
        map: Option<&Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        self.questions.answered(&Asked::CleanIdle);
        let Some(cleans) = cleans else {
            return;
        };
        for clean in cleans {
            let by = By::UserForIdle {
                project: clean.project_path,
            };
            self.queue(clean.clicked, clean.prepared, by);
        }
        self.start_next(map, now, home);
    }

    /// The user answered the question about the folders kept only by sessions at rest, with
    /// `cleans` to go on a yes. Either way the question lets its folders go. Each clean looks at
    /// whether the sessions on its project still rest, and nothing else started there, when its
    /// turn comes, as `map` shows it then.
    pub(in crate::dashboard) fn answered_at_rest(
        &mut self,
        cleans: Option<Vec<AtRestClean>>,
        map: Option<&Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        self.questions.answered(&Asked::CleanAtRest);
        let Some(cleans) = cleans else {
            return;
        };
        for clean in cleans {
            let by = By::UserForAtRest {
                project: clean.project_path,
            };
            self.queue(clean.clicked, clean.prepared, by);
        }
        self.start_next(map, now, home);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::super::tests::{idle_project, reaching};
    use super::*;
    use crate::dashboard::cleaning::tests::{
        ALWAYS_SHORT, at_rest_clean, cleaning, idle_clean, next_report, offered, prepared_at,
        reporting,
    };
    use crate::map::clean::{IdleCleans, PrepareError, WORTH_CLEANING_BYTES, Why};
    use crate::map::test_support::{TempDir, agent_session, at, project_at, snapshot_of};
    use crate::map::{AgentSession, AgentStatus, Project};

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    /// The questions among `alerts`, each by what a yes to it cleans: `idle` or `at rest`, and the
    /// folders.
    fn questions(alerts: &[Alert]) -> Vec<(&'static str, Vec<&PathBuf>)> {
        alerts
            .iter()
            .map(|alert| match alert {
                Alert::Question {
                    proposal: Proposal::CleanIdle(cleans),
                    ..
                } => ("idle", cleans.iter().map(|clean| &clean.clicked).collect()),
                Alert::Question {
                    proposal: Proposal::CleanAtRest(cleans),
                    ..
                } => (
                    "at rest",
                    cleans.iter().map(|clean| &clean.clicked).collect(),
                ),
                alert => panic!("not a question of the toolbar's: {alert:?}"),
            })
            .collect()
    }

    #[test]
    fn the_toolbars_question_holds_its_folders_until_the_answer_while_cleaning_by_itself_goes_on() {
        let fixture = TempDir::new("asking-idle-holds");
        let (app, asked) = idle_project(&fixture, "app", 10 * WORTH_CLEANING_BYTES);
        let (other, elsewhere) = idle_project(&fixture, "other", 5 * WORTH_CLEANING_BYTES);
        let map = Arc::new(snapshot_of(vec![app.clone(), other]));
        let (mut cleaning, inbox) = reporting();
        cleaning.set_keep_free_floor(ALWAYS_SHORT);

        cleaning.asking_idle(offered(vec![idle_clean(&[&asked])], Vec::new()));
        assert!(
            cleaning.state_of(&app.artifacts[0]).is_some(),
            "its card shows the wait"
        );
        cleaning.keep_free_space(&map, Instant::now(), at(100), fixture.root());
        let Report::UnaskedPrepared { clicked, .. } = next_report(&inbox) else {
            panic!("cleaning by itself works out a clean");
        };
        assert_eq!(
            clicked, elsewhere,
            "the larger goes first unless a question asks about it"
        );

        cleaning.answered_idle(None, None, at(200), fixture.root());
        assert_eq!(
            cleaning.state_of(&app.artifacts[0]),
            None,
            "a no lets it go"
        );
    }

    #[test]
    fn the_toolbars_clean_leaves_out_each_folder_a_card_is_working_out_or_asking_about() {
        let fixture = TempDir::new("asking-idle-leaves-out");
        let package = |name: &str| {
            fixture.package(name);
            idle_project(&fixture, name, 5 * WORTH_CLEANING_BYTES)
        };
        let (asked_project, asked) = package("asked");
        let (worked_out_project, _) = package("worked-out");
        let (idle, still_idle) = package("idle");
        let map = Arc::new(snapshot_of(vec![
            asked_project.clone(),
            worked_out_project.clone(),
            idle,
        ]));
        let (mut cleaning, inbox) = reporting();
        let start = |cleaning: &mut Cleaning, project: &Project| {
            cleaning.start(
                project,
                &project.artifacts[0],
                Arc::clone(&map),
                fixture.root(),
            );
        };
        start(&mut cleaning, &asked_project);
        let prepared = Some(Ok(reaching(&[&asked], "fwd")));
        cleaning.asking(&asked, prepared, None, at(100), fixture.root());
        start(&mut cleaning, &worked_out_project);

        cleaning.start_idle(Arc::clone(&map), at(100), fixture.root());
        let found = loop {
            if let Report::OfferPrepared(found) = next_report(&inbox) {
                break found.expect("the search finished");
            }
        };
        let cleaned: Vec<&PathBuf> = (found.idle.cleans.iter())
            .map(|clean| &clean.clicked)
            .collect();
        assert_eq!(cleaned, [&still_idle]);
    }

    #[test]
    fn the_toolbars_question_leaves_out_what_a_card_asked_about_while_it_was_worked_out() {
        let fixture = TempDir::new("asking-idle-meanwhile");
        let (app, target) = idle_project(&fixture, "app", 5_000);
        let build_dir = fixture.dir("cache/app");
        let map = Arc::new(snapshot_of(vec![app.clone()]));
        let mut cleaning = cleaning();
        cleaning.start(&app, &app.artifacts[0], Arc::clone(&map), fixture.root());
        let prepared = Some(Ok(reaching(&[&target, &build_dir], "fwd")));
        cleaning.asking(&target, prepared, None, at(100), fixture.root());
        cleaning.take_alerts();

        let taking = fixture.dir("other/target");
        let apart = fixture.dir("apart/target");
        let resting = fixture.dir("resting/target");
        let resting_apart = fixture.dir("resting-apart/target");
        cleaning.asking_idle(offered(
            vec![
                idle_clean(&[&target]),
                idle_clean(&[&taking, &build_dir.join("debug")]),
                idle_clean(&[&apart]),
            ],
            vec![
                at_rest_clean(&[&resting, &build_dir]),
                at_rest_clean(&[&resting_apart]),
            ],
        ));

        assert_eq!(
            questions(&cleaning.take_alerts()),
            [("idle", vec![&apart]), ("at rest", vec![&resting_apart])]
        );
    }

    #[test]
    fn the_second_question_comes_after_the_first_and_each_answer_lets_only_its_own_folders_go() {
        let fixture = TempDir::new("asking-at-rest-after");
        let (idle, idle_target) = idle_project(&fixture, "idle", 5 * WORTH_CLEANING_BYTES);
        let (resting, resting_target) = idle_project(&fixture, "resting", 5 * WORTH_CLEANING_BYTES);
        let mut cleaning = cleaning();

        cleaning.asking_idle(offered(
            vec![idle_clean(&[&idle_target])],
            vec![at_rest_clean(&[&resting_target])],
        ));
        let alerts = cleaning.take_alerts();
        assert_eq!(
            questions(&alerts),
            [
                ("idle", vec![&idle_target]),
                ("at rest", vec![&resting_target])
            ],
            "the idle folders first, then apart those kept by sessions at rest"
        );
        let shows_the_wait = |cleaning: &Cleaning, project: &Project| {
            cleaning.state_of(&project.artifacts[0]).is_some()
        };
        assert!(shows_the_wait(&cleaning, &idle) && shows_the_wait(&cleaning, &resting));

        cleaning.answered_idle(None, None, at(200), fixture.root());
        assert!(
            !shows_the_wait(&cleaning, &idle),
            "the first no lets its folder go"
        );
        assert!(
            shows_the_wait(&cleaning, &resting),
            "and the second question holds its own"
        );
        cleaning.answered_at_rest(None, None, at(200), fixture.root());
        assert!(!shows_the_wait(&cleaning, &resting));
        assert!(cleaning.running.is_empty() && cleaning.waiting.is_empty());
    }

    #[test]
    fn with_nothing_idle_the_second_question_still_comes_and_with_neither_nothing_is_to_clean() {
        let fixture = TempDir::new("asking-at-rest-alone");
        let resting = fixture.dir("resting/target");
        let busy = LeftOut {
            project_name: "busy".to_string(),
            folder: PathBuf::from("/code/busy/target"),
            why: Why::Refused(PrepareError::Locks(clean::CleanError::BuildRunning {
                lock: PathBuf::from("/code/busy/target/debug/.cargo-lock"),
            })),
        };
        let mut asking_apart = cleaning();
        let mut asking_nothing = cleaning();

        asking_apart.asking_idle(offered(Vec::new(), vec![at_rest_clean(&[&resting])]));
        assert_eq!(
            questions(&asking_apart.take_alerts()),
            [("at rest", vec![&resting])]
        );

        let at_rest = clean::AtRestCleans {
            cleans: Vec::new(),
            left_out: vec![LeftOut {
                project_name: "tied".to_string(),
                folder: PathBuf::from("/code/tied/target"),
                why: Why::SharedWith("working".to_string()),
            }],
        };
        asking_nothing.asking_idle(Some(Offered {
            idle: IdleCleans {
                cleans: Vec::new(),
                left_out: vec![busy],
            },
            at_rest,
        }));
        let alerts = asking_nothing.take_alerts();
        let [Alert::Notice { title, detail }] = alerts.as_slice() else {
            panic!("one notice, got {alerts:?}");
        };
        assert_eq!(title, "Nothing to clean right now");
        assert_eq!(
            detail,
            "Left out for now: busy (a build is using it) and tied (working builds there and \
             isn't idle)."
        );
    }

    #[test]
    fn a_yes_to_every_idle_folder_starts_them_at_once() {
        let fixture = TempDir::new("asking-idle-yes");
        let target = fixture.dir("app/target");
        let project = fixture.path("app");
        let map = snapshot_of(vec![project_at(project.clone())]);
        let clean = IdleClean {
            clicked: target.clone(),
            project_name: "app".to_string(),
            project_path: project,
            quiet_at_least: OFFER.quiet,
            prepared: prepared_at(&target),
        };
        let mut cleaning = cleaning();
        cleaning.asking_idle(offered(vec![clean.clone()], Vec::new()));
        assert_eq!(
            questions(&cleaning.take_alerts()),
            [("idle", vec![&target])]
        );
        cleaning.start_idle(Arc::new(map.clone()), at(100), fixture.root());
        assert!(
            !cleaning.preparing_idle,
            "a second click waits for the answer"
        );

        cleaning.answered_idle(Some(vec![clean]), Some(&map), at(100), fixture.root());
        assert_eq!(cleaning.running[0].clicked, target);
        assert!(cleaning.waiting.is_empty());
    }

    /// The project `resting` in `fixture`, a package with a large build folder, whose one session
    /// has been at rest since `rested_since`, and that folder.
    fn resting_project(fixture: &TempDir, rested_since: SystemTime) -> (Project, PathBuf) {
        fixture.package("resting");
        let (project, folder) = idle_project(fixture, "resting", 5 * WORTH_CLEANING_BYTES);
        let session = AgentSession {
            since: Some(rested_since),
            ..agent_session("s1", AgentStatus::Idle)
        };
        let project = Project {
            agents: vec![session],
            ..project
        };
        (project, folder)
    }

    #[test]
    fn the_toolbars_clean_asks_about_the_folders_of_sessions_at_rest_for_a_day_and_a_yes_cleans_them()
     {
        let fixture = TempDir::new("asking-at-rest-click");
        let now = at(100 * DAY.as_secs());
        let (resting, target) = resting_project(&fixture, now - 2 * DAY);
        let map = snapshot_of(vec![resting.clone()]);
        let (mut cleaning, inbox) = reporting();

        cleaning.start_idle(Arc::new(map.clone()), now, fixture.root());
        let found = loop {
            if let Report::OfferPrepared(found) = next_report(&inbox) {
                break found;
            }
        };
        cleaning.asking_idle(found);
        let alerts = cleaning.take_alerts();
        assert_eq!(questions(&alerts), [("at rest", vec![&target])]);
        cleaning.start_idle(Arc::new(map.clone()), now, fixture.root());
        assert!(
            !cleaning.preparing_idle,
            "a second click waits for the second answer too"
        );

        let Some(Alert::Question {
            proposal: Proposal::CleanAtRest(cleans),
            ..
        }) = alerts.into_iter().next()
        else {
            unreachable!("asserted above");
        };
        cleaning.answered_at_rest(Some(cleans), Some(&map), now, fixture.root());
        assert_eq!(cleaning.running[0].clicked, target, "a yes starts it");
    }

    #[test]
    fn a_session_at_rest_for_less_than_a_day_keeps_its_folders_from_the_toolbars_clean() {
        let fixture = TempDir::new("asking-at-rest-recent");
        let now = at(100 * DAY.as_secs());
        let (resting, _) = resting_project(&fixture, now - DAY / 2);
        let (mut cleaning, inbox) = reporting();

        cleaning.start_idle(Arc::new(snapshot_of(vec![resting])), now, fixture.root());
        let Report::OfferPrepared(found) = next_report(&inbox) else {
            panic!("the toolbar's search reports");
        };
        cleaning.asking_idle(found);

        let alerts = cleaning.take_alerts();
        let [Alert::Notice { title, .. }] = alerts.as_slice() else {
            panic!("one notice, got {alerts:?}");
        };
        assert_eq!(title, "Nothing to clean right now");
    }
}
