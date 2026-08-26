//! Tests de bout en bout de la CLI **sans moteur de conteneurs**.
//!
//! Ces tests doivent rester verts sur une machine sans Docker : ils ne
//! vérifient que ce qui est décidé avant toute connexion (analyse des
//! arguments, validation des références, résolution du point de terminaison) et
//! les codes de sortie associés.
//!
//! Les scénarios nécessitant un vrai moteur vivent dans
//! `crates/hormos-docker/tests/engine.rs`.

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

/// Code de sortie « entrée invalide », partagé avec l'erreur d'usage de clap.
const EXIT_INVALID_INPUT: i32 = 2;
/// Code de sortie « moteur injoignable ».
const EXIT_DAEMON_UNAVAILABLE: i32 = 3;
/// Code de sortie « moteur ou transport non supporté ».
const EXIT_UNSUPPORTED: i32 = 8;

/// Socket qui n'existe pas : le moteur est donc forcément injoignable.
const MISSING_SOCKET: &str = "unix:///tmp/hormos-absent-socket.sock";

fn hormos() -> Command {
    let mut command = Command::cargo_bin("hormos").unwrap_or_else(|error| {
        panic!("binaire « hormos » introuvable : {error}");
    });
    // Neutralise l'environnement de la machine hôte : ces tests ne doivent
    // jamais toucher un vrai démon, même s'il tourne.
    command.env_remove("XDG_RUNTIME_DIR");
    command
}

#[test]
fn help_lists_every_command() {
    hormos().arg("--help").assert().success().stdout(
        contains("tui")
            .and(contains("info"))
            .and(contains("ps"))
            .and(contains("inspect"))
            .and(contains("start"))
            .and(contains("stop"))
            .and(contains("restart")),
    );
}

#[test]
fn version_is_printed() {
    hormos()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn the_interface_refuses_to_start_without_a_terminal() {
    // Sans sous-commande, `hormos` vise le TUI. Sortie redirigée : il refuse
    // clairement, avant même d'ouvrir le socket Docker.
    for args in [vec![], vec!["tui"]] {
        hormos()
            .args(&args)
            .assert()
            .code(EXIT_INVALID_INPUT)
            .stdout(predicates::str::is_empty())
            .stderr(contains("terminal"));
    }
}

#[test]
fn unknown_subcommand_is_a_usage_error() {
    hormos()
        .arg("exec")
        .assert()
        .code(EXIT_INVALID_INPUT)
        .stderr(contains("unrecognized").or(contains("unexpected")));
}

#[test]
fn invalid_references_are_rejected_before_any_connection() {
    // Aucun démon n'est joignable ici : si la sortie n'est pas « entrée
    // invalide », c'est que la validation a lieu trop tard.
    for reference in ["", " ", "nginx/json", "../etc/passwd", "a?b", "a#b", "a%2f"] {
        hormos()
            .env("DOCKER_HOST", MISSING_SOCKET)
            .args(["inspect", reference])
            .assert()
            .code(EXIT_INVALID_INPUT);
    }
}

#[test]
fn every_container_command_validates_its_reference() {
    for command in ["inspect", "start", "stop", "restart"] {
        hormos()
            .env("DOCKER_HOST", MISSING_SOCKET)
            .args([command, "nginx/json"])
            .assert()
            .code(EXIT_INVALID_INPUT);
    }
}

#[test]
fn an_unreachable_daemon_is_reported_distinctly() {
    hormos()
        .env("DOCKER_HOST", MISSING_SOCKET)
        .arg("info")
        .assert()
        .code(EXIT_DAEMON_UNAVAILABLE)
        .stderr(contains("injoignable"));
}

#[test]
fn remote_transports_are_refused() {
    for host in [
        "tcp://10.0.0.1:2375",
        "http://10.0.0.1:2375",
        "https://10.0.0.1:2376",
        "ssh://user@10.0.0.1",
        "npipe:////./pipe/docker_engine",
    ] {
        hormos()
            .env("DOCKER_HOST", host)
            .arg("info")
            .assert()
            .code(EXIT_UNSUPPORTED)
            .stderr(contains("non supporté"));
    }
}

#[test]
fn errors_go_to_stderr_and_leave_stdout_empty() {
    hormos()
        .env("DOCKER_HOST", MISSING_SOCKET)
        .arg("info")
        .assert()
        .failure()
        .stdout(predicates::str::is_empty());
}
