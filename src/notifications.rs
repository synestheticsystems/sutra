use std::collections::{HashMap, HashSet};
use std::mem;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use crate::model::{Environment, State};

/// Action sent to the background audio/speech thread.
enum Action {
    SoundAndSpeak { sound: &'static str, text: String },
    Shutdown,
}

/// Build a combined key for unit lookups (avoids allocating a tuple of two Strings).
fn unit_key(env_id: &str, unit_name: &str) -> String {
    format!("{}\x00{}", env_id, unit_name)
}

/// Notifier detects state transitions and plays sounds/speech/notifications.
pub struct Notifier {
    tx: mpsc::Sender<Action>,
    thread: Option<JoinHandle<()>>,
    pub global_mute: bool,
    pub muted_units: HashSet<String>,
    pub global_notifications_off: bool,
    pub notifications_off_units: HashSet<String>,
    prev_states: HashMap<(String, String), State>,
    first_load: bool,
}

impl Default for Notifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifier {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();

        let thread = thread::spawn(move || {
            Self::audio_thread(rx);
        });

        #[cfg(target_os = "macos")]
        let _ = mac_notification_sys::set_application("systems.synesthetic.sutra");

        Notifier {
            tx,
            thread: Some(thread),
            global_mute: false,
            muted_units: HashSet::new(),
            global_notifications_off: false,
            notifications_off_units: HashSet::new(),
            prev_states: HashMap::new(),
            first_load: true,
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
                        let sound_path = format!("/System/Library/Sounds/{}.aiff", sound);
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

    /// Detect transitions and fire sounds/notifications.
    /// Call this after each load_all().
    pub fn process(&mut self, envs: &[Environment]) {
        // Build current state map
        let mut current: HashMap<(String, String), State> = HashMap::new();
        for env in envs {
            for unit in &env.units {
                current.insert((env.id.clone(), unit.name.clone()), unit.state.clone());
            }
        }

        if self.first_load {
            // First load: snapshot states, no sounds
            self.prev_states = current;
            self.first_load = false;
            return;
        }

        // Diff against prev_states — collect all transitions, then batch audio.
        let mut batched_speeches: Vec<String> = Vec::new();
        let mut best_sound: Option<&'static str> = None;

        for (key, new_state) in &current {
            let changed = match self.prev_states.get(key) {
                Some(old_state) => !state_variant_eq(old_state, new_state),
                None => true, // new unit appeared
            };

            if !changed {
                continue;
            }

            // Determine sound/speech for this transition
            let (sound, speech) = match new_state {
                State::None | State::Other(_) | State::Stopped => (None, None),
                State::Building | State::Starting => {
                    (Some("Submarine"), Some(format!("{} {}", key.1, new_state)))
                }
                State::Ready | State::Running => {
                    (Some("Ping"), Some(format!("{} {}", key.1, new_state)))
                }
                State::Failed => (Some("Basso"), Some(format!("{} {}", key.1, new_state))),
            };

            let Some(sound) = sound else { continue };

            let uk = unit_key(&key.0, &key.1);

            // Send macOS notification (independent of sound mute)
            let notifications_off =
                self.global_notifications_off || self.notifications_off_units.contains(&uk);
            if !notifications_off {
                #[cfg(target_os = "macos")]
                {
                    let unit_name = &key.1;
                    let state_str = new_state.to_string();
                    let _ = mac_notification_sys::send_notification(
                        &format!("sutra — {}", unit_name),
                        None,
                        &state_str,
                        None,
                    );
                }
            }

            // Check sound mute
            if self.global_mute || self.muted_units.contains(&uk) {
                continue;
            }

            // Batch: collect speech and track highest-priority sound
            if let Some(text) = speech {
                batched_speeches.push(text);
                best_sound = Some(match best_sound {
                    None => sound,
                    Some(prev) => higher_priority_sound(prev, sound),
                });
            }
        }

        // Send one batched action: single sound + combined speech utterance
        if let Some(sound) = best_sound {
            if !batched_speeches.is_empty() {
                let text = batched_speeches.join(", ");
                let _ = self.tx.send(Action::SoundAndSpeak { sound, text });
            }
        }

        self.prev_states = current;
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

/// Compare two State values by variant, also comparing the inner string for Other.
fn state_variant_eq(a: &State, b: &State) -> bool {
    match (a, b) {
        (State::Other(a), State::Other(b)) => a == b,
        _ => mem::discriminant(a) == mem::discriminant(b),
    }
}

/// Return the higher-priority sound between two macOS system sounds.
/// Priority: Basso (failed) > Ping (ready/running) > Submarine (building/starting).
fn higher_priority_sound(a: &'static str, b: &'static str) -> &'static str {
    fn priority(s: &str) -> u8 {
        match s {
            "Basso" => 2,
            "Ping" => 1,
            _ => 0,
        }
    }
    if priority(b) > priority(a) { b } else { a }
}
