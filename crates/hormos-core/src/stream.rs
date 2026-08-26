//! Flux temps réel, indépendants du moteur.
//!
//! Un flux Hormos est un [`RuntimeStream<T>`] : une suite d'éléments du domaine
//! produits au fil de l'eau. Le type est **opaque** — les interfaces ne voient ni
//! `futures`, ni Bollard, ni HTTP — et se consomme par une seule méthode,
//! [`RuntimeStream::next`].
//!
//! # Pourquoi un type concret plutôt qu'un trait
//!
//! [`crate::runtime::ContainerRuntime`] est manipulé derrière `dyn`. Une méthode
//! renvoyant `impl Stream` ne serait pas utilisable ainsi ; renvoyer un type
//! concret qui *contient* un flux boxé garde le trait object-safe sans imposer
//! `#[async_trait]` là où le moteur n'a rien à attendre.
//!
//! # Bornes mémoire
//!
//! Un flux ne conserve **rien** : il n'accumule pas ses éléments, ne les
//! journalise pas et n'en garde pas de copie. Chaque élément est produit, remis à
//! l'appelant, puis relâché. Les bornes de rétention sont la responsabilité de
//! l'interface qui affiche (voir `docs/streams.md`).
//!
//! # Ouverture paresseuse
//!
//! Ouvrir un flux ne parle pas encore au moteur : pour Docker, la requête HTTP
//! n'est émise qu'à la **première** lecture. La conséquence est visible sur les
//! événements, qui ne se rejouent pas : s'abonner puis agir sur le moteur sans
//! avoir lu une seule fois manquerait les événements de cette action. Il faut
//! donc commencer à lire avant de provoquer ce que l'on veut observer. Un test
//! d'intégration l'exige explicitement.
//!
//! # Annulation
//!
//! Détruire un [`RuntimeStream`] libère la ressource sous-jacente — pour Docker,
//! la requête HTTP en cours. Il n'y a donc rien de plus à faire pour annuler : ni
//! jeton, ni message d'arrêt.

use std::fmt;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;

use crate::error::Result;

/// Flux d'éléments du domaine, produits au fil de l'eau.
///
/// Chaque élément est un [`Result`] : une erreur du moteur survenue **pendant**
/// le flux est transmise comme les autres, sans interrompre le programme.
pub struct RuntimeStream<T> {
    inner: Pin<Box<dyn Stream<Item = Result<T>> + Send>>,
}

impl<T> RuntimeStream<T> {
    /// Construit un flux à partir d'une source quelconque.
    pub fn new<S>(source: S) -> Self
    where
        S: Stream<Item = Result<T>> + Send + 'static,
    {
        Self {
            inner: Box::pin(source),
        }
    }

    /// Construit un flux en traduisant chaque élément d'une source.
    ///
    /// C'est la porte d'entrée des adaptateurs : ils fournissent le flux natif de
    /// leur moteur et la traduction vers le domaine, sans avoir à nommer le trait
    /// `Stream` ni à écrire une projection.
    pub fn mapped<S, I, F>(source: S, translate: F) -> Self
    where
        S: Stream<Item = I> + Send + 'static,
        I: 'static,
        T: 'static,
        F: FnMut(I) -> Result<T> + Send + 'static,
    {
        Self::new(Mapped {
            source: Box::pin(source),
            translate: Box::new(translate),
        })
    }

    /// Flux fini, entièrement connu d'avance. Réservé aux tests.
    pub fn from_items<I>(items: I) -> Self
    where
        I: IntoIterator<Item = Result<T>>,
        T: Send + Unpin + 'static,
    {
        Self::new(Items {
            items: items.into_iter().collect::<Vec<_>>().into_iter(),
        })
    }

    /// Flux qui ne produit jamais rien et ne se termine jamais.
    ///
    /// Représente un abonnement silencieux : seule sa destruction y met fin.
    #[must_use]
    pub fn never() -> Self
    where
        T: Send + 'static,
    {
        Self::new(Never {
            marker: PhantomData,
        })
    }

    /// Élément suivant, ou `None` quand le flux est terminé.
    pub async fn next(&mut self) -> Option<Result<T>> {
        std::future::poll_fn(|context| self.inner.as_mut().poll_next(context)).await
    }
}

impl<T> fmt::Debug for RuntimeStream<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeStream")
            .finish_non_exhaustive()
    }
}

/// Flux source dont chaque élément est traduit vers le domaine.
///
/// Les deux champs sont `Unpin` (un `Pin<Box<…>>` et un `Box<dyn FnMut>`) : la
/// projection se fait donc par `Pin::get_mut`, sans `unsafe` — que le workspace
/// interdit de toute façon.
struct Mapped<I, T> {
    source: Pin<Box<dyn Stream<Item = I> + Send>>,
    translate: Box<dyn FnMut(I) -> Result<T> + Send>,
}

impl<I, T> Stream for Mapped<I, T> {
    type Item = Result<T>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match this.source.as_mut().poll_next(context) {
            Poll::Ready(Some(item)) => Poll::Ready(Some((this.translate)(item))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Flux fini adossé à une collection.
struct Items<T> {
    items: std::vec::IntoIter<Result<T>>,
}

impl<T: Unpin> Stream for Items<T> {
    type Item = Result<T>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.get_mut().items.next())
    }
}

/// Flux muet et sans fin.
struct Never<T> {
    marker: PhantomData<fn() -> T>,
}

impl<T> Stream for Never<T> {
    type Item = Result<T>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Items, RuntimeStream};
    use crate::error::{HormosError, Result};

    /// Source brute, pour éprouver l'adaptateur de traduction.
    fn source<T>(items: impl IntoIterator<Item = Result<T>>) -> Items<T> {
        Items {
            items: items.into_iter().collect::<Vec<_>>().into_iter(),
        }
    }

    #[tokio::test]
    async fn a_finite_stream_yields_then_ends() {
        let mut stream = RuntimeStream::from_items([Ok(1_u8), Ok(2)]);

        assert_eq!(stream.next().await, Some(Ok(1)));
        assert_eq!(stream.next().await, Some(Ok(2)));
        assert_eq!(stream.next().await, None);
    }

    #[tokio::test]
    async fn an_empty_stream_ends_immediately() {
        let mut stream = RuntimeStream::<u8>::from_items([]);
        assert_eq!(stream.next().await, None);
    }

    #[tokio::test]
    async fn an_error_in_the_middle_is_delivered_like_any_item() {
        let boom = HormosError::runtime("boom");
        let mut stream = RuntimeStream::from_items([Ok(1_u8), Err(boom.clone()), Ok(3)]);

        assert_eq!(stream.next().await, Some(Ok(1)));
        assert_eq!(stream.next().await, Some(Err(boom)));
        // Le flux n'est pas rompu par une erreur : c'est l'appelant qui décide.
        assert_eq!(stream.next().await, Some(Ok(3)));
        assert_eq!(stream.next().await, None);
    }

    #[tokio::test]
    async fn mapped_translates_every_item() {
        let mut stream = RuntimeStream::mapped(source([Ok(1_u8), Ok(2)]), |item: Result<u8>| {
            item.map(|value| value * 10)
        });

        assert_eq!(stream.next().await, Some(Ok(10)));
        assert_eq!(stream.next().await, Some(Ok(20)));
        assert_eq!(stream.next().await, None);
    }

    #[tokio::test]
    async fn mapped_can_turn_an_item_into_an_error() {
        let mut stream =
            RuntimeStream::<u8>::mapped(source([Ok(1_u8)]), |_| Err(HormosError::runtime("nope")));

        assert_eq!(stream.next().await, Some(Err(HormosError::runtime("nope"))));
        assert_eq!(stream.next().await, None);
    }

    #[tokio::test]
    async fn a_never_ending_stream_never_yields() {
        let mut stream = RuntimeStream::<u8>::never();
        let outcome = tokio::time::timeout(Duration::from_millis(20), stream.next()).await;
        assert!(outcome.is_err(), "un flux muet ne doit rien produire");
    }
}
