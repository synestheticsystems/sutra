//! Sounds, speech and banners for the transitions the user wants to hear about: a dev-runner unit
//! changing state, and a coding agent that has been waiting on the user for a while.

mod transitions;

use std::collections::HashSet;
use std::mem;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::SystemTime;

use crate::map::Snapshot;
use transitions::{Announcement, Transitions};

/// Action sent to the background audio/speech thread.
enum Action {
    SoundAndSpeak { sound: Sound, text: String },
    Shutdown,
}

/// The macOS system sounds Sutra plays, in rising priority, because a batch of transitions plays
/// only its highest: bad news first, then a request for the user, then good news, then progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Sound {
    /// A unit is starting or building.
    Submarine,
    /// A unit is ready or running.
    Ping,
    /// An agent is waiting on the user.
    Glass,
    /// A unit failed.
    Basso,
}

impl Sound {
    /// The file stem in `/System/Library/Sounds/`.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn system_name(self) -> &'static str {
        match self {
            Sound::Submarine => "Submarine",
            Sound::Ping => "Ping",
            Sound::Glass => "Glass",
            Sound::Basso => "Basso",
        }
    }
}

/// Build a combined key for unit lookups (avoids allocating a tuple of two Strings).
fn unit_key(env_id: &str, unit_name: &str) -> String {
    format!("{}\x00{}", env_id, unit_name)
}

/// Shows a banner, a title and a body, in Notification Center: the app that owns the window does,
/// as the system lets it.
type PostBanner = Box<dyn Fn(&str, &str) + Send>;

/// Notifier detects state transitions and plays sounds/speech/notifications.
pub struct Notifier {
    tx: mpsc::Sender<Action>,
    thread: Option<JoinHandle<()>>,
    post_banner: PostBanner,
    pub global_mute: bool,
    pub muted_units: HashSet<String>,
    pub global_notifications_off: bool,
    pub notifications_off_units: HashSet<String>,
    transitions: Transitions,
}

impl Notifier {
    /// A notifier that sends its banners to `post_banner`, and speaks and plays its sounds on a
    /// thread of its own.
    pub fn new(post_banner: impl Fn(&str, &str) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();

        let thread = thread::spawn(move || {
            Self::audio_thread(rx);
        });

        Notifier {
            tx,
            thread: Some(thread),
            post_banner: Box::new(post_banner),
            global_mute: false,
            muted_units: HashSet::new(),
            global_notifications_off: false,
            notifications_off_units: HashSet::new(),
            transitions: Transitions::default(),
        }
    }

    fn audio_thread(rx: mpsc::Receiver<Action>) {
        // Init rodio (optional — skip if unavailable)
        #[cfg(target_os = "macos")]
        let audio_stream = rodio::OutputStreamBuilder::open_default_stream().ok();

        // Init TTS via AppKit backend (NSSpeechSynthesizer) — uses the same
        // voice as `say`, which is the system default from System Preferences.
        #[cfg(target_os = "macos")]
        let mut tts_engine = tts::Tts::new(tts::Backends::AppKit).ok();

        for action in rx {
            match action {
                Action::SoundAndSpeak {
                    #[cfg(target_os = "macos")]
                    sound,
                    #[cfg(not(target_os = "macos"))]
                        sound: _,
                    #[cfg(target_os = "macos")]
                    text,
                    #[cfg(not(target_os = "macos"))]
                        text: _,
                } => {
                    // Play system sound
                    #[cfg(target_os = "macos")]
                    if let Some(ref stream) = audio_stream {
                        let sound_path =
                            format!("/System/Library/Sounds/{}.aiff", sound.system_name());
                        if let Ok(file) = std::fs::File::open(&sound_path) {
                            if let Ok(source) = rodio::Decoder::new(std::io::BufReader::new(file)) {
                                let sink = rodio::Sink::connect_new(stream.mixer());
                                sink.append(source);
                                sink.sleep_until_end();
                            }
                        }
                    }

                    // Speak — poll is_speaking() since AppKit backend has no callbacks
                    #[cfg(target_os = "macos")]
                    if let Some(ref mut tts) = tts_engine {
                        if tts.speak(&text, false).is_ok() {
                            while tts.is_speaking().unwrap_or(false) {
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                        }
                    }
                }
                Action::Shutdown => break,
            }
        }
    }

    /// Detect the unit and agent transitions since the previous snapshot and fire
    /// sounds/notifications: units at once, and an agent's wait once it has lasted (see
    /// `announce_lasting_waits`). An agent outside every project is named after its working
    /// folder.
    pub fn process_snapshot(&mut self, snapshot: &Snapshot) {
        let envs = snapshot
            .projects
            .iter()
            .filter_map(|project| project.environment.as_ref());
        let placed = snapshot.projects.iter().flat_map(|project| {
            project
                .agents
                .iter()
                .map(move |agent| (project.name.as_str(), agent))
        });
        let unplaced = snapshot.unplaced_agents.iter().map(|agent| {
            let folder = agent.cwd.file_name().and_then(|name| name.to_str());
            // The GUI lists these agents under "Elsewhere"; "/" and non-UTF-8 names have no
            // better short name.
            (folder.unwrap_or("Elsewhere"), agent)
        });
        let announcements =
            self.transitions
                .advance(envs, placed.chain(unplaced), SystemTime::now());
        self.announce(announcements);
    }

    /// Announce the agent waits that have lasted long enough by now. A snapshot comes only when
    /// something changes, so a wait that simply goes on needs this called on a timer while
    /// `has_pending_waits` holds.
    pub fn announce_lasting_waits(&mut self) {
        let announcements = self.transitions.lasting_waits(SystemTime::now());
        self.announce(announcements);
    }

    /// Whether an agent's wait is still to be announced, should it last.
    pub fn has_pending_waits(&self) -> bool {
        self.transitions.has_pending_waits()
    }

    /// Forget the previous map, so the first snapshot of an engine started afresh is recorded
    /// rather than announced as a flood of changes.
    pub fn forget_map(&mut self) {
        self.transitions = Transitions::default();
    }

    /// Send the banners the toggles allow, then play one sound and speak every audible
    /// transition in one utterance, so a burst of transitions does not queue a string of sounds.
    fn announce(&self, announcements: Vec<Announcement>) {
        let mut audible = Vec::new();
        for announcement in announcements {
            let unit_key = announcement.unit_key.as_deref();
            // Banners are independent of the sound mute.
            let banners_off = self.global_notifications_off
                || unit_key.is_some_and(|key| self.notifications_off_units.contains(key));
            if !banners_off {
                (self.post_banner)(&announcement.banner_title, &announcement.banner_body);
            }
            let muted =
                self.global_mute || unit_key.is_some_and(|key| self.muted_units.contains(key));
            if !muted {
                audible.push(announcement);
            }
        }
        if let Some((sound, text)) = utterance(audible) {
            let _ = self.tx.send(Action::SoundAndSpeak { sound, text });
        }
    }

    /// Send a banner no unit is behind, unless banners are off.
    pub fn banner(&self, title: &str, body: &str) {
        if !self.global_notifications_off {
            (self.post_banner)(title, body);
        }
    }

    pub fn toggle_global_mute(&mut self) {
        self.global_mute = !self.global_mute;
    }

    pub fn toggle_unit_mute(&mut self, env_id: &str, unit_name: &str) {
        let key = unit_key(env_id, unit_name);
        if !self.muted_units.remove(&key) {
            self.muted_units.insert(key);
        }
    }

    pub fn is_unit_muted(&self, env_id: &str, unit_name: &str) -> bool {
        self.muted_units.contains(&unit_key(env_id, unit_name))
    }

    pub fn toggle_global_notifications(&mut self) {
        self.global_notifications_off = !self.global_notifications_off;
    }

    pub fn toggle_unit_notifications(&mut self, env_id: &str, unit_name: &str) {
        let key = unit_key(env_id, unit_name);
        if !self.notifications_off_units.remove(&key) {
            self.notifications_off_units.insert(key);
        }
    }

    pub fn is_unit_notifications_off(&self, env_id: &str, unit_name: &str) -> bool {
        self.notifications_off_units
            .contains(&unit_key(env_id, unit_name))
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        let _ = self.tx.send(Action::Shutdown);
        if let Some(thread) = mem::take(&mut self.thread) {
            let _ = thread.join();
        }
    }
}

/// One sound and one sentence for a batch: the highest-priority sound, and every transition
/// spoken in the order it was found. `None` when nothing is audible.
fn utterance(audible: Vec<Announcement>) -> Option<(Sound, String)> {
    let sound = audible
        .iter()
        .map(|announcement| announcement.sound)
        .max()?;
    let speech: Vec<String> = audible
        .into_iter()
        .map(|announcement| announcement.speech)
        .collect();
    Some((sound, speech.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audible(sound: Sound, speech: &str) -> Announcement {
        Announcement {
            sound,
            speech: speech.to_string(),
            banner_title: String::new(),
            banner_body: String::new(),
            unit_key: None,
        }
    }

    #[test]
    fn a_batch_plays_its_highest_priority_sound_and_speaks_every_transition_in_order() {
        let batch = vec![
            audible(Sound::Submarine, "server building"),
            audible(Sound::Basso, "web failed"),
            audible(Sound::Glass, "app Claude waiting"),
        ];
        assert_eq!(
            utterance(batch),
            Some((
                Sound::Basso,
                "server building, web failed, app Claude waiting".to_string()
            ))
        );
    }

    #[test]
    fn an_agent_waiting_outranks_a_unit_becoming_ready() {
        let batch = vec![
            audible(Sound::Ping, "server ready"),
            audible(Sound::Glass, "app Claude waiting"),
        ];
        assert_eq!(utterance(batch).unwrap().0, Sound::Glass);
    }

    #[test]
    fn nothing_audible_makes_no_utterance() {
        assert_eq!(utterance(Vec::new()), None);
    }
}
