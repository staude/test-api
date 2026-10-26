# test-app

Notizen-App mit Benutzern, Rollen und Rechten – als Beispiel für eine
„API first“-Anwendung in Rust. Ein einziges Binary liefert:

- **REST-API** unter `/api/v1` mit OpenAPI-Spec (`/api/v1/openapi.json`) und Doku (`/api/docs`)
- **MCP-Server** unter `/mcp` (Streamable HTTP) und lokal über stdio
- **Web-UI** unter `/` (serverseitig gerendert, htmx)
- **CLI**: `test-app …` als Client gegen die API, plus `test-app admin …` direkt auf der DB

Alle vier Zugänge nutzen denselben Service-Kern mit derselben Rechteprüfung.
Details: [docs/konzept.md](docs/konzept.md).

## Screenshots

| | |
|---|---|
| ![Notizen](docs/screenshots/02-notizen.png) | ![Audit-Log](docs/screenshots/07-audit.png) |
| ![Rollen](docs/screenshots/06-rollen.png) | ![Dunkelmodus](docs/screenshots/09-dunkel.png) |

Weitere Bilder in [docs/screenshots](docs/screenshots).

## Schnellstart mit Docker

```bash
cp .env.example .env          # POSTGRES_PASSWORD setzen
docker compose up -d --build
docker compose logs app | grep setup   # Einrichtungslink für den ersten Admin
```

Dann den Link öffnen (Standard: `http://localhost:8080/setup?code=…`) und den
ersten Administrator anlegen. Alternativ `TESTAPP_ADMIN_EMAIL` und
`TESTAPP_ADMIN_PASSWORD` in `.env` setzen, oder per CLI im Container:

```bash
docker compose exec app /test-app admin create-user --email du@example.org --role admin
```

## Lokale Entwicklung

```bash
docker run -d --name test-api-pg -e POSTGRES_USER=test -e POSTGRES_PASSWORD=test \
  -e POSTGRES_DB=test -p 55432:5432 postgres:17-alpine

export TESTAPP_DATABASE_URL=postgres://test:test@localhost:55432/test
cargo run -- admin create-user --email du@example.org --role admin
cargo run -- serve
```

Tests (jeder Integrationstest bekommt eine frische Datenbank):

```bash
DATABASE_URL=postgres://test:test@localhost:55432/test cargo test
```

## Kommandozeile

```bash
test-app login --url http://localhost:8080      # fragt E-Mail und Passwort, speichert Token
test-app whoami
test-app notes list -q einkauf
test-app notes create --title "Einkauf" --body "Milch, Brot"
echo "langer Text" | test-app notes update <id> --body -
test-app users create --email anna@example.org --name Anna --role member
test-app users grant --email anna@example.org --role editor
test-app roles create reviewer --perm notes.read --perm audit.read
test-app tokens create --name claude --scope notes.read --scope notes.read.own
test-app audit --limit 20
test-app notes list --json                      # maschinenlesbar
```

Der Token liegt in `~/.config/test-app/config.toml` (Rechte `0600`).
`TESTAPP_URL` und `TESTAPP_TOKEN` überschreiben die Datei.

## MCP

**Entfernt (HTTP):** In der Web-UI unter *Profil → API-Tokens* einen Token
erstellen, am besten auf die nötigen Rechte beschränkt. Dann im MCP-Client
eintragen:

- URL: `https://<host>/mcp`
- Header: `Authorization: Bearer tapp_…`

**Lokal (stdio)**, z. B. für Claude Desktop:

```json
{
  "mcpServers": {
    "test-app": {
      "command": "/pfad/zu/test-app",
      "args": ["mcp", "--stdio"],
      "env": {
        "TESTAPP_DATABASE_URL": "postgres://test:test@localhost:55432/test",
        "TESTAPP_TOKEN": "tapp_…"
      }
    }
  }
}
```

Die Tools (`whoami`, `notes_search`, `notes_get`, `notes_create`,
`notes_update`, `notes_delete`, `users_list`) werden pro Token nach Rechten
gefiltert. Notizen sind zusätzlich als Ressourcen `testapp://notes/{id}` lesbar.

## Rollen

| Rolle | Rechte |
|---|---|
| `admin` | alle |
| `editor` | alle Notizen lesen und schreiben |
| `member` | nur eigene Notizen |
| `viewer` | alle Notizen lesen |

Eigene Rollen lassen sich anlegen. Niemand kann Rechte vergeben, die er selbst
nicht hat. Mindestens ein aktiver Benutzer behält immer `users.manage`.

## Konfiguration

| Variable | Standard | Bedeutung |
|---|---|---|
| `TESTAPP_DATABASE_URL` | – | PostgreSQL-Verbindung (Pflicht) |
| `TESTAPP_BIND` | `0.0.0.0:8080` | Adresse und Port |
| `TESTAPP_PUBLIC_URL` | `http://localhost:8080` | externe URL; `https://` aktiviert Secure-Cookies und HSTS |
| `TESTAPP_TRUSTED_PROXIES` | leer | CIDRs, deren `X-Forwarded-For` vertraut wird |
| `TESTAPP_ADMIN_EMAIL` / `_PASSWORD` | leer | erster Admin beim ersten Start |
| `TESTAPP_AUDIT_READS` | `true` | Lesezugriffe (Notizen ansehen/suchen, Benutzer ansehen) protokollieren |
| `TESTAPP_AUDIT_RETENTION_DAYS` | `90` | Audit-Einträge nach so vielen Tagen löschen, `0` = nie |
| `TESTAPP_DB_MAX_CONNECTIONS` | `20` | Größe des Datenbank-Verbindungspools (im Lasttest optimal, 30 bringt nichts mehr) |
| `TESTAPP_LOG_FORMAT` | `text` | `json` für strukturierte Logs |
| `RUST_LOG` | `info` | Log-Filter |

## Lasttest

`loadtest/web.js` ist ein k6-Skript mit drei Szenarien (`realistic`, `stress`, `login`). Es läuft
gegen einen eigenen Compose-Stack, damit Daten und Audit-Log des normalen Stacks sauber bleiben:

```bash
TESTAPP_PORT=18091 docker compose -p test-api-load up -d
docker compose -p test-api-load exec -e TESTAPP_NEW_PASSWORD=lasttest-passwort-1 app \
  /test-app admin create-user --email load@example.org --role admin
loadtest/run.sh stress stress
TESTAPP_PORT=18091 docker compose -p test-api-load down -v
```

Ergebnisse vom 17.10.2026 (nach den Optimierungen, 10.000 Notizen): 200 gleichzeitige Nutzer bei
p95 6 ms, unter Volllast 4.100 Seiten/s bei p95 19 ms (vorher 1.600 Seiten/s, p95 76 ms), keine Fehler.

## Lokale API-Dokumentation

Die lokale Docker-Compose-Installation liegt unter `~/dev/test-api`.

- Web-UI: <http://localhost:8080/>
- API-Basis: <http://localhost:8080/api/v1>
- Interaktive API-Dokumentation: <http://localhost:8080/api/docs>
- OpenAPI-Spezifikation: <http://localhost:8080/api/v1/openapi.json>

### Wichtige Endpunkte

Alle folgenden API-Pfade sind relativ zu `/api/v1`:

- `POST /auth/login`: Anmeldung mit E-Mail und Passwort; standardmäßig wird ein
  API-Token zurückgegeben, alternativ ist eine Sitzung möglich.
- `POST /auth/logout`: Sitzung beenden oder den verwendeten Token widerrufen.
- `GET /me`: angemeldeten Benutzer und dessen effektive Rechte abrufen.
- `GET /notes`, `POST /notes`: Notizen auflisten bzw. anlegen.
- `GET /notes/{id}`, `PATCH /notes/{id}`, `DELETE /notes/{id}`:
  einzelne Notiz lesen, bearbeiten bzw. löschen.
- `/users`, `/roles`, `/permissions`, `/audit`: Benutzer, Rollen, Rechte und
  Audit-Protokoll; unterstützte Methoden und Datenmodelle stehen in der OpenAPI-Doku.

Geschützte Endpunkte benötigen eine Anmeldung und die passenden Rechte.
API-Tokens werden im Header `Authorization: Bearer <Token>` übermittelt.
Tokens und Passwörter nicht in Quellcode, Dokumentation oder geteilte Logs übernehmen.

### Gesundheitsprüfung und Testaufrufe

Diese Endpunkte benötigen keine Anmeldung:

```bash
curl --fail --show-error http://localhost:8080/healthz
curl --fail --show-error http://localhost:8080/readyz
curl --fail --show-error http://localhost:8080/api/v1/openapi.json
```

`/healthz` prüft die Erreichbarkeit der App. `/readyz` prüft zusätzlich die
Datenbankverbindung: Bei Erfolg antworten beide mit HTTP 200 und `ok`;
bei nicht verfügbarer Datenbank liefert `/readyz` HTTP 503.

Zugriffsschutz ohne Zugangsdaten prüfen:

```bash
curl --include http://localhost:8080/api/v1/notes
curl --include http://localhost:8080/api/v1/me
```

Erwartet wird jeweils HTTP 401 mit einer JSON-Fehlerantwort
(`status: 401`, `detail: "Anmeldung erforderlich"`), nicht HTTP 200.

Beim lokalen Smoke-Test am 26.10.2026 wurden die drei öffentlichen Endpunkte
erfolgreich geprüft; die OpenAPI-3.1.0-Spezifikation enthielt 15 Pfade.
Beide geschützten Endpunkte wiesen anonyme Anfragen korrekt mit HTTP 401 ab.
App- und Datenbankcontainer waren `healthy`. Authentifizierte Lese- und
Schreiboperationen wurden nicht getestet; der Smoke-Test änderte keine Nutzdaten.

## Wartung der lokalen Docker-Installation

Voraussetzung: Docker Desktop läuft. Alle folgenden Befehle werden im
Projektordner ausgeführt:

```bash
cd ~/dev/test-api
```

### Start, Stopp und Diagnose

```bash
docker compose ps           # Containerzustand anzeigen
docker compose stop         # Stoppen, Daten behalten
docker compose up -d        # Starten
docker compose restart app  # Nur die App neu starten
curl --fail --show-error http://localhost:8080/readyz
```

Logs bei Bedarf lokal ansehen:

```bash
docker compose logs --tail=100 app db
```

Vor dem Teilen auf vertrauliche Inhalte prüfen: App-Logs können den geheimen
Einrichtungslink für den ersten Administrator enthalten. Der Docker-Healthcheck
prüft `/healthz`; für eine Prüfung inklusive Datenbank zusätzlich `/readyz` verwenden.

### Konfiguration

Lokale Einstellungen liegen in `.env` (nicht versioniert). Die aktuelle Installation
verwendet ein zufälliges Datenbankpasswort und `TESTAPP_PORT=127.0.0.1:8080`.
Damit ist der veröffentlichte App-Port nur auf dem eigenen Mac erreichbar.

- `.env` enthält Zugangsdaten: nicht veröffentlichen oder committen.
- Dateirechte auf den eigenen Benutzer beschränken (`chmod 600 .env`).
- Nach Änderungen mit `docker compose up -d` die betroffenen Container neu erstellen.
- Das Datenbankpasswort nicht nur in `.env` ändern: Bei einem bestehenden
  PostgreSQL-Volume wird das gespeicherte Passwort dadurch nicht automatisch geändert.

### Backup

Vor Updates die Datenbank sichern. Das Custom-Format lässt sich mit `pg_restore`
wiederherstellen. Backups können personenbezogene Daten und Authentifizierungsdaten
enthalten; deshalb mit eingeschränkten Rechten speichern:

```bash
mkdir -p backups
(umask 077; docker compose exec -T db pg_dump -U test -d test -Fc > "backups/test-$(date +%Y%m%d-%H%M%S).dump")
```

Den erfolgreichen Abschluss des Dumps prüfen. Backups zusätzlich außerhalb des
Projektordners sicher aufbewahren und niemals committen. Wiederherstellungen
regelmäßig in einer separaten Testdatenbank prüfen, nicht ungeprüft in der laufenden
Datenbank.

### Aktualisieren

Zuerst ein Backup erstellen, dann:

```bash
git status --short
git pull --ff-only
docker compose up -d --build
docker compose ps
curl --fail --show-error http://localhost:8080/readyz
```

Bei eigenen Änderungen zuerst deren Umgang klären, bevor der Stand aktualisiert
wird. Datenbankmigrationen laufen beim App-Start automatisch. Ein Zurücksetzen
nur des App-Images macht bereits ausgeführte Migrationen nicht rückgängig.

### Datenhaltung

PostgreSQL speichert seine Daten im persistenten Compose-Volume `db-data`.
`docker compose stop` und `docker compose down` behalten dieses Volume.
**`docker compose down -v` löscht das Volume und damit die lokale Datenbank.**
