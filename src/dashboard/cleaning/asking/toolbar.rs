//! What happens when the user clicks the toolbar's clean: working out on a thread of its own the
//! clean of every idle folder, then one question about them all.

use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use super::super::queue::By;
use super::super::{Cleaning, OFFER_QUIET_AT_LEAST, Report, words};
use super::Asked;
use crate::dashboard::state::{Alert, Proposal};
use crate::map::Snapshot;
use crate::map::clean::{self, CargoEnv, IdleClean, IdleCleans};

impl Cleaning {
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
    pub(in crate::dashboard::cleaning) fn asking_idle(&mut self, mut found: Option<IdleCleans>) {
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
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

    use super::super::tests::{idle_project, reaching};
    use super::*;
    use crate::dashboard::cleaning::tests::{
        ALWAYS_SHORT, cleaning, idle_clean, next_report, prepared_at, reporting,
    };
    use crate::map::Project;
    use crate::map::clean::WORTH_CLEANING_BYTES;
    use crate::map::test_support::{TempDir, at, project_at, snapshot_of};

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
