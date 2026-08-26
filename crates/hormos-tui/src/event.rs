//! Lecture des événements du clavier.
//!
//! `crossterm` lit le terminal de façon **bloquante**. Plutôt que d'ajouter une
//! dépendance asynchrone supplémentaire (`event-stream`, qui tire `futures` et
//! `signal-hook-mio`), Hormos consacre un fil système à cette lecture et publie
//! les événements dans un canal **borné** : si l'interface prend du retard, la
//! lecture ralentit au lieu de faire grossir une file sans limite.
//!
//! # Arrêt
//!
//! La contre-pression et l'arrêt doivent coexister. Un envoi bloquant classique
//! les oppose : un fil arrêté au milieu d'un envoi sur un canal plein n'observe
//! plus l'arrêt demandé, et l'attente ne se termine que si quelqu'un vide le
//! canal — ce que la boucle principale, déjà terminée, ne fera jamais.
//!
//! L'envoi est donc **annulable** ([`send_cancellable`]) : il réessaie par
//! tranches de [`BACKPRESSURE_PARK`] et vérifie l'arrêt entre deux tentatives.
//! [`InputReader::drop`] demande l'arrêt, puis réveille le fil : l'attente de
//! contre-pression est interrompue immédiatement.
//!
//! Le fil s'arrête donc au plus tard [`POLL_INTERVAL`] après la destruction du
//! lecteur — le seul point d'attente qui ne peut pas être réveillé étant la
//! lecture du terminal elle-même. Le terminal n'est ainsi jamais restauré
//! pendant qu'un fil lit encore l'entrée standard.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::TrySendError;

use crate::app::Message;

/// Pas d'attente entre deux vérifications de l'arrêt demandé.
///
/// C'est aussi le délai maximal d'arrêt du fil : la lecture du terminal est le
/// seul point d'attente qu'un réveil ne peut pas interrompre.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Attente entre deux tentatives d'envoi sur un canal saturé.
///
/// Assez courte pour rester imperceptible à la frappe, assez longue pour ne pas
/// occuper un cœur à tourner à vide.
const BACKPRESSURE_PARK: Duration = Duration::from_millis(20);

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
        if !send_cancellable(sender, stop, message) {
            break;
        }
    }
}

/// Publie un message en respectant la contre-pression, sans jamais rendre
/// l'arrêt impossible.
///
/// Renvoie `false` si l'arrêt a été demandé ou si la boucle principale est
/// terminée. Dans les deux cas, le message est abandonné : plus personne ne
/// pourrait le traiter.
fn send_cancellable(sender: &Sender<Message>, stop: &AtomicBool, message: Message) -> bool {
    let mut pending = message;
    loop {
        // Vérifié **avant** chaque tentative : un arrêt déjà demandé n'envoie
        // rien du tout.
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        match sender.try_send(pending) {
            Ok(()) => return true,
            Err(TrySendError::Closed(_)) => return false,
            Err(TrySendError::Full(returned)) => {
                // Le message est conservé, jamais perdu silencieusement.
                pending = returned;
                // Réveillable par `InputReader::drop` : l'attente cesse dès que
                // l'arrêt est demandé, sans dépendre du consommateur.
                thread::park_timeout(BACKPRESSURE_PARK);
            }
        }
    }
}

impl Drop for InputReader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            // L'ordre compte : l'arrêt est visible avant le réveil, donc le fil
            // ne peut pas se rendormir sur une attente de contre-pression.
            handle.thread().unpark();
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use tokio::sync::mpsc;

    use super::{BACKPRESSURE_PARK, send_cancellable};
    use crate::app::Message;

    /// Marge confortable : l'assertion ne doit jamais échouer sur une machine
    /// chargée, mais un vrai blocage reste détecté en quelques secondes.
    const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);

    /// Attend la fin d'un fil sans jamais bloquer la suite de tests.
    ///
    /// `JoinHandle::join` sur un fil bloqué figerait la CI : on interroge donc
    /// `is_finished` jusqu'à une échéance stricte, et on échoue avant de joindre.
    fn join_before(handle: thread::JoinHandle<bool>, deadline: Duration) -> bool {
        let limit = Instant::now() + deadline;
        while !handle.is_finished() {
            assert!(
                Instant::now() < limit,
                "le fil d'entrée ne s'est pas arrêté : blocage sur l'envoi"
            );
            thread::sleep(Duration::from_millis(5));
        }
        handle.join().unwrap_or_else(|_| panic!("fil en panique"))
    }

    #[test]
    fn an_available_channel_takes_the_message() {
        let (sender, mut receiver) = mpsc::channel::<Message>(1);
        let stop = AtomicBool::new(false);

        assert!(send_cancellable(&sender, &stop, Message::Resized));
        assert_eq!(receiver.try_recv().ok(), Some(Message::Resized));
    }

    #[test]
    fn a_closed_channel_ends_the_thread() {
        let (sender, receiver) = mpsc::channel::<Message>(1);
        drop(receiver);
        let stop = AtomicBool::new(false);

        assert!(!send_cancellable(&sender, &stop, Message::Resized));
    }

    #[test]
    fn an_already_requested_stop_sends_nothing() {
        let (sender, mut receiver) = mpsc::channel::<Message>(1);
        let stop = AtomicBool::new(true);

        assert!(!send_cancellable(&sender, &stop, Message::Resized));
        assert!(
            receiver.try_recv().is_err(),
            "message envoyé malgré l'arrêt"
        );
    }

    /// Régression : un canal saturé ne doit pas rendre l'arrêt impossible.
    ///
    /// C'est exactement la situation d'un envoi bloquant non annulable — rafale
    /// de touches, canal plein, boucle principale terminée donc plus personne
    /// pour vider : le fil attendrait un consommateur qui ne reviendra jamais,
    /// et la destruction du lecteur ne rendrait jamais la main.
    #[test]
    fn a_full_channel_never_blocks_the_shutdown() {
        let (sender, mut receiver) = mpsc::channel::<Message>(1);
        // Le canal est plein, et le receiver reste vivant : le canal n'est donc
        // pas fermé. L'envoi ne peut aboutir d'aucune façon.
        assert!(sender.try_send(Message::Resized).is_ok());

        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let attempt = sender.clone();
        let handle = thread::spawn(move || send_cancellable(&attempt, &flag, Message::Resized));

        // Le fil est nécessairement en contre-pression : aucune issue ne lui
        // permet de terminer tant que le canal reste plein et ouvert.
        thread::sleep(BACKPRESSURE_PARK * 3);
        assert!(
            !handle.is_finished(),
            "l'envoi aurait dû rencontrer la contre-pression"
        );

        stop.store(true, Ordering::Relaxed);
        handle.thread().unpark();

        assert!(
            !join_before(handle, SHUTDOWN_DEADLINE),
            "l'arrêt aurait dû interrompre l'envoi"
        );
        // Le message annulé n'est jamais arrivé : seul celui d'amorçage est là.
        assert!(receiver.try_recv().is_ok());
        assert!(
            receiver.try_recv().is_err(),
            "message publié après l'annulation"
        );
    }
}
