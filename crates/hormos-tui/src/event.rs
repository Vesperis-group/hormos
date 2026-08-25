//! Lecture des événements du clavier.
//!
//! `crossterm` lit le terminal de façon **bloquante**. Plutôt que d'ajouter une
//! dépendance asynchrone supplémentaire (`event-stream`, qui tire `futures` et
//! `signal-hook-mio`), Hormos consacre un fil système à cette lecture et publie
//! les événements dans un canal **borné** : si l'interface prend du retard, la
//! lecture ralentit au lieu de faire grossir une file sans limite.
//!
//! Le fil s'arrête au plus tard [`POLL_INTERVAL`] après la destruction du
//! lecteur, ce qui garantit que le terminal n'est jamais restauré pendant qu'un
//! fil lit encore l'entrée standard.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc::Sender;

use crate::app::Message;

/// Pas d'attente entre deux vérifications de l'arrêt demandé.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Fil de lecture du clavier, arrêté à la destruction.
pub struct InputReader {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

/// Démarre la lecture des événements du terminal.
pub fn spawn(sender: Sender<Message>) -> InputReader {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let handle = thread::spawn(move || read_loop(&sender, &flag));
    InputReader {
        stop,
        handle: Some(handle),
    }
}

fn read_loop(sender: &Sender<Message>, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        match event::poll(POLL_INTERVAL) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => break,
        }
        let Ok(event) = event::read() else { break };
        let message = match event {
            // `Release` et `Repeat` existent sur certains terminaux : ne traiter
            // que l'appui évite d'exécuter deux fois la même action.
            Event::Key(key) if key.kind == KeyEventKind::Press => Message::Key(key),
            Event::Resize(_, _) => Message::Resized,
            _ => continue,
        };
        // Canal borné : `blocking_send` applique la contre-pression. L'échec
        // signifie que la boucle principale est terminée.
        if sender.blocking_send(message).is_err() {
            break;
        }
    }
}

impl Drop for InputReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl fmt::Debug for InputReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InputReader")
            .field("stopped", &self.stop.load(Ordering::Relaxed))
            .finish()
    }
}
