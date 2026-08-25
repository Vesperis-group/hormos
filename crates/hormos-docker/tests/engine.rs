//! Tests d'intégration contre un **vrai** moteur Docker.
//!
//! Ces tests ne s'exécutent que si `HORMOS_DOCKER_INTEGRATION=1` est défini :
//! `cargo test --workspace` reste donc vert sur une machine sans Docker, et
//! personne ne touche accidentellement au moteur d'un poste de développement.
//!
//! Garanties d'innocuité :
//!
//! - chaque exécution de la suite possède une **identité propre**
//!   (`io.hormos.test.run`) et chaque conteneur une identité individuelle
//!   (`io.hormos.test.fixture`) ; aucune sélection ne se fait sur la seule
//!   étiquette `io.hormos.test=true`, qui appartient à toutes les suites ;
//! - l'image est **épinglée par digest** : aucune balise mouvante, aucun `latest` ;
//! - le nettoyage est garanti même en cas d'échec (garde `Drop`) et ne supprime
//!   que le conteneur de la fixture concernée ;
//! - aucun `prune`, aucune suppression d'image, aucune modification d'un
//!   conteneur préexistant.
//!
//! La **fixture** est créée et détruite via le client en ligne de commande
//! `docker`, invoqué sans shell (arguments passés en tableau) : le produit, lui,
//! ne parle à Docker que par Bollard. Créer et supprimer des conteneurs est hors
//! du périmètre d'Hormos à ce stade — le test ne doit donc pas dépendre de
//! fonctionnalités qui n'existent pas.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use hormos_core::domain::ContainerState;
use hormos_core::error::ErrorKind;
use hormos_core::service::ContainerService;
use hormos_docker::DockerRuntime;

/// Image de test, épinglée par digest (Alpine 3.22).
const IMAGE: &str =
    "alpine@sha256:14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce";

/// Étiquette commune à tous les conteneurs créés par la suite de tests.
///
/// Elle sert à **reconnaître** un conteneur de test, jamais à en sélectionner un
/// pour suppression : une autre exécution d'Hormos, sur le même démon, la porte
/// aussi.
const TEST_LABEL: &str = "io.hormos.test=true";

/// Étiquette d'identité de l'exécution courante de la suite.
const RUN_LABEL_KEY: &str = "io.hormos.test.run";

/// Étiquette d'identité d'un conteneur particulier.
const FIXTURE_LABEL_KEY: &str = "io.hormos.test.fixture";

/// Variable d'activation des tests contre un moteur réel.
const ENABLE_VAR: &str = "HORMOS_DOCKER_INTEGRATION";

/// Variable fournissant l'identité de l'exécution (positionnée par la CI).
const RUN_ID_VAR: &str = "HORMOS_DOCKER_TEST_RUN_ID";

/// Indique si les tests contre un moteur réel sont activés.
fn enabled() -> bool {
    std::env::var(ENABLE_VAR).is_ok_and(|value| value == "1")
}

/// Identifiant unique, monotone à l'échelle du processus.
fn unique() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default()
}

/// Identité de **l'exécution courante** de la suite, calculée une seule fois.
///
/// La CI fournit `HORMOS_DOCKER_TEST_RUN_ID` afin que l'étape de nettoyage de
/// secours puisse cibler exactement les conteneurs de ce job. En local, une
/// identité est dérivée du processus : elle doit rester **la même pour toute la
/// suite**, sinon un nettoyage global de l'exécution serait impossible.
fn suite_run_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let provided = std::env::var(RUN_ID_VAR).unwrap_or_default();
        let provided = provided.trim();
        if provided.is_empty() {
            return format!("local-{}-{}", std::process::id(), unique());
        }
        assert!(
            is_safe_label_value(provided),
            "{RUN_ID_VAR} contient des caractères inattendus"
        );
        provided.to_owned()
    })
}

/// Restreint l'identité d'exécution à un alphabet inoffensif.
///
/// La valeur devient une étiquette et un critère de filtre : la contraindre
/// évite qu'une variable mal formée n'élargisse involontairement une sélection.
fn is_safe_label_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Exécute `docker` sans shell et renvoie sa sortie standard.
fn docker(args: &[&str]) -> String {
    let output = Command::new("docker")
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("impossible d'exécuter docker {args:?} : {error}"));
    assert!(
        output.status.success(),
        "docker {args:?} a échoué : {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// Identifiants des conteneurs appartenant à une exécution donnée.
///
/// C'est **exactement** la sélection utilisée pour nettoyer : elle croise
/// l'étiquette générique et l'identité de l'exécution, de sorte qu'une autre
/// suite Hormos vivant sur le même démon ne peut jamais être sélectionnée.
fn containers_of_run(run_id: &str) -> Vec<String> {
    docker(&[
        "ps",
        "--all",
        "--quiet",
        "--no-trunc",
        "--filter",
        &format!("label={TEST_LABEL}"),
        "--filter",
        &format!("label={RUN_LABEL_KEY}={run_id}"),
    ])
    .lines()
    .filter(|line| !line.is_empty())
    .map(ToOwned::to_owned)
    .collect()
}

/// Conteneur de test, supprimé automatiquement en fin de test.
struct Fixture {
    name: String,
    run_id: String,
    fixture_id: String,
}

impl Fixture {
    /// Crée et démarre un conteneur de test appartenant à l'exécution courante.
    fn start() -> Self {
        Self::start_for_run(suite_run_id())
    }

    /// Crée un conteneur rattaché à l'exécution `run_id`.
    ///
    /// Utilisé par le test d'isolation pour simuler une **autre** suite Hormos.
    fn start_for_run(run_id: &str) -> Self {
        let fixture_id = unique().to_string();
        let name = format!("hormos-test-{fixture_id}");
        let run_label = format!("{RUN_LABEL_KEY}={run_id}");
        let fixture_label = format!("{FIXTURE_LABEL_KEY}={fixture_id}");

        docker(&[
            "run",
            "--detach",
            "--name",
            &name,
            "--label",
            TEST_LABEL,
            "--label",
            &run_label,
            "--label",
            &fixture_label,
            "--network",
            "none",
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            IMAGE,
            "sleep",
            "300",
        ]);

        Self {
            name,
            run_id: run_id.to_owned(),
            fixture_id,
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn run_id(&self) -> &str {
        &self.run_id
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Sélection par identité de fixture : ni `prune`, ni suppression d'un
        // conteneur d'une autre fixture, ni même d'une autre exécution.
        let ids = Command::new("docker")
            .args(["ps", "--all", "--quiet", "--no-trunc", "--filter"])
            .arg(format!("label={RUN_LABEL_KEY}={}", self.run_id))
            .arg("--filter")
            .arg(format!("label={FIXTURE_LABEL_KEY}={}", self.fixture_id))
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .unwrap_or_default();

        for id in ids.lines().filter(|line| !line.is_empty()) {
            let _ = Command::new("docker").args(["rm", "--force", id]).output();
        }
    }
}

/// Construit le service au-dessus du moteur local réel.
async fn service() -> ContainerService {
    let runtime = DockerRuntime::connect()
        .await
        .unwrap_or_else(|error| panic!("connexion au moteur impossible : {error}"));
    ContainerService::new(Arc::new(runtime))
}

/// Durée maximale d'attente d'une transition d'état côté moteur.
const SETTLE_ATTEMPTS: u32 = 60;
const SETTLE_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

/// Attend que le conteneur apparaisse (ou disparaisse) de la liste des actifs.
///
/// Le moteur répond à `stop` dès que le processus est mort, mais publie la
/// transition d'état dans une étape distincte : observer la liste dans la
/// foulée mesurerait la latence du démon, pas le comportement d'Hormos.
/// L'attente reste bornée : un moteur qui ne converge pas fait échouer le test.
async fn wait_until_listed_as_running(service: &ContainerService, name: &str, expected: bool) {
    for _ in 0..SETTLE_ATTEMPTS {
        let running = service.list_containers(false).await.expect("ps a échoué");
        if running.iter().any(|c| c.name == name) == expected {
            return;
        }
        tokio::time::sleep(SETTLE_DELAY).await;
    }
    panic!("le moteur n'a jamais listé « {name} » avec actif = {expected}");
}

/// Attend que l'inspection rapporte l'état d'exécution attendu.
async fn wait_until_state(
    service: &ContainerService,
    name: &str,
    expected: bool,
) -> hormos_core::domain::ContainerDetails {
    for _ in 0..SETTLE_ATTEMPTS {
        let details = service
            .inspect_container(name)
            .await
            .expect("inspect a échoué");
        if details.state.is_running() == expected {
            return details;
        }
        tokio::time::sleep(SETTLE_DELAY).await;
    }
    panic!("le moteur n'a jamais rapporté « {name} » avec actif = {expected}");
}

/// Identifiants de tous les conteneurs connus du moteur, hors fixture.
fn foreign_container_ids(fixture: &Fixture) -> Vec<String> {
    docker(&[
        "ps",
        "--all",
        "--no-trunc",
        "--format",
        "{{.ID}} {{.Names}}",
    ])
    .lines()
    .filter(|line| !line.contains(fixture.name()))
    .map(ToOwned::to_owned)
    .collect()
}

#[tokio::test]
async fn system_info_reports_a_real_engine() {
    if !enabled() {
        return;
    }
    let info = service().await.system_info().await.expect("info a échoué");

    assert!(info.server_version.is_some(), "version du serveur absente");
    assert!(info.api_version.is_some(), "version d'API absente");
    assert!(info.os.is_some(), "système absent");
    assert!(
        info.endpoint.starts_with('/'),
        "point de terminaison inattendu : {}",
        info.endpoint
    );
}

#[tokio::test]
async fn a_running_fixture_appears_in_both_listings() {
    if !enabled() {
        return;
    }
    let fixture = Fixture::start();
    let service = service().await;

    let running = service.list_containers(false).await.expect("ps a échoué");
    assert!(
        running.iter().any(|c| c.name == fixture.name()),
        "fixture absente de la liste des conteneurs actifs"
    );

    let all = service.list_containers(true).await.expect("ps -a a échoué");
    assert!(
        all.len() >= running.len(),
        "« tous » renvoie moins que « actifs »"
    );
}

#[tokio::test]
async fn a_stopped_fixture_only_appears_with_all() {
    if !enabled() {
        return;
    }
    let fixture = Fixture::start();
    let service = service().await;

    service
        .stop_container(fixture.name())
        .await
        .expect("stop a échoué");

    wait_until_listed_as_running(&service, fixture.name(), false).await;

    let all = service.list_containers(true).await.expect("ps -a a échoué");
    assert!(
        all.iter().any(|c| c.name == fixture.name()),
        "un conteneur arrêté n'apparaît pas dans « tous »"
    );
}

#[tokio::test]
async fn inspect_returns_consistent_details() {
    if !enabled() {
        return;
    }
    let fixture = Fixture::start();
    let details = service()
        .await
        .inspect_container(fixture.name())
        .await
        .expect("inspect a échoué");

    assert_eq!(details.name, fixture.name());
    assert_eq!(details.state, ContainerState::Running);
    assert!(!details.id.is_empty(), "identifiant vide");
    assert!(details.created.is_some(), "date de création absente");
}

#[tokio::test]
async fn inspect_accepts_the_container_id() {
    if !enabled() {
        return;
    }
    let fixture = Fixture::start();
    let service = service().await;

    let by_name = service
        .inspect_container(fixture.name())
        .await
        .expect("inspect par nom a échoué");
    let by_id = service
        .inspect_container(&by_name.id)
        .await
        .expect("inspect par identifiant a échoué");

    assert_eq!(by_name.id, by_id.id);
}

#[tokio::test]
async fn lifecycle_stop_start_restart_is_observable() {
    if !enabled() {
        return;
    }
    let fixture = Fixture::start();
    let service = service().await;

    service
        .stop_container(fixture.name())
        .await
        .expect("stop a échoué");
    let stopped = wait_until_state(&service, fixture.name(), false).await;
    assert!(!stopped.state.is_running(), "toujours actif après stop");

    service
        .start_container(fixture.name())
        .await
        .expect("start a échoué");
    let started = wait_until_state(&service, fixture.name(), true).await;
    assert!(started.state.is_running(), "inactif après start");

    service
        .restart_container(fixture.name())
        .await
        .expect("restart a échoué");
    let restarted = wait_until_state(&service, fixture.name(), true).await;
    assert!(restarted.state.is_running(), "inactif après restart");
    assert_eq!(restarted.id, started.id, "le conteneur a été recréé");
}

#[tokio::test]
async fn start_and_stop_are_idempotent() {
    if !enabled() {
        return;
    }
    let fixture = Fixture::start();
    let service = service().await;

    // Déjà démarré : le moteur répond 304, ce qui doit rester un succès.
    service
        .start_container(fixture.name())
        .await
        .expect("start sur un conteneur déjà actif a échoué");

    service
        .stop_container(fixture.name())
        .await
        .expect("stop a échoué");
    service
        .stop_container(fixture.name())
        .await
        .expect("stop sur un conteneur déjà arrêté a échoué");
}

#[tokio::test]
async fn a_missing_container_is_reported_as_not_found() {
    if !enabled() {
        return;
    }
    let service = service().await;
    let missing = "hormos-absent-container-9f3a2b";

    for outcome in [
        service.inspect_container(missing).await.map(|_| ()),
        service.start_container(missing).await.map(|_| ()),
        service.stop_container(missing).await.map(|_| ()),
        service.restart_container(missing).await.map(|_| ()),
    ] {
        assert_eq!(
            outcome.map_err(|e| e.kind()),
            Err(ErrorKind::ContainerNotFound),
            "un conteneur inexistant n'est pas signalé comme introuvable"
        );
    }
}

#[tokio::test]
async fn an_unreachable_socket_is_reported_as_daemon_unavailable() {
    if !enabled() {
        return;
    }
    let endpoint = hormos_docker::LocalEndpoint::resolve(
        Some("unix:///tmp/hormos-absent-socket.sock"),
        None,
        |_| false,
    )
    .expect("résolution du point de terminaison");

    let outcome = DockerRuntime::connect_to(&endpoint).await.map(|_| ());
    assert_eq!(
        outcome.map_err(|e| e.kind()),
        Err(ErrorKind::DaemonUnavailable)
    );
}

#[tokio::test]
async fn hormos_never_touches_containers_it_was_not_asked_about() {
    if !enabled() {
        return;
    }
    let fixture = Fixture::start();
    let before = foreign_container_ids(&fixture);

    let service = service().await;
    service.system_info().await.expect("info a échoué");
    service.list_containers(true).await.expect("ps -a a échoué");
    service
        .restart_container(fixture.name())
        .await
        .expect("restart a échoué");

    let after = foreign_container_ids(&fixture);
    assert_eq!(
        before, after,
        "des conteneurs étrangers au test ont été modifiés"
    );
}

/// Vérifie l'identité de suite sans dépendre d'un moteur Docker.
///
/// Ce test s'exécute toujours : la logique qui borne le nettoyage ne doit pas
/// n'être validée que sur les machines équipées d'un démon.
#[test]
fn the_suite_identity_is_stable_and_safe() {
    assert_eq!(
        suite_run_id(),
        suite_run_id(),
        "l'identité de suite change d'un appel à l'autre"
    );
    assert!(
        is_safe_label_value(suite_run_id()),
        "identité de suite inutilisable comme étiquette"
    );

    for rejected in ["", " ", "a b", "io.hormos.test=true", "run,other", "é"] {
        assert!(
            !is_safe_label_value(rejected),
            "valeur d'étiquette acceptée à tort : {rejected:?}"
        );
    }
}

/// Démontre que le nettoyage d'une exécution ne peut pas atteindre une autre.
///
/// Deux conteneurs coexistent : l'un appartient à l'exécution courante, l'autre
/// simule une suite Hormos concurrente sur le même démon. La sélection utilisée
/// par le nettoyage — étiquette générique **plus** identité d'exécution — ne doit
/// jamais franchir cette frontière, dans un sens comme dans l'autre.
#[tokio::test]
async fn cleanup_selection_never_crosses_run_boundaries() {
    if !enabled() {
        return;
    }
    let mine = Fixture::start();
    let foreign_run = format!("{}-foreign", suite_run_id());
    let theirs = Fixture::start_for_run(&foreign_run);

    let selected_for_me = containers_of_run(mine.run_id());
    let selected_for_them = containers_of_run(&foreign_run);

    let their_id = docker(&["inspect", "--format", "{{.Id}}", theirs.name()]);
    let my_id = docker(&["inspect", "--format", "{{.Id}}", mine.name()]);

    assert!(
        selected_for_me.contains(&my_id),
        "la sélection ignore un conteneur de l'exécution courante"
    );
    assert!(
        !selected_for_me.contains(&their_id),
        "la sélection de l'exécution courante atteint une autre exécution"
    );
    assert!(
        selected_for_them.contains(&their_id) && !selected_for_them.contains(&my_id),
        "la sélection de l'autre exécution atteint l'exécution courante"
    );

    // Le nettoyage de la suite simulée ne doit rien laisser derrière lui, et ne
    // doit rien retirer à l'exécution courante.
    drop(theirs);
    assert!(
        containers_of_run(&foreign_run).is_empty(),
        "la suite simulée a laissé des résidus"
    );
    assert!(
        containers_of_run(mine.run_id()).contains(&my_id),
        "le nettoyage d'une autre exécution a supprimé un conteneur de la nôtre"
    );
}
