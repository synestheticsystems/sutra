//! The dashboard: what the window shows and what the user can do there, decided in Rust for the
//! SwiftUI window to draw. It runs the map engine, keeps the user's arrangement of the list, cleans
//! build folders, and announces what changed; after each change it works out the window's
//! contents and tells the window to read them again.
//!
//! One thread owns the dashboard's state (`state`) and handles its events a batch at a time
//! (`live`): what the user asks for, the engine's maps, what the cleaning threads report
//! (`cleaning`), and the clock. The list's order is `listing`'s to decide, and `contents` works out
//! each part of what the window shows, in the words `format` writes alike everywhere.

mod cleaning;
mod contents;
mod disk_space;
mod format;
mod listing;
mod live;
mod state;

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::SystemTime;

pub use cleaning::{IdleTime, WhenIdle};
pub use contents::*;
use live::Event;
use state::State;

/// The window that draws the dashboard, which the SwiftUI app implements. Both calls come on the
/// dashboard's own thread and must not wait for the main thread, which may itself be waiting in
/// `Dashboard::stop` for that thread to end.
#[uniffi::export(with_foreign)]
pub trait DashboardWindow: Send + Sync {
    /// What the window shows has changed: read it again with `Dashboard::contents`. Called as
    /// often as several times a second.
    fn changed(&self);
    /// Tell the user something in a notification banner, as Notification Center shows them.
    fn notify(&self, title: String, body: String);
}

/// The running dashboard, which the window holds for as long as it is open.
#[derive(uniffi::Object)]
pub struct Dashboard {
    /// Where `perform` and `stop` send the dashboard thread its work.
    events: Sender<Event>,
    /// What the window shows, as the dashboard thread last worked it out.
    contents: Arc<Mutex<WindowContents>>,
    /// The dashboard thread, until `stop` has waited for it to end.
    thread: Mutex<Option<JoinHandle<()>>>,
}

#[uniffi::export]
impl Dashboard {
    /// Start the dashboard on its own thread, scanning the folders this edition may read, and
    /// tell `window` each time there is something new to show.
    #[uniffi::constructor]
    pub fn start(window: Arc<dyn DashboardWindow>) -> Arc<Dashboard> {
        let (events, inbox) = mpsc::channel();
        let post_banner = {
            let window = Arc::clone(&window);
            move |title: &str, body: &str| window.notify(title.to_string(), body.to_string())
        };
        let state = State::launch(post_banner, events.clone());
        let contents = Arc::new(Mutex::new(state.contents(SystemTime::now())));
        let thread = {
            let contents = Arc::clone(&contents);
            thread::Builder::new()
                .name("sutra-dashboard".into())
                .spawn(move || live::run(state, &inbox, window.as_ref(), &contents))
                .expect("spawn the dashboard thread")
        };
        Arc::new(Dashboard {
            events,
            contents,
            thread: Mutex::new(Some(thread)),
        })
    }

    /// What the window shows now.
    pub fn contents(&self) -> WindowContents {
        lock(&self.contents).clone()
    }

    /// Do what the user asked. Returns at once; what it changes arrives through
    /// `DashboardWindow::changed`.
    pub fn perform(&self, action: Action) {
        // Fails only once the dashboard has stopped, when there is nothing left to do it.
        let _ = self.events.send(Event::Action(action));
    }

    /// Stop the engine and the dashboard's thread, as the app quits. Once this returns the window
    /// hears nothing more; a second call does nothing.
    pub fn stop(&self) {
        let Some(thread) = lock(&self.thread).take() else {
            return;
        };
        // Fails only when the thread has ended already, which the join then finds.
        let _ = self.events.send(Event::Stop);
        // A panic on the dashboard thread was printed when it happened, and the thread is gone
        // either way.
        let _ = thread.join();
    }
}

impl Drop for Dashboard {
    /// A window that lets go of the dashboard without stopping it would leave its thread at work,
    /// telling no one.
    fn drop(&mut self) {
        self.stop();
    }
}

/// The value `mutex` guards. Each guards a value replaced whole in one move, so a panic while one
/// was held leaves no half-made value behind to refuse.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
