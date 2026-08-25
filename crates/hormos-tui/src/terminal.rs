//! Cycle de vie du terminal.
//!
//! Le terminal est restauré **quoi qu'il arrive** :
//!
//! - sortie normale ou erreur : le `Drop` de [`TerminalGuard`] rend le curseur,
//!   quitte l'écran alternatif et désactive le mode brut ;
//! - panique : le gestionnaire installé par `ratatui::try_init` restaure l'écran
//!   avant de laisser la panique se propager. C'est le helper officiel de
//!   Ratatui — Hormos ne remplace jamais le gestionnaire de panique lui-même.
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
