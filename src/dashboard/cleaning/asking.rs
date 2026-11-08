//! What happens when the user asks for a clean: working out what it means on a thread of its own,
//! then queueing it, after a question only when the click does more than it says. While a question
//! waits for its answer the dashboard goes on, and the folders it asks about wait with it: their
//! cards show the clean, and neither cleaning by itself nor another click on Clean takes them.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use super::queue::By;
use super::{Cleaning, OFFER_QUIET_AT_LEAST, Report, overlap, words};
use crate::dashboard::state::{Alert, Proposal};
use crate::map::clean::{self, CargoEnv, IdleClean, IdleCleans, PrepareError, Prepared};
use crate::map::{ArtifactDir, Project, Snapshot};

/// The questions waiting for an answer, and every folder each would clean, so that the answer
/// decides those folders. Until it comes, neither cleaning by itself nor a new click on Clean, on a
/// card or the toolbar, takes one of them, a folder inside one or a folder holding one, and each
/// shows on its card as being cleaned. Cleans of other folders go on meanwhile, and so do cleans
/// the user asked for before.
#[derive(Debug, Default)]
pub(super) struct OpenQuestions {
    /// Each folder an open question would clean, by what the user clicked for it.
    folders: HashMap<PathBuf, Asked>,
}

/// What the user clicked for a clean that a question then asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Asked {
    /// Clean on the card of this build folder.
    Clean(PathBuf),
    /// The toolbar's clean of every idle folder.
    CleanIdle,
}

impl OpenQuestions {
    /// `asked`'s question is open, about the clean started from `clicked` that deletes what
    /// `prepared` deletes.
    fn open(&mut self, asked: &Asked, clicked: &Path, prepared: &Prepared) {
        for folder in folders_of(clicked, prepared) {
            self.folders.insert(folder.to_path_buf(), asked.clone());
        }
    }

    /// `asked`'s question was answered, and lets its folders go.
    fn answered(&mut self, asked: &Asked) {
        self.folders.retain(|_, open| open != asked);
    }

    fn is_open(&self, asked: &Asked) -> bool {
        self.folders.values().any(|open| open == asked)
    }

    /// Whether an open question would clean `folder`.
    pub(super) fn asks_about(&self, folder: &Path) -> bool {
        self.folders.contains_key(folder)
    }

    /// The folder an open question would clean that the clean started from `clicked` would take,
    /// deleting what `prepared` deletes: the same folder, one inside it or one holding it. `None`
    /// when the clean keeps clear of them all.
    pub(super) fn asked_about_in(&self, clicked: &Path, prepared: &Prepared) -> Option<&Path> {
        folders_of(clicked, prepared).find_map(|folder| {
            self.folders
                .keys()
                .find(|asked| overlap(asked, folder))
                .map(PathBuf::as_path)
        })
    }

    fn folders(&self) -> impl Iterator<Item = &PathBuf> {
        self.folders.keys()
    }
}

/// The folders the clean started from `clicked` deletes: that one, as the map shows it, and each
/// that `prepared` deletes.
fn folders_of<'a>(clicked: &'a Path, prepared: &'a Prepared) -> impl Iterator<Item = &'a Path> {
    std::iter::once(clicked).chain(prepared.plan().folders())
}

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
    /// The user clicked Clean on `dir`, a build folder of `project`, on its card (see
    /// `start_each`).
    pub(in crate::dashboard) fn start(
        &mut self,
        project: &Project,
        dir: &ArtifactDir,
        map: Arc<Snapshot>,
        home: &Path,
    ) {
        self.start_each(&[(project, dir)], map, home);
    }

    /// The user clicked Clean on each of `clicked`, a build folder and the project it is in: on a
    /// card, its one, or on a line, every build folder its menu offers in what it holds. Each
    /// card shows its clean at once, and what the cleans mean is worked out on one thread, as `map`
    /// shows them: `asking` then takes each from there, as it would the one of a card.
    pub(in crate::dashboard) fn start_each(
        &mut self,
        clicked: &[(&Project, &ArtifactDir)],
        map: Arc<Snapshot>,
        home: &Path,
    ) {
        // A folder whose clean is worked out or asked about already, from its card or the
        // toolbar, waits for that: another click would only ask again.
        let clicked: Vec<Clicked> = clicked
            .iter()
            .filter(|(_, dir)| {
                !self.preparing.contains_key(&dir.path) && !self.questions.asks_about(&dir.path)
            })
            .map(|(project, dir)| Clicked {
                project: (*project).clone(),
                dir: (*dir).clone(),
                sandboxed: map.sandboxed,
            })
            .collect();
        if clicked.is_empty() {
            return;
        }
        let folders: Vec<(PathBuf, PathBuf)> = clicked
            .iter()
            .map(|about| (about.dir.path.clone(), about.project.path.clone()))
            .collect();
        let reported = folders.iter().map(|(folder, _)| folder.clone()).collect();
        let env = CargoEnv::of_this_process(home);
        let preparing = self.threads.run(
            "sutra-clean-prepare",
            move || clean::prepare_each(&folders, &map, &env),
            move |prepared| Report::Prepared {
                clicked: reported,
                prepared,
            },
        );
        for about in clicked {
            match &preparing {
                Ok(()) => {
                    self.preparing.insert(about.dir.path.clone(), about);
                }
                Err(err) => {
                    let words = about.words(home);
                    self.alerts
                        .push(Alert::notice(words.cannot_clean(), err.to_string()));
                }
            }
        }
    }

    /// What the click on `clicked` means has been worked out, or its thread ended with no word
    /// (`None`). Say why not, or queue the clean. One that would take a folder another question
    /// asks about goes no further, as that answer decides the folder. Choosing Clean on a folder's
    /// card says what it wants, unless other projects' builds would go too, which only a question
    /// can name. The clean's thread takes the locks first, so a build started meanwhile, or a
    /// folder changed, stops it there.
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
        if let Some(asked) = self.questions.asked_about_in(clicked, &prepared) {
            let refused = Alert::notice(words.cannot_clean(), words.asked_already(asked));
            self.alerts.push(refused);
            return;
        }
        if prepared.sharing.reaches_other_projects() {
            let question = words.question(prepared.plan(), &prepared.sizes, &prepared.sharing);
            let asked = Asked::Clean(clicked.to_path_buf());
            self.questions.open(&asked, clicked, &prepared);
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
    /// yes. Either way the question lets its folders go, and on a no their cards stop showing the
    /// wait.
    pub(in crate::dashboard) fn answered_clean(
        &mut self,
        clicked: PathBuf,
        prepared: Option<Prepared>,
        map: Option<&Snapshot>,
        now: SystemTime,
        home: &Path,
    ) {
        self.questions.answered(&Asked::Clean(clicked.clone()));
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
        if self.preparing_idle || self.questions.is_open(&Asked::CleanIdle) {
            return;
        }
        // The toolbar's offer leaves out each folder whose card shows a clean of its own, and so
        // does the question that follows it.
        let being_cleaned = self.being_cleaned();
        let env = CargoEnv::of_this_process(home);
        let preparing = self.threads.run(
            "sutra-clean-prepare",
            move || {
                clean::prepare_idle(
                    &map,
                    now,
                    |_| Some(OFFER_QUIET_AT_LEAST),
                    &being_cleaned,
                    &env,
                )
            },
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
    pub(super) fn asking_idle(&mut self, mut found: Option<IdleCleans>) {
        self.preparing_idle = false;
        // A card's question opened while these were worked out keeps the folders it asks about.
        if let Some(found) = &mut found {
            found.cleans.retain(|clean| {
                self.questions
                    .asked_about_in(&clean.clicked, &clean.prepared)
                    .is_none()
            });
        }
        if let Some((title, detail)) = words::nothing_to_ask(found.as_ref()) {
            self.alerts.push(Alert::notice(title, detail));
            return;
        }
        let Some(IdleCleans { cleans, left_out }) = found else {
            return;
        };
        let question = words::idle_question(&cleans, &left_out, OFFER_QUIET_AT_LEAST);
        for clean in &cleans {
            self.questions
                .open(&Asked::CleanIdle, &clean.clicked, &clean.prepared);
        }
        self.alerts
            .push(Alert::question(question, Proposal::CleanIdle(cleans)));
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

    /// Every folder whose card shows a clean of its own: being cleaned or not yet measured since,
    /// worked out for a click, or asked about. A clean of the idle folders that Sutra or the user
    /// starts leaves them out, as their own clean decides them.
    pub(super) fn being_cleaned(&self) -> HashSet<PathBuf> {
        (self.folders.keys())
            .chain(self.preparing.keys())
            .chain(self.questions.folders())
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::dashboard::cleaning::tests::{
        ALWAYS_SHORT, cleaning, idle_clean, next_report, prepared_at, prepared_of, reporting,
    };
    use crate::dashboard::cleaning::{CleanState, IdleOffer};
    use crate::map::clean::{Sharing, WORTH_CLEANING_BYTES};
    use crate::map::test_support::{
        CACHEDIR_TAG, TempDir, at, cargo_target, measured, project_at, snapshot_of,
    };

    /// A clean of `folders` that takes builds of the project `other` with it.
    fn reaching(folders: &[&Path], other: &str) -> Prepared {
        let mut prepared = prepared_of(folders);
        prepared.sharing = Sharing {
            projects: vec![other.to_string()],
            ..Sharing::default()
        };
        prepared
    }

    /// A project at `name` in `fixture` holding a measured build folder, idle since it has no
    /// activity, and that folder.
    fn idle_project(fixture: &TempDir, name: &str, bytes: u64) -> (Project, PathBuf) {
        let folder = fixture.dir(&format!("{name}/target"));
        let project = Project {
            artifacts: vec![cargo_target(folder.clone(), measured(bytes))],
            ..project_at(fixture.path(name))
        };
        (project, folder)
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
    fn a_click_on_several_shows_on_each_card_at_once_and_each_goes_as_its_own_click_would() {
        let fixture = TempDir::new("asking-each");
        // Scratch folders with a build folder `CARGO_TARGET_DIR` named, which cargo tagged, or
        // a copy of one, which it did not, and Sutra will not delete.
        let scratch = |name: &str, tagged: bool| {
            let tag = if tagged { CACHEDIR_TAG } else { "" };
            fixture.write(&format!("{name}/target/CACHEDIR.TAG"), tag);
            let dir = cargo_target(fixture.path(&format!("{name}/target")), measured(5_000));
            let folder = Project {
                artifacts: vec![dir.clone()],
                ..project_at(fixture.path(name))
            };
            (folder, dir)
        };
        let (made, made_target) = scratch("r567", true);
        let (copied, copied_target) = scratch("copied", false);
        let map = Arc::new(snapshot_of(vec![made.clone(), copied.clone()]));
        let (mut cleaning, inbox) = reporting();

        let clicked = [(&made, &made_target), (&copied, &copied_target)];
        cleaning.start_each(&clicked, map, fixture.root());
        for dir in [&made_target, &copied_target] {
            assert!(cleaning.state_of(dir).is_some(), "its card shows the clean");
        }
        let report = next_report(&inbox);
        cleaning.receive(report, None, at(100), fixture.root());

        assert_eq!(
            cleaning.running[0].clicked, made_target.path,
            "the one cargo made goes"
        );
        let alerts = cleaning.take_alerts();
        let [Alert::Notice { title, .. }] = alerts.as_slice() else {
            panic!("one notice, got {alerts:?}");
        };
        assert_eq!(title, "Can't clean target in copied");
        assert_eq!(
            cleaning.state_of(&copied_target),
            None,
            "it shows its size again"
        );
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
                Some(Ok(reaching(&[&folder], "fwd"))),
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
            Some(reaching(&[&folder], "fwd")),
            None,
            at(200),
            fixture.root(),
        );
        assert_eq!(cleaning.running[0].clicked, folder, "a yes starts it");
    }

    #[test]
    fn a_folder_asked_about_leaves_the_offer_and_cleaning_by_itself_until_the_answer() {
        let fixture = TempDir::new("asking-off-the-offer");
        let folder = fixture.dir("app/target");
        let dir = cargo_target(folder.clone(), measured(5 * WORTH_CLEANING_BYTES));
        // An idle project, so the toolbar offers its folder and cleaning by itself would pick it.
        let app = Project {
            artifacts: vec![dir.clone()],
            ..project_at(fixture.path("app"))
        };
        let map = Arc::new(snapshot_of(vec![app.clone()]));
        let mut cleaning = cleaning();
        cleaning.set_keep_free_floor(ALWAYS_SHORT);
        let offered = Some(IdleOffer {
            folders: 1,
            bytes: 5 * WORTH_CLEANING_BYTES,
        });
        assert_eq!(cleaning.idle_offer(&map, at(100)), offered);

        cleaning.start(&app, &dir, Arc::clone(&map), fixture.root());
        let prepared = Some(Ok(reaching(&[&folder], "fwd")));
        cleaning.asking(&folder, prepared, None, at(100), fixture.root());
        assert_eq!(cleaning.idle_offer(&map, at(100)), None);
        let looked = Instant::now();
        cleaning.keep_free_space(&map, looked, at(100), fixture.root());
        assert!(
            !cleaning.preparing_unasked,
            "cleaning by itself passes it by"
        );

        cleaning.answered_clean(folder.clone(), None, None, at(200), fixture.root());
        assert_eq!(
            cleaning.idle_offer(&map, at(200)),
            offered,
            "a no offers it again"
        );
        let next_look = looked + Duration::from_secs(60);
        cleaning.keep_free_space(&map, next_look, at(200), fixture.root());
        assert!(
            cleaning.preparing_unasked,
            "and cleaning by itself may pick it"
        );
    }

    #[test]
    fn a_question_holds_every_folder_its_clean_deletes_until_the_answer() {
        let fixture = TempDir::new("asking-holds-the-plan");
        let (app, target) = idle_project(&fixture, "app", 5 * WORTH_CLEANING_BYTES);
        // The build-dir a config sets, which lies in another project and shows on its card.
        let (cache, build_dir) = idle_project(&fixture, "cache", 5 * WORTH_CLEANING_BYTES);
        let map = Arc::new(snapshot_of(vec![app.clone(), cache.clone()]));
        let mut cleaning = cleaning();
        cleaning.set_keep_free_floor(ALWAYS_SHORT);

        cleaning.start(&app, &app.artifacts[0], Arc::clone(&map), fixture.root());
        let prepared = Some(Ok(reaching(&[&target, &build_dir], "fwd")));
        cleaning.asking(&target, prepared, None, at(100), fixture.root());
        assert!(
            cleaning.state_of(&cache.artifacts[0]).is_some(),
            "the build-dir's card shows the wait too"
        );
        let looked = Instant::now();
        cleaning.keep_free_space(&map, looked, at(100), fixture.root());
        assert!(
            !cleaning.preparing_unasked,
            "cleaning by itself leaves the build-dir to the answer"
        );

        cleaning.answered_clean(target.clone(), None, None, at(200), fixture.root());
        assert_eq!(
            cleaning.state_of(&cache.artifacts[0]),
            None,
            "a no lets it go"
        );
        let next_look = looked + Duration::from_secs(60);
        cleaning.keep_free_space(&map, next_look, at(200), fixture.root());
        assert!(
            cleaning.preparing_unasked,
            "and cleaning by itself may pick it"
        );
    }

    #[test]
    fn the_toolbars_question_holds_its_folders_until_the_answer_while_cleaning_by_itself_goes_on() {
        let fixture = TempDir::new("asking-idle-holds");
        let (app, asked) = idle_project(&fixture, "app", 10 * WORTH_CLEANING_BYTES);
        let (other, elsewhere) = idle_project(&fixture, "other", 5 * WORTH_CLEANING_BYTES);
        let map = Arc::new(snapshot_of(vec![app.clone(), other]));
        let (mut cleaning, inbox) = reporting();
        cleaning.set_keep_free_floor(ALWAYS_SHORT);

        cleaning.asking_idle(Some(IdleCleans {
            cleans: vec![idle_clean(&[&asked])],
            left_out: Vec::new(),
        }));
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
    fn a_click_whose_clean_takes_a_folder_a_question_asks_about_is_refused_and_one_apart_goes() {
        let fixture = TempDir::new("asking-taken");
        let asked = fixture.dir("cache/app");
        let map = Arc::new(snapshot_of(Vec::new()));
        let mut cleaning = cleaning();
        cleaning.asking_idle(Some(IdleCleans {
            cleans: vec![idle_clean(&[&asked])],
            left_out: Vec::new(),
        }));
        cleaning.take_alerts();
        // A click on Clean in the project `name`, whose clean takes `build_dir` with its target.
        let click = |cleaning: &mut Cleaning, name: &str, build_dir: &Path| {
            let (project, target) = idle_project(&fixture, name, 5_000);
            cleaning.start(
                &project,
                &project.artifacts[0],
                Arc::clone(&map),
                fixture.root(),
            );
            let prepared = Some(Ok(prepared_of(&[&target, build_dir])));
            cleaning.asking(&target, prepared, None, at(100), fixture.root());
            cleaning.take_alerts()
        };

        for (name, build_dir) in [
            ("inside", asked.join("debug")),
            ("around", fixture.path("cache")),
        ] {
            let alerts = click(&mut cleaning, name, &build_dir);
            let [Alert::Notice { title, detail }] = alerts.as_slice() else {
                panic!("one notice, got {alerts:?}");
            };
            assert_eq!(*title, format!("Can't clean target in {name}"));
            assert!(detail.contains("~/cache/app"), "{detail}");
        }
        assert!(cleaning.running.is_empty() && cleaning.waiting.is_empty());

        let apart = fixture.dir("elsewhere/build");
        assert!(click(&mut cleaning, "apart", &apart).is_empty());
        assert_eq!(cleaning.running.len(), 1, "a clean of other folders goes");
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
            if let Report::IdlePrepared(found) = next_report(&inbox) {
                break found.expect("the search finished");
            }
        };
        let cleaned: Vec<&PathBuf> = found.cleans.iter().map(|clean| &clean.clicked).collect();
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
        cleaning.asking_idle(Some(IdleCleans {
            cleans: vec![
                idle_clean(&[&target]),
                idle_clean(&[&taking, &build_dir.join("debug")]),
                idle_clean(&[&apart]),
            ],
            left_out: Vec::new(),
        }));

        let alerts = cleaning.take_alerts();
        let [
            Alert::Question {
                proposal: Proposal::CleanIdle(cleans),
                ..
            },
        ] = alerts.as_slice()
        else {
            panic!("one question, got {alerts:?}");
        };
        let asked: Vec<&PathBuf> = cleans.iter().map(|clean| &clean.clicked).collect();
        assert_eq!(asked, [&apart]);
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
            quiet_at_least: OFFER_QUIET_AT_LEAST,
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
