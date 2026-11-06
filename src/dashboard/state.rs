//! The dashboard's state, which its thread owns: the newest map and the engine that keeps it, the
//! user's arrangement of the list, the cleans, the announcements, the alerts and the settings. Each
//! event the thread handles has a method here, and `contents` works out what the window shows at
//! an instant.

mod dialogs;
mod settings;
#[cfg(test)]
mod tests;
mod watching;

pub(super) use dialogs::{Alert, Proposal, Question};
pub(super) use watching::EngineId;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Instant, SystemTime};

use nix::errno::Errno;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;

use super::cleaning::{self, Cleaning, IdleOffer, Report, Threads};
use super::contents::{CardContext, banner, card, footer, heading, list, toolbar};
use super::listing::{Arrangement, Listing};
use super::live::Event;
use super::{Action, PrivateFolderSwitch, Settings, WindowContents, format};
use crate::map::grant::{self, GrantError, Roots};
use crate::map::{PrivateFolder, Snapshot, clean};
use crate::notifications::Notifier;
use dialogs::Dialogs;
use settings::SavedSettings;
use watching::Watching;

/// Everything the dashboard thread owns.
pub(super) struct State {
    /// The real home folder: what `~` stands for on screen, and where a clean finds cargo's home.
    home: PathBuf,
    /// The store edition, which reads only the folder the user granted.
    sandboxed: bool,
    /// The folders the engine watches. Empty while the store edition has no folder granted.
    roots: Vec<PathBuf>,
    /// Why the folder granted on an earlier launch could not be reopened, until another is.
    lost_grant: Option<GrantError>,
    watching: Option<Watching>,
    /// Where each engine started sends its snapshots.
    events: Sender<Event>,
    /// The newest map; `None` until the engine's first scan finishes.
    snapshot: Option<Arc<Snapshot>>,
    /// The engine's channel closed, so the map on screen no longer updates.
    engine_stopped: bool,
    /// What the user set that outlasts a run.
    settings: SavedSettings,
    /// Where `settings` are kept between runs, when the system has a place for them.
    settings_file: Option<PathBuf>,
    arrangement: Arrangement,
    cleaning: Cleaning,
    notifier: Notifier,
    dialogs: Dialogs,
}

impl State {
    /// The dashboard as the app starts, on the folders this edition may read and with what the
    /// user set in an earlier run, its engine at work. Banners go to `post_banner`, and every
    /// thread the dashboard starts reports into `events`.
    pub(super) fn launch(
        post_banner: impl Fn(&str, &str) + Send + 'static,
        events: Sender<Event>,
    ) -> State {
        // Never shows a panel: with no folder granted yet, the window asks.
        let (roots, lost_grant) = grant::resolve_roots();
        let settings_file = SavedSettings::file();
        let (settings, unreadable) = match SavedSettings::load(settings_file.as_deref()) {
            Ok(settings) => (settings, None),
            Err(err) => (SavedSettings::default(), Some(err)),
        };
        let notifier = Notifier::new(post_banner);
        let mut state = State::new(roots, lost_grant, settings, settings_file, notifier, events);
        if let Some(err) = unreadable {
            state.dialogs.push(Alert::notice(
                "Sutra couldn't read its settings",
                format!("It won't clean by itself until you choose that again: {err}."),
            ));
        }
        state.start_engine();
        state
    }

    /// The dashboard before any engine starts.
    fn new(
        roots: Roots,
        lost_grant: Option<GrantError>,
        settings: SavedSettings,
        settings_file: Option<PathBuf>,
        notifier: Notifier,
        events: Sender<Event>,
    ) -> State {
        State {
            home: roots.home,
            sandboxed: roots.sandboxed,
            roots: roots.dirs,
            lost_grant,
            watching: None,
            cleaning: Cleaning::new(Threads::new(events.clone()), settings.keep_free_bytes),
            events,
            snapshot: None,
            engine_stopped: false,
            settings,
            settings_file,
            arrangement: Arrangement::default(),
            notifier,
            dialogs: Dialogs::default(),
        }
    }

    /// Do what the user asked, at `now`.
    pub(super) fn perform(&mut self, action: Action, now: SystemTime) {
        match action {
            Action::Clean { folder } => self.clean(&folder),
            Action::CleanIdle => self.clean_idle(now),
            Action::KeepFree { bytes } => self.keep_free(bytes),
            Action::SetSort { order } => {
                if let Some(map) = &self.snapshot {
                    self.arrangement.set_sort(order, map, now);
                }
            }
            Action::ShowFilter => self.arrangement.show_filter(),
            Action::FilterChanged { text } => {
                let map = self.snapshot.as_deref();
                self.arrangement.set_filter_text(text, map, now);
            }
            Action::HideFilter => self.arrangement.hide_filter(self.snapshot.as_deref(), now),
            Action::ToggleQuietProjects => {
                self.arrangement.toggle_quiet(self.snapshot.as_deref(), now);
            }
            Action::ToggleRepositoryFold { common_dir } => self.toggle_fold(&common_dir),
            Action::PointerEnteredList => {
                let map = self.snapshot.as_deref();
                self.arrangement.pointer_entered(map, now);
            }
            Action::PointerLeftList => self.arrangement.pointer_left(),
            Action::ToggleGlobalMute => self.notifier.toggle_global_mute(),
            Action::ToggleGlobalNotifications => self.notifier.toggle_global_notifications(),
            Action::ToggleUnitMute { environment, unit } => {
                self.notifier.toggle_unit_mute(&environment, &unit);
            }
            Action::ToggleUnitNotifications { environment, unit } => {
                self.notifier.toggle_unit_notifications(&environment, &unit);
            }
            Action::TerminateEnvironment { pid } => self.terminate(pid),
            Action::Answer { dialog, yes } => self.answer(dialog, yes, now),
            Action::FolderGranted { folder } => self.folder_granted(folder),
            Action::WatchPrivateFolder { folder, watch } => {
                self.watch_private_folder(&folder, watch);
            }
        }
        self.pass_on_clean_news();
    }

    /// A map from engine `engine` arrived at `now`. One from an engine since replaced is dropped.
    pub(super) fn on_snapshot(
        &mut self,
        engine: EngineId,
        snapshot: Arc<Snapshot>,
        now: SystemTime,
    ) {
        if self.sent_by_the_running_engine(engine) {
            self.show(snapshot, now);
        }
    }

    /// The channel of engine `engine` closed, so its map no longer changes. No later map can end
    /// the waits still to be announced, so none is announced after it may have ended.
    pub(super) fn on_engine_stopped(&mut self, engine: EngineId) {
        if self.sent_by_the_running_engine(engine) {
            self.engine_stopped = true;
            self.notifier.forget_pending_waits();
        }
    }

    /// A cleaning thread reported what it found, at `now`.
    pub(super) fn on_report(&mut self, report: Report, now: SystemTime) {
        let map = self.live_map().cloned();
        self.cleaning
            .receive(report, map.as_deref(), now, &self.home);
        self.pass_on_clean_news();
    }

    /// The clock ticked: while the map is live, see whether to clean by itself.
    pub(super) fn tick(&mut self, at: Instant, now: SystemTime) {
        if let Some(map) = self.live_map().cloned() {
            self.cleaning.keep_free_space(&map, at, now, &self.home);
            self.pass_on_clean_news();
        }
    }

    /// Announce the agent waits that have lasted long enough by now.
    pub(super) fn announce_lasting_waits(&mut self) {
        self.notifier.announce_lasting_waits();
    }

    /// Whether an agent's wait is still to be announced, should it last.
    pub(super) fn has_pending_waits(&self) -> bool {
        self.notifier.has_pending_waits()
    }

    /// Whether a clean is deleting now, which its card's bar shows moving.
    pub(super) fn is_deleting(&self) -> bool {
        self.cleaning.is_deleting()
    }

    /// Stop the engine, as the app quits.
    pub(super) fn stop(&mut self) {
        if let Some(watching) = self.watching.take() {
            watching.stop();
        }
    }

    /// What the window shows at `now`.
    pub(super) fn contents(&self, now: SystemTime) -> WindowContents {
        let listing = self.listing(now);
        let waiting = listing
            .as_ref()
            .map_or(0, |listing| listing.snapshot.waiting_agents());
        let map = listing.as_ref().map(|listing| listing.snapshot);
        let cards = CardContext {
            notifier: &self.notifier,
            cleaning: &self.cleaning,
            home: &self.home,
            now,
        };
        let clean = toolbar::clean_control(
            self.idle_offer(now),
            self.can_tell_what_is_idle(),
            self.cleaning.is_preparing_idle(),
            self.cleaning.keep_free_floor(),
        );
        WindowContents {
            title: heading::title(waiting, self.engine_stopped),
            dock_badge: heading::dock_badge(waiting, self.engine_stopped),
            toolbar: toolbar::toolbar(
                listing.as_ref(),
                &self.arrangement,
                clean,
                &self.notifier,
                self.sandboxed,
                &self.home,
            ),
            banner: map.and_then(|map| banner::banner(&map.problems, self.sandboxed, &self.home)),
            // With no map shown, the list says the engine stopped before its first.
            stopped: (self.engine_stopped && map.is_some()).then(|| heading::STOPPED.to_string()),
            list: list::list(
                listing.as_ref(),
                self.arrangement.filter(),
                &self.roots,
                self.lost_grant.as_ref(),
                self.engine_stopped,
                &self.settings.folded_repositories,
                &cards,
            ),
            footer: map.and_then(|map| footer::footer(map, &self.home)),
            dialog: self.dialogs.shown(),
            settings: Settings {
                keep_free: self.cleaning.keep_free_floor(),
                keep_free_choices: cleaning::keep_free_choices(),
                private_folders: PrivateFolder::ALL
                    .into_iter()
                    .map(|folder| PrivateFolderSwitch {
                        folder: folder.name().to_string(),
                        watched: self.settings.watched_private_folders.contains(&folder),
                    })
                    .collect(),
            },
        }
    }

    /// The list as it shows at `now`, once a map has arrived.
    fn listing(&self, now: SystemTime) -> Option<Listing<'_>> {
        self.snapshot
            .as_deref()
            .map(|snapshot| Listing::new(snapshot, &self.arrangement, now))
    }

    fn show(&mut self, snapshot: Arc<Snapshot>, now: SystemTime) {
        self.notifier.process_snapshot(&snapshot);
        self.cleaning.settle(&snapshot);
        self.arrangement.observe(&snapshot, now);
        if self.settings.folded_repositories.forget_gone(&snapshot) {
            self.save_settings();
        }
        self.snapshot = Some(snapshot);
    }

    /// The user clicked the header of the group of the repository whose git folder reads
    /// `common_dir`: fold it shut or open it, and keep that for the next run.
    fn toggle_fold(&mut self, common_dir: &str) {
        let Some(map) = &self.snapshot else {
            return;
        };
        if self.settings.folded_repositories.toggle(common_dir, map) {
            self.save_settings();
        }
    }

    /// Start an engine on the roots in place of the one running, and start afresh all that follows
    /// its map. With no folder granted there is nothing to scan until the user chooses one.
    fn start_engine(&mut self) {
        self.snapshot = None;
        self.engine_stopped = false;
        self.notifier.forget_map();
        self.arrangement.forget_map();
        if self.roots.is_empty() {
            return;
        }
        let roots = Roots {
            home: self.home.clone(),
            dirs: self.roots.clone(),
            sandboxed: self.sandboxed,
        };
        let options = self.settings.scan_options();
        let replaced = self.watching.take();
        self.watching = Some(Watching::start(roots, options, replaced, &self.events));
    }

    /// The user granted `folder` in the store edition's open panel: keep the grant for the next
    /// launch, and watch the folder in place of those watched now.
    fn folder_granted(&mut self, folder: String) {
        // Only the store edition reads a saved grant at launch; the direct edition reads the whole
        // home folder every time.
        if self.sandboxed {
            if let Err(err) = grant::save_grant(&folder) {
                self.dialogs.push(Alert::notice(
                    "Sutra will ask for this folder again next time",
                    format!("It can read the folder until it quits, but {err}."),
                ));
            }
        }
        self.roots = vec![PathBuf::from(folder)];
        self.lost_grant = None;
        self.start_engine();
    }

    /// The user chose whether Sutra watches the private folder named `folder`: keep it for the next
    /// run, and start the engine afresh on what the scanner may now enter. A name the window could
    /// only have made up changes nothing.
    fn watch_private_folder(&mut self, folder: &str, watch: bool) {
        let Some(folder) = PrivateFolder::named(folder) else {
            return;
        };
        let watched = &mut self.settings.watched_private_folders;
        let changed = if watch {
            watched.insert(folder)
        } else {
            watched.remove(&folder)
        };
        if changed {
            self.save_settings();
            self.start_engine();
        }
    }

    /// The user chose how much free space Sutra keeps by cleaning by itself. Turning that off goes
    /// at once; turning it on, or to another amount, waits for a yes to the question that says
    /// what will happen.
    fn keep_free(&mut self, floor: Option<u64>) {
        match floor {
            None => self.keep_free_from_now(None),
            Some(floor) => {
                let free = cleaning::free_bytes(&self.home).ok();
                let question = cleaning::keep_free_question(floor, free);
                let alert = Alert::question(question, Proposal::KeepFree(floor));
                self.dialogs.push(alert);
            }
        }
    }

    /// Keep `floor` free from now on, and remember it for the next run.
    fn keep_free_from_now(&mut self, floor: Option<u64>) {
        self.cleaning.set_keep_free_floor(floor);
        self.settings.keep_free_bytes = floor;
        self.save_settings();
    }

    fn save_settings(&mut self) {
        if let Err(err) = self.settings.save(self.settings_file.as_deref()) {
            self.dialogs.push(Alert::notice(
                "Sutra will forget this when it quits",
                format!("It couldn't save the setting: {err}."),
            ));
        }
    }

    /// The user answered alert `dialog`, at `now`: `yes` to what a question proposed, or not.
    fn answer(&mut self, dialog: u64, yes: bool, now: SystemTime) {
        let Some(proposal) = self.dialogs.answer(dialog) else {
            return;
        };
        let map = self.live_map().cloned();
        let map = map.as_deref();
        match proposal {
            Proposal::Clean { clicked, prepared } => {
                let prepared = yes.then_some(*prepared);
                self.cleaning
                    .answered_clean(clicked, prepared, map, now, &self.home);
            }
            Proposal::CleanIdle(cleans) => {
                let cleans = yes.then_some(cleans);
                self.cleaning.answered_idle(cleans, map, now, &self.home);
            }
            Proposal::KeepFree(floor) => {
                if yes {
                    self.keep_free_from_now(Some(floor));
                }
            }
        }
    }

    /// The user chose Clean on the build folder at `folder`, as a card named it with
    /// `format::path_text`.
    fn clean(&mut self, folder: &str) {
        let Some(map) = self.snapshot.clone() else {
            return;
        };
        let found = map.projects.iter().find_map(|project| {
            let dir = project
                .artifacts
                .iter()
                .find(|dir| format::path_text(&dir.path) == folder)?;
            Some((project, dir))
        });
        // The folder left the map since the card showed it.
        let Some((project, dir)) = found else {
            return;
        };
        self.cleaning
            .start(project, dir, Arc::clone(&map), &self.home);
    }

    /// The user chose the toolbar's clean of every idle build folder, at `now`.
    fn clean_idle(&mut self, now: SystemTime) {
        if let Some(map) = self.live_map().cloned() {
            self.cleaning.start_idle(map, now, &self.home);
        }
    }

    /// The user chose to terminate the environment whose supervisor is `pid`. Sutra is a
    /// situational-awareness dashboard with intentionally limited control: one shutdown button per
    /// env, which sends a single SIGTERM to the one PID the dev runner published, its supervisor.
    /// The supervisor's own trap reaps its children, does any orderly cleanup, and removes the
    /// registry entry. Signaling the process group ourselves would override the supervisor's
    /// shutdown sequence and overstep the dashboard boundary.
    fn terminate(&mut self, pid: u32) {
        // The menu that offered it may be a while old, and a pid freed since may be another
        // process's now, which must not get the signal: only one the menu would offer now does.
        let offered = self.snapshot.as_ref().is_some_and(|map| {
            map.projects
                .iter()
                .filter_map(|project| project.environment.as_ref())
                .any(|env| env.pid == pid && card::offers_terminate(env, map.sandboxed))
        });
        if !offered {
            return;
        }
        // Offered means alive, and only a pid `kill` takes is ever found alive
        // (`model::process_exists`).
        let Ok(raw) = i32::try_from(pid) else {
            return;
        };
        match signal::kill(Pid::from_raw(raw), Signal::SIGTERM) {
            // Gone already, which its card shows once the engine notices.
            Ok(()) | Err(Errno::ESRCH) => {}
            Err(err) => self.dialogs.push(Alert::notice(
                "Couldn't terminate the environment",
                format!("Signalling its process {pid} failed: {err}."),
            )),
        }
    }

    /// The map, while an engine keeps it up to date. Once the engine has stopped the last map only
    /// shows what was, which is nothing to decide a clean on.
    fn live_map(&self) -> Option<&Arc<Snapshot>> {
        self.snapshot.as_ref().filter(|_| !self.engine_stopped)
    }

    /// What the toolbar offers to clean at `now`, if anything.
    fn idle_offer(&self, now: SystemTime) -> Option<IdleOffer> {
        self.live_map()
            .and_then(|map| self.cleaning.idle_offer(map, now))
    }

    /// Whether the map can say what is idle.
    fn can_tell_what_is_idle(&self) -> bool {
        self.live_map()
            .is_some_and(|map| clean::can_tell_what_is_at_work(map))
    }

    /// Pass on what the cleans have for others: alerts to the dialogs, banners to the notifier,
    /// and the folders they just deleted from to the engine, so their cards show what is left.
    fn pass_on_clean_news(&mut self) {
        for alert in self.cleaning.take_alerts() {
            self.dialogs.push(alert);
        }
        for (title, body) in self.cleaning.take_banners() {
            self.notifier.banner(&title, &body);
        }
        let folders = self.cleaning.take_measure_again();
        if let Some(watching) = self.watching.as_ref().filter(|_| !folders.is_empty()) {
            watching.measure_after_clean(folders);
        }
    }

    fn sent_by_the_running_engine(&self, id: EngineId) -> bool {
        self.watching
            .as_ref()
            .is_some_and(|watching| watching.started(id))
    }
}
