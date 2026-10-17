# test-api — Projektregeln

## Plattform

@../../_shared/platform-web.md

Abweichend vom Web-Standard-Stack: Rust statt TypeScript, kein Node-Build.
Die Web-UI wird serverseitig gerendert (askama + htmx).

## Zweck

Beispiel-App für den Aufbau „API first“: ein Rust-Binary mit REST-API,
MCP-Server, CLI und Web-UI auf einem gemeinsamen Service-Kern mit
Benutzern, Rollen und Rechten. Fachliche Beispiel-Ressource: Notizen.
Konzept: `docs/konzept.md`.

## Setup & Start

- Dev-Datenbank: `docker run -d --name test-api-pg -e POSTGRES_USER=test -e POSTGRES_PASSWORD=test -e POSTGRES_DB=test -p 55432:5432 postgres:17-alpine`
- Lokal starten: `TESTAPP_DATABASE_URL=postgres://test:test@localhost:55432/test cargo run -- serve`
- Tests: `DATABASE_URL=postgres://test:test@localhost:55432/test cargo test`
- Vor jedem Commit: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`

## Deploy

- Ziel: lokal per Docker Compose (Testprojekt, keine Registry)
- Weg: `cp .env.example .env`, anpassen, `docker compose up -d --build`

## Architekturregeln

- Geschäftslogik und Rechteprüfung **nur** in `crates/test-core`. REST-,
  Web- und MCP-Handler sind dünne Adapter: parsen, Service aufrufen, formatieren.
- Jeder Service-Aufruf bekommt einen `Principal`. Neue Rechte als Variante in
  `Permission` anlegen; sie werden beim Start in die DB synchronisiert.
- Neue REST-Endpunkte: `#[utoipa::path]` setzen, in `ApiDoc` eintragen und in
  den Matrix-Test `permission_matrix` (`crates/test-app/tests/api.rs`) aufnehmen.
- Neue MCP-Tools: in `tool_allowed` (`crates/test-mcp/src/lib.rs`) eintragen,
  sonst sind sie unsichtbar und nicht aufrufbar.
- SQL nur in `crates/test-db`. Queries werden zur Laufzeit geprüft (kein
  `query!`-Makro), deshalb deckt jede Query ein Integrationstest ab.
- Nutzerseitige Texte (Fehler, UI, CLI) auf Deutsch, Code und Kommentare auf Englisch.

## Besonderheiten

- Binary heißt `test-app`, nicht `test` (Kollision mit dem Shell-Builtin).
- Umgebungsvariablen haben das Präfix `TESTAPP_`, API-Tokens das Präfix `tapp_`.
