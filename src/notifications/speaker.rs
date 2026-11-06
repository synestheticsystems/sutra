//! Saying announcements aloud: each one's sound, then its sentence, on a thread of the speaker's
//! own, one announcement after another so that none talks over another. The app waits for that
//! thread as it quits, so stopping the speaker cuts short what it is saying and drops what waits
//! its turn.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
#[cfg(target_os = "macos")]
use std::time::Duration;

use super::Sound;

/// What reaches the speaker's thread.
enum Message {
    /// An announcement to say once those before it have been said: its sound, then its sentence.
    Utterance { sound: Sound, text: String },
    /// Wakes the thread to end, should it be waiting for the next announcement.
    Stop,
}

/// Says the notifier's announcements aloud, on a thread of its own.
pub(super) struct Speaker {
    messages: Sender<Message>,
    /// Set as the speaker stops: its thread cuts short what it is saying, and says nothing more.
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Speaker {
    /// A speaker that says announcements in the system's sounds and voice.
    pub(super) fn new() -> Speaker {
        Speaker::start(SystemVoice::open)
    }

    /// A speaker whose thread says each announcement in the voice `open` makes there.
    fn start<V: Voice>(open: impl FnOnce() -> V + Send + 'static) -> Speaker {
        let (messages, inbox) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let thread = {
            let stopping = Arc::clone(&stopping);
            thread::Builder::new()
                .name("sutra-speaker".into())
                .spawn(move || {
                    let mut voice = open();
                    while let Ok(Message::Utterance { sound, text }) = inbox.recv() {
                        // Stopping: what still waits its turn goes unsaid.
                        if stopping.load(Ordering::Relaxed) {
                            break;
                        }
                        voice.utter(sound, &text, &stopping);
                    }
                })
                .expect("spawn the speaker thread")
        };
        Speaker {
            messages,
            stopping,
            thread: Some(thread),
        }
    }

    /// Play `sound`, then speak `text`, once every announcement before them has been said.
    pub(super) fn say(&self, sound: Sound, text: String) {
        // Fails only once the thread has ended, which only a panic makes it do.
        let _ = self.messages.send(Message::Utterance { sound, text });
    }
}

impl Drop for Speaker {
    /// The app waits for this as it quits, so it cuts short what is being said and drops what
    /// waits its turn, rather than wait for every announcement queued to be said.
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        // Fails only once the thread has ended, when there is nothing to wake.
        let _ = self.messages.send(Message::Stop);
        if let Some(thread) = self.thread.take() {
            // A panic on the thread was printed as it happened, and the thread is gone either way.
            let _ = thread.join();
        }
    }
}

/// What the speaker says an announcement in.
trait Voice {
    /// Play `sound`, then speak `text`, now, cutting both short once `stopping` is set.
    fn utter(&mut self, sound: Sound, text: &str, stopping: &AtomicBool);
}

/// The system's sounds, played through the default output, and the voice the user chose for
/// spoken content in System Settings, which the `say` command speaks in too. Either may fail to
/// open, and the other still plays.
#[cfg(target_os = "macos")]
struct SystemVoice {
    output: Option<rodio::OutputStream>,
    speech: Option<tts::Tts>,
}

/// Off macOS there are no system sounds or voice to say announcements in, so they go unsaid.
#[cfg(not(target_os = "macos"))]
struct SystemVoice;

/// How often the system voice looks whether a sound or a sentence has ended, and whether the
/// speaker is stopping.
#[cfg(target_os = "macos")]
const LOOK_AGAIN: Duration = Duration::from_millis(50);

#[cfg(target_os = "macos")]
impl SystemVoice {
    fn open() -> SystemVoice {
        SystemVoice {
            output: rodio::OutputStreamBuilder::open_default_stream().ok(),
            // NSSpeechSynthesizer, whose voice is the system's.
            speech: tts::Tts::new(tts::Backends::AppKit).ok(),
        }
    }

    /// Play `sound` through the default output, until it ends or `stopping` is set.
    fn play(&self, sound: Sound, stopping: &AtomicBool) {
        let Some(output) = &self.output else {
            return;
        };
        let path = format!("/System/Library/Sounds/{}.aiff", sound.system_name());
        // A system sound that cannot be read leaves the sentence to be said alone.
        let source = std::fs::File::open(path)
            .ok()
            .and_then(|file| rodio::Decoder::new(std::io::BufReader::new(file)).ok());
        let Some(source) = source else {
            return;
        };
        let sink = rodio::Sink::connect_new(output.mixer());
        sink.append(source);
        // The sink stops its sound as it drops here, so a wait cut short cuts the sound short too.
        wait_until(|| sink.empty(), stopping);
    }

    /// Speak `text`, until the sentence ends or `stopping` is set.
    fn speak(&mut self, text: &str, stopping: &AtomicBool) {
        let Some(speech) = &mut self.speech else {
            return;
        };
        if speech.speak(text, false).is_err() {
            return;
        }
        // The AppKit backend has no callbacks, so its speaking is polled. One that cannot say
        // whether it is speaking is taken to have finished.
        if !wait_until(|| !speech.is_speaking().unwrap_or(false), stopping) {
            // The speaker is stopping, and no one is left to tell should the voice fail to stop.
            let _ = speech.stop();
        }
    }
}

#[cfg(target_os = "macos")]
impl Voice for SystemVoice {
    fn utter(&mut self, sound: Sound, text: &str, stopping: &AtomicBool) {
        self.play(sound, stopping);
        if !stopping.load(Ordering::Relaxed) {
            self.speak(text, stopping);
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl SystemVoice {
    fn open() -> SystemVoice {
        SystemVoice
    }
}

#[cfg(not(target_os = "macos"))]
impl Voice for SystemVoice {
    fn utter(&mut self, _sound: Sound, _text: &str, _stopping: &AtomicBool) {}
}

/// Wait until `ended` holds, unless `stopping` is set first. Whether it ended.
#[cfg(target_os = "macos")]
fn wait_until(ended: impl Fn() -> bool, stopping: &AtomicBool) -> bool {
    loop {
        if ended() {
            return true;
        }
        if stopping.load(Ordering::Relaxed) {
            return false;
        }
        thread::sleep(LOOK_AGAIN);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// How long a sentence of `Endless` goes on unless the speaker stops it, so a speaker that
    /// fails to stop it fails the test rather than hang it.
    const SENTENCE: Duration = Duration::from_secs(10);

    /// A voice that tells the test each sentence it begins, then goes on with it until the speaker
    /// stops, as a long sentence would.
    struct Endless {
        begun: Sender<String>,
    }

    impl Voice for Endless {
        fn utter(&mut self, _sound: Sound, text: &str, stopping: &AtomicBool) {
            let _ = self.begun.send(text.to_string());
            let end = Instant::now() + SENTENCE;
            while !stopping.load(Ordering::Relaxed) && Instant::now() < end {
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[test]
    fn stopping_mid_sentence_cuts_it_short_and_leaves_the_queued_announcements_unsaid() {
        let (begun, said) = mpsc::channel();
        let speaker = Speaker::start(move || Endless { begun });
        for text in ["server ready", "web failed", "app Claude waiting"] {
            speaker.say(Sound::Ping, text.to_string());
        }
        let first = said.recv_timeout(Duration::from_secs(30));
        assert_eq!(first.as_deref(), Ok("server ready"));

        let asked = Instant::now();
        drop(speaker);
        assert!(asked.elapsed() < SENTENCE / 2, "stopped mid-sentence");
        let begun_after: Vec<String> = said.try_iter().collect();
        assert!(
            begun_after.is_empty(),
            "said after stopping: {begun_after:?}"
        );
    }
}
