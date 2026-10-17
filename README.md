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

## Backup

```bash
docker compose exec db pg_dump -U test test > backup-$(date +%F).sql
```
