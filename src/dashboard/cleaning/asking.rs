//! What happens when the user asks for a clean: working out what it means on a thread of its own,
//! then queueing it, after a question only when the click does more than it says. While a question
//! waits for its answer the dashboard goes on, and the card shows the clean meanwhile.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use super::queue::By;
use super::{Cleaning, OFFER_QUIET_AT_LEAST, Report, words};
use crate::dashboard::state::{Alert, Proposal};
use crate::map::clean::{self, CargoEnv, IdleClean, IdleCleans, PrepareError, Prepared};
use crate::map::{ArtifactDir, Project, Snapshot};

/// The card a click on Clean came from, which whatever is said about the clean speaks of even if
/// the map has moved on while it was worked out.
#[derive(Debug)]
pub(super) struct Clicked {
    project: Project,
    dir: ArtifactDir,
    /// The map was of the store edition, where an unreadable file is one outside the grant.
    sandboxed: bool,
}

impl Clicked {
    fn words<'a>(&'a self, home: &'a Path) -> words::Context<'a> {
        words::Context::new(&self.project, &self.dir, home, self.sandboxed)
    }
}

impl Cleaning {
    /// The user clicked Clean on `dir`, a build folder of `project`. Its card shows the clean at
    /// once, and what it means is worked out on a thread of its own, as `map` shows them: `asking`
    /// then takes it from there.
    pub(in crate::dashboard) fn start(
        &mut self,
        project: &Project,
        dir: &ArtifactDir,
        map: Arc<Snapshot>,
        home: &Path,
    ) {
        let clicked = dir.path.clone();
        // A second click while the first is worked out or asked about would only do the same again.
        if self.preparing.contains_key(&clicked) || self.asking.contains(&clicked) {
            return;
        }
        let about = Clicked {
            project: project.clone(),
            dir: dir.clone(),
            sandboxed: map.sandboxed,
        };
        let preparing = {
            let (folder, project) = (clicked.clone(), project.path.clone());
            let reported = clicked.clone();
            let env = CargoEnv::of_this_process(home);
            self.threads.run(
                "sutra-clean-prepare",
                move || clean::prepare(&folder, &project, &map, &env),
                move |prepared| Report::Prepared {
                    clicked: reported,
                    prepared,
                },
            )
        };
        if let Err(err) = preparing {
            let words = about.words(home);
            self.alerts
                .push(Alert::notice(words.cannot_clean(), err.to_string()));
            return;
        }
        self.preparing.insert(clicked, about);
    }

    /// What the click on `clicked` means has been worked out, or its thread ended with no word
    /// (`None`). Say why not, or queue the clean. Choosing Clean on a folder's card says what it
    /// wants, unless other projects' builds would go too, which only a question can name. The
    /// clean's thread takes the locks first, so a build started meanwhile, or a folder changed,
    /// stops it there.
    pub(super) fn asking(
        &mut self,
        clicked: &Path,
        prepared: Option<Result<Prepared, PrepareError>>,
        map: Option<&Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        let Some(about) = self.preparing.remove(clicked) else {
            return;
        };
        let words = about.words(home);
        let prepared = match prepared {
            Some(Ok(prepared)) => prepared,
            Some(Err(err)) => {
                let refused = Alert::notice(words.cannot_clean(), words.refused(&err));
                self.alerts.push(refused);
                return;
            }
            None => {
                let (title, detail) = words::stopped(clicked, None, 0, home);
                self.alerts.push(Alert::notice(title, detail));
                return;
            }
        };
        if prepared.sharing.reaches_other_projects() {
            let question = words.question(&prepared.plan, &prepared.sizes, &prepared.sharing);
            self.asking.insert(clicked.to_path_buf());
            let proposal = Proposal::Clean {
                clicked: clicked.to_path_buf(),
                prepared: Box::new(prepared),
            };
            self.alerts.push(Alert::question(question, proposal));
            return;
        }
        self.queue(clicked.to_path_buf(), prepared, By::User);
        self.start_next(map, now, home);
    }

    /// The user answered the question about the clean of `clicked`, with `prepared` to go on a
    /// yes. Either way its card stops showing the wait.
    pub(in crate::dashboard) fn answered_clean(
        &mut self,
        clicked: PathBuf,
        prepared: Option<Prepared>,
        map: Option<&Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        self.asking.remove(&clicked);
        if let Some(prepared) = prepared {
            self.queue(clicked, prepared, By::User);
            self.start_next(map, now, home);
        }
    }

    /// The user clicked the toolbar's clean. Work out the clean of every idle folder on a thread of
    /// its own, as `map` shows them: `asking_idle` takes it from there.
    pub(in crate::dashboard) fn start_idle(
        &mut self,
        map: Arc<Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        // A second click while the first is worked out or asked about would only ask again.
        if self.preparing_idle || self.asking_idle {
            return;
        }
        let being_cleaned: HashSet<PathBuf> = self.folders.keys().cloned().collect();
        let env = CargoEnv::of_this_process(home);
        let preparing = self.threads.run(
            "sutra-clean-prepare",
            move || clean::prepare_idle(&map, now, OFFER_QUIET_AT_LEAST, &being_cleaned, &env),
            Report::IdlePrepared,
        );
        if let Err(err) = preparing {
            let stopped = Alert::notice(words::IDLE_SEARCH_STOPPED, err.to_string());
            self.alerts.push(stopped);
            return;
        }
        self.preparing_idle = true;
    }

    /// The cleans of the idle folders have been worked out, or their thread ended with no word
    /// (`None`). Ask once, naming how many and how much; `answered_idle` takes the answer.
    pub(super) fn asking_idle(&mut self, found: Option<IdleCleans>) {
        self.preparing_idle = false;
        if let Some((title, detail)) = words::nothing_to_ask(found.as_ref()) {
            self.alerts.push(Alert::notice(title, detail));
            return;
        }
        let Some(IdleCleans { cleans, left_out }) = found else {
            return;
        };
        let question = words::idle_question(&cleans, &left_out, OFFER_QUIET_AT_LEAST);
        self.asking_idle = true;
        self.alerts
            .push(Alert::question(question, Proposal::CleanIdle(cleans)));
    }

    /// The user answered the question about every idle folder, with `cleans` to go on a yes. Each
    /// looks at whether its project is still idle when its turn comes, as `map` shows it then.
    pub(in crate::dashboard) fn answered_idle(
        &mut self,
        cleans: Option<Vec<IdleClean>>,
        map: Option<&Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        self.asking_idle = false;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::cleaning::CleanState;
    use crate::dashboard::cleaning::tests::{cleaning, prepared_at};
    use crate::map::clean::{Sharing, WORTH_CLEANING_BYTES};
    use crate::map::test_support::{TempDir, at, cargo_target, measured, project_at, snapshot_of};

    /// A clean of `folder` that takes builds of the project `other` with it.
    fn reaching(folder: &Path, other: &str) -> Prepared {
        let mut prepared = prepared_at(folder);
        prepared.sharing = Sharing {
            projects: vec![other.to_string()],
            ..Sharing::default()
        };
        prepared
    }

    #[test]
    fn a_click_on_clean_shows_at_once_and_goes_with_no_question() {
        let fixture = TempDir::new("asking-no-question");
        let folder = fixture.dir("app/target");
        let project = fixture.path("app");
        let dir = cargo_target(folder.clone(), measured(5_000));
        let map = Arc::new(snapshot_of(vec![project_at(project.clone())]));
        let mut cleaning = cleaning();

        cleaning.start(&project_at(project), &dir, map, fixture.root());
        assert_eq!(
            cleaning.state_of(&dir).and_then(|state| state.left()),
            Some(5_000),
            "the card shows it while it is worked out"
        );

        cleaning.asking(
            &folder,
            Some(Ok(prepared_at(&folder))),
            None,
            at(100),
            fixture.root(),
        );
        assert_eq!(cleaning.running[0].clicked, folder);
        assert!(cleaning.take_alerts().is_empty());
    }

    #[test]
    fn a_clean_reaching_other_projects_asks_once_and_its_card_shows_the_wait_until_the_answer() {
        let fixture = TempDir::new("asking-reaches-others");
        let folder = fixture.dir("app/target");
        let dir = cargo_target(folder.clone(), measured(5 * WORTH_CLEANING_BYTES));
        let map = Arc::new(snapshot_of(vec![project_at(fixture.path("app"))]));
        let asked = || {
            let mut cleaning = cleaning();
            cleaning.start(
                &project_at(fixture.path("app")),
                &dir,
                Arc::clone(&map),
                fixture.root(),
            );
            cleaning.asking(
                &folder,
                Some(Ok(reaching(&folder, "fwd"))),
                None,
                at(100),
                fixture.root(),
            );
            cleaning
        };

        let mut cleaning = asked();
        let alerts = cleaning.take_alerts();
        let [Alert::Question { question, proposal }] = alerts.as_slice() else {
            panic!("one question, got {alerts:?}");
        };
        assert!(question.detail.contains("Builds of fwd are in there too"));
        assert!(matches!(proposal, Proposal::Clean { clicked, .. } if *clicked == folder));
        assert!(
            cleaning.running.is_empty(),
            "nothing goes before the answer"
        );
        assert_eq!(
            cleaning.state_of(&dir),
            Some(CleanState::Deleting {
                before: Some(5 * WORTH_CLEANING_BYTES),
                freed: 0
            }),
            "the card shows the clean while the question is open"
        );
        let map_now = snapshot_of(vec![project_at(fixture.path("app"))]);
        assert!(cleaning.idle_offer(&map_now, at(100)).is_none());
        cleaning.start(
            &project_at(fixture.path("app")),
            &dir,
            Arc::clone(&map),
            fixture.root(),
        );
        assert!(
            cleaning.preparing.is_empty(),
            "a second click waits for the answer"
        );

        cleaning.answered_clean(folder.clone(), None, None, at(200), fixture.root());
        assert_eq!(cleaning.state_of(&dir), None, "a no shows the size again");
        assert!(cleaning.running.is_empty());

        let mut cleaning = asked();
        cleaning.answered_clean(
            folder.clone(),
            Some(reaching(&folder, "fwd")),
            None,
            at(200),
            fixture.root(),
        );
        assert_eq!(cleaning.running[0].clicked, folder, "a yes starts it");
    }

    #[test]
    fn a_refused_clean_or_one_whose_thread_ended_with_no_word_is_told_in_a_notice() {
        let fixture = TempDir::new("asking-refused");
        let folder = fixture.dir("app/target");
        let dir = cargo_target(folder.clone(), measured(5_000));
        let map = Arc::new(snapshot_of(vec![project_at(fixture.path("app"))]));
        let told = |prepared| {
            let mut cleaning = cleaning();
            cleaning.start(
                &project_at(fixture.path("app")),
                &dir,
                Arc::clone(&map),
                fixture.root(),
            );
            cleaning.asking(&folder, prepared, None, at(100), fixture.root());
            assert_eq!(
                cleaning.state_of(&dir),
                None,
                "its card shows its size again"
            );
            match cleaning.take_alerts().as_slice() {
                [Alert::Notice { title, .. }] => title.clone(),
                alerts => panic!("one notice, got {alerts:?}"),
            }
        };
        let busy = PrepareError::Locks(crate::map::clean::CleanError::BuildRunning {
            lock: folder.join("debug/.cargo-lock"),
        });
        assert_eq!(told(Some(Err(busy))), "Can't clean target in app");
        assert!(told(None).starts_with("Couldn't finish cleaning"));
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
            prepared: prepared_at(&target),
        };
        let mut cleaning = cleaning();
        cleaning.asking_idle(Some(IdleCleans {
            cleans: vec![clean.clone()],
            left_out: Vec::new(),
        }));
        assert!(matches!(
            cleaning.take_alerts().as_slice(),
            [Alert::Question {
                proposal: Proposal::CleanIdle(_),
                ..
            }]
        ));
        cleaning.start_idle(Arc::new(map.clone()), at(100), fixture.root());
        assert!(
            !cleaning.preparing_idle,
            "a second click waits for the answer"
        );

        cleaning.answered_idle(Some(vec![clean]), Some(&map), at(100), fixture.root());
        assert_eq!(cleaning.running[0].clicked, target);
        assert!(cleaning.waiting.is_empty());
    }
}
