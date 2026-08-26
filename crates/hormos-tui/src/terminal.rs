//! Cycle de vie du terminal.
//!
//! La restitution dépend de la façon dont le programme se termine, et il vaut
//! mieux l'énoncer que le laisser croire :
//!
//! - **sortie normale ou erreur remontée** : le `Drop` de [`TerminalGuard`] rend
//!   le curseur, quitte l'écran alternatif et désactive le mode brut. Le
//!   terminal est intégralement restauré ;
//! - **panique** : le gestionnaire installé par `ratatui::try_init` restaure
//!   l'écran — mode brut et écran alternatif — avant de laisser la panique se
//!   propager. C'est le helper officiel de Ratatui ; Hormos ne remplace jamais
//!   le gestionnaire de panique lui-même. En revanche, le profil `release` du
//!   workspace utilise `panic = "abort"` : les `Drop` **ne s'exécutent pas**, et
//!   le curseur peut donc rester masqué. Un `reset` suffit à revenir à la
//!   normale.
//!
//! Cette limite est assumée : la corriger demanderait soit d'abandonner
//! `panic = "abort"`, soit d'installer un gestionnaire de panique propre à
//! Hormos — deux prix trop élevés pour un curseur.
//!
//! Aucune de ces opérations ne nécessite `unsafe` (interdit dans le workspace).

use std::fmt;
use std::io;

use ratatui::{DefaultTerminal, Frame};

/// Terminal en mode brut + écran alternatif, restauré à la destruction.
pub struct TerminalGuard {
    terminal: DefaultTerminal,
}

impl TerminalGuard {
    /// Prend le contrôle du terminal.
    ///
    /// # Errors
    ///
    /// Renvoie l'erreur d'entrée/sortie si le terminal refuse le mode brut ou
    /// l'écran alternatif. Dans ce cas, l'état déjà modifié est **rendu** avant
    /// de propager l'erreur.
    pub fn new() -> io::Result<Self> {
        let terminal = ratatui::try_init().inspect_err(|_| ratatui::restore())?;
        Ok(Self { terminal })
    }

    /// Dessine une image complète.
    ///
    /// # Errors
    ///
    /// Renvoie l'erreur d'entrée/sortie remontée par le terminal.
    pub fn draw(&mut self, render: impl FnOnce(&mut Frame)) -> io::Result<()> {
        self.terminal.draw(render)?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Le curseur est rendu tant que l'écran alternatif est encore actif, afin
        // que l'écran d'origine soit retrouvé intact.
        let _ = self.terminal.show_cursor();
        ratatui::restore();
    }
}

impl fmt::Debug for TerminalGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalGuard")
            .finish_non_exhaustive()
    }
}
