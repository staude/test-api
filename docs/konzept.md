# Konzept: Rust-App mit API, MCP, CLI, Web-UI und RBAC

Stand: 2026-10-17 · Status: freigegeben und als Beispiel umgesetzt

Name: **test** · Repo `~/dev/test/test-api` · Binary **`test-app`**.
Fachliche Beispiel-Ressource: **Notizen** (`notes`). Sie dient nur dazu, alle
Schichten einmal komplett durchzuspielen, und lässt sich später durch die
echte Domäne ersetzen.

---

## 1. Ziele

| Anforderung | Umsetzung in einem Satz |
|---|---|
| In Rust gebaut | Ein Cargo-Workspace, ein einziges Binary `test-app`. |
| Docker-installierbar | Multi-Stage-Image, `docker compose up -d` startet App + Datenbank. |
| API first | Die REST-API ist die primäre Schnittstelle, OpenAPI-Spec wird aus dem Code erzeugt. |
| MCP-Schnittstelle | Der Server spricht MCP (Streamable HTTP unter `/mcp`, zusätzlich stdio). |
| Kommandozeile | `test-app` hat Subcommands, die als Client gegen die API arbeiten. |
| Webseite | Der Server liefert selbst eine Web-Oberfläche aus, ohne separaten Frontend-Server. |
| User-Login | Passwort-Login mit Sessions im Browser, API-Tokens für CLI und MCP. |
| Rechte und Rollen | RBAC: Benutzer haben Rollen, Rollen bündeln Rechte. |
| Datenbank | PostgreSQL mit versionierten Migrationen. |

**Nicht-Ziele für die erste Version:** Mandantenfähigkeit, SSO/OIDC,
E-Mail-Versand (Passwort-Reset per Mail), horizontale Skalierung über mehrere
Instanzen. Alles davon ist in der Architektur vorgesehen, aber nicht gebaut.

---

## 2. Grundprinzip: ein Kern, vier Zugänge

```
          Browser            CLI (test-app …)       KI-Client (Claude etc.)
             │                    │                      │
     ┌───────▼──────┐    ┌────────▼───────┐    ┌─────────▼────────┐
     │  Web-UI       │    │  REST-API      │    │  MCP-Server      │
     │  /  (HTML)    │    │  /api/v1 (JSON)│    │  /mcp  · stdio   │
     │  Cookie-Login │    │  Bearer-Token  │    │  Bearer-Token    │
     └───────┬──────┘    └────────┬───────┘    └─────────┬────────┘
             └────────────────────┼──────────────────────┘
                                  ▼
                 ┌──────────────────────────────────┐
                 │  Service-Schicht (test-core)      │
                 │  · Anwendungsfälle               │
                 │  · Rechteprüfung (authorize)     │
                 │  · Validierung, Audit-Log        │
                 └───────────────┬──────────────────┘
                                 ▼
                 ┌──────────────────────────────────┐
                 │  Repository-Schicht (sqlx)        │
                 └───────────────┬──────────────────┘
                                 ▼
                            PostgreSQL
```

Regeln, die daraus folgen:

1. **Geschäftslogik und Rechteprüfung leben ausschließlich in der
   Service-Schicht.** REST-Handler, Web-Handler und MCP-Tools sind dünne
   Adapter: Eingabe parsen, Service aufrufen, Ergebnis formatieren.
2. Jeder Service-Aufruf bekommt einen **`Principal`** (wer handelt, mit welchen
   Rechten, über welchen Kanal). Ohne `Principal` kein Zugriff.
3. Die **CLI ist ein reiner API-Client**. Sie greift nie direkt auf die
   Datenbank zu, mit einer Ausnahme: `test-app admin …` für Bootstrap und Notfälle
   (siehe 7.2).
4. Die Web-UI ruft die Services direkt im Prozess auf (kein HTTP-Umweg),
   nutzt aber exakt dieselben Funktionen wie die API.

Damit ist garantiert: Was die API darf, darf auch MCP, und was die API
verbietet, verbietet auch die Webseite.

---

## 3. Technologie-Stack

| Bereich | Wahl | Begründung |
|---|---|---|
| Sprache | Rust, Edition 2024 | wie TorroMail |
| HTTP | `axum` + `tokio` + `tower-http` | Standard im Ökosystem, gut testbar |
| Datenbank | PostgreSQL 17, `sqlx` (compile-time-geprüfte Queries, Migrationen) | robust, Rollen-/Rechte-Joins sauber abbildbar |
| OpenAPI | `utoipa` + Scalar-UI unter `/api/docs` | Spec aus dem Code, keine Doppelpflege |
| MCP | `rmcp` (offizielles Rust-SDK), Transport Streamable HTTP + stdio | offizielles SDK |
| CLI | `clap` (derive), `reqwest` als API-Client | |
| Web-UI | Server-gerendert mit `askama`-Templates + `htmx`, Assets per `rust-embed` im Binary | kein Node-Build, ein Binary liefert alles |
| Passwörter | `argon2` (Argon2id) | aktueller Standard |
| Sessions | eigene Tabelle, Cookie `HttpOnly`, `Secure`, `SameSite=Lax`, CSRF-Token | volle Kontrolle, Logout = Zeile löschen |
| Konfiguration | Umgebungsvariablen (`TESTAPP_*`), optional `.env` | 12-Factor, Docker-freundlich |
| Logging | `tracing` + `tracing-subscriber` (JSON in Docker) | |
| Fehler | `thiserror` in Bibliotheken, `anyhow` nur im Binary | |

Workspace-Lints wie in TorroMail: `unsafe_code = "forbid"`,
`clippy::unwrap_used = "deny"`, `dbg_macro`/`todo` = deny.

---

## 4. Projektstruktur

```
test-api/                      ← Repo (~/dev/test/test-api)
├── Cargo.toml                 ← Workspace
├── crates/
│   ├── test-core/             ← Domänenmodelle, Services, Rechte, Fehler-Typen
│   ├── test-db/               ← sqlx-Repositories, Migrationen
│   ├── test-rest/              ← axum-Router für /api/v1, OpenAPI
│   ├── test-web/              ← Web-UI (askama-Templates, htmx, statische Assets)
│   ├── test-mcp/              ← MCP-Server (Tools, Ressourcen)
│   ├── test-client/           ← typisierter HTTP-Client (von CLI genutzt)
│   └── test-app/                  ← Binary: main.rs, clap-Kommandos, Server-Start
├── migrations/                ← SQL-Migrationen (sqlx)
├── docker/
│   └── Dockerfile
├── docker-compose.yml
├── .env.example
├── README.md
└── CLAUDE.md
```

Abhängigkeitsrichtung: `test-app` → (`test-rest`, `test-web`, `test-mcp`,
`test-client`) → `test-core` → `test-db`-Traits. `test-core` kennt weder
axum noch MCP.

---

## 5. Datenmodell

```
users ──< user_roles >── roles ──< role_permissions >── permissions
  │
  ├──< sessions          (Browser-Login)
  ├──< api_tokens        (CLI / MCP / Skripte)
  ├──< notes             (Beispiel-Ressource, owner_id)
  └──< audit_log         (actor_id)
```

| Tabelle | Wichtige Spalten |
|---|---|
| `users` | `id uuid`, `email citext unique`, `display_name`, `password_hash`, `is_active`, `created_at`, `updated_at`, `last_login_at` |
| `roles` | `id`, `name unique`, `description`, `is_system` (System-Rollen nicht löschbar) |
| `permissions` | `key text pk` (z. B. `notes.read`), `description` |
| `role_permissions` | `role_id`, `permission_key` |
| `user_roles` | `user_id`, `role_id` |
| `sessions` | `id_hash` (nur Hash des Cookie-Werts), `user_id`, `csrf_token`, `expires_at`, `user_agent`, `ip` |
| `api_tokens` | `id`, `user_id`, `name`, `prefix` (sichtbar, z. B. `tapp_ab12`), `token_hash`, `scopes text[]`, `expires_at`, `last_used_at`, `revoked_at` |
| `audit_log` | `id`, `at`, `actor_id`, `channel` (`web`/`api`/`mcp`/`cli-admin`), `action`, `target_type`, `target_id`, `details jsonb` |
| `notes` | `id`, `owner_id`, `title`, `body`, `created_at`, `updated_at` |

Tokens und Session-IDs werden **nur gehasht** gespeichert (SHA-256 reicht,
da sie zufällig mit 256 Bit erzeugt werden). Der Klartext wird genau einmal
bei der Erstellung angezeigt.

---

## 6. Rechte und Rollen

### 6.1 Rechte

Rechte sind feste Strings im Code (`enum Permission`), werden beim Start in
die Tabelle `permissions` synchronisiert und sind damit in der Admin-UI
sichtbar. Schema: `<bereich>.<aktion>[.own]`.

| Recht | Bedeutung |
|---|---|
| `notes.read` | alle Notizen lesen |
| `notes.read.own` | nur eigene Notizen lesen |
| `notes.write` | alle Notizen anlegen/ändern/löschen |
| `notes.write.own` | nur eigene Notizen anlegen/ändern/löschen |
| `users.read` | Benutzerliste sehen |
| `users.manage` | Benutzer anlegen, sperren, Rollen zuweisen |
| `roles.manage` | Rollen anlegen und Rechte zuweisen |
| `audit.read` | Audit-Log einsehen |
| `tokens.manage.own` | eigene API-Tokens verwalten |

### 6.2 Vordefinierte Rollen

| Rolle | Rechte |
|---|---|
| `admin` | alle |
| `editor` | `notes.read`, `notes.write`, `tokens.manage.own` |
| `member` | `notes.read.own`, `notes.write.own`, `tokens.manage.own` |
| `viewer` | `notes.read`, `tokens.manage.own` |

Eigene Rollen lassen sich über API, CLI und Web-UI anlegen.

### 6.3 Prüfung

```rust
// in test-core, nie in den Adaptern
principal.require(Permission::NotesWrite)?;                    // global
principal.require_on(Permission::NotesWrite, note.owner_id)?;  // global ODER .own + Besitzer
```

Effektive Rechte eines Requests = Rechte aller Rollen des Benutzers
**geschnitten mit** den Scopes des verwendeten API-Tokens. Ein Token kann also
nie mehr als sein Benutzer, aber gezielt weniger (z. B. ein MCP-Token nur mit
`notes.read`).

Rechte werden pro Request frisch aus der DB geladen (eine Query mit Joins).
Entzieht ein Admin eine Rolle, wirkt das sofort.

---

## 7. Die vier Zugänge im Detail

### 7.1 REST-API (`/api/v1`)

- JSON, Fehler einheitlich als `application/problem+json` (RFC 9457).
- Auth: `Authorization: Bearer tapp_…` oder Session-Cookie (+ CSRF-Header
  bei schreibenden Requests).
- Paginierung per `?limit=&cursor=`.
- OpenAPI-Spec: `/api/v1/openapi.json`, Doku-UI: `/api/docs`.

| Methode | Pfad | Recht |
|---|---|---|
| `POST` | `/auth/login` | – (liefert Session-Cookie oder Token) |
| `POST` | `/auth/logout` | angemeldet |
| `GET` | `/me` | angemeldet |
| `GET/POST/DELETE` | `/me/tokens[/{id}]` | `tokens.manage.own` |
| `GET/POST` | `/notes` | `notes.read[.own]` / `notes.write[.own]` |
| `GET/PATCH/DELETE` | `/notes/{id}` | dto. mit Besitzprüfung |
| `GET/POST` | `/users` | `users.read` / `users.manage` |
| `GET/PATCH` | `/users/{id}` | `users.read` / `users.manage` |
| `PUT` | `/users/{id}/roles` | `users.manage` |
| `GET/POST` | `/roles` | angemeldet / `roles.manage` |
| `PATCH/DELETE` | `/roles/{id}` | `roles.manage` |
| `GET` | `/permissions` | angemeldet |
| `GET` | `/audit` | `audit.read` |
| `GET` | `/healthz`, `/readyz` (ohne `/api/v1`) | – |

### 7.2 Kommandozeile

Ein Binary, zwei Betriebsarten:

```bash
# Server
test-app serve                         # HTTP: Web-UI, API, MCP
test-app mcp --stdio                   # MCP über stdio (lokal für Claude Desktop/Code)

# Admin direkt auf der DB (Bootstrap, Notfall; im Container per docker exec)
test-app admin migrate
test-app admin create-user --email frank@… --role admin
test-app admin reset-password --email frank@…

# Client gegen die API
test-app login --url https://notes.example.org     # fragt Passwort, speichert Token
test-app whoami
test-app notes list [--json]
test-app notes create --title "Einkauf" --body "Milch"
test-app users list
test-app users grant --email anna@… --role editor
test-app roles create reviewer --perm notes.read --perm audit.read
test-app tokens create --name "claude" --scope notes.read
```

- Zugangsdaten liegen in `~/.config/test-app/config.toml` (Server-URL) und im
  System-Schlüsselbund (Token, via `keyring`-Crate), Fallback Datei mit
  Rechten `0600`.
- Ausgabe standardmäßig als Tabelle, mit `--json` maschinenlesbar.
- Überschreibbar per `TESTAPP_URL` / `TESTAPP_TOKEN` für Skripte und CI.

### 7.3 MCP-Server

- Transport **Streamable HTTP** unter `/mcp` (für entfernte Clients) und
  **stdio** über `test-app mcp --stdio` (für lokale Clients; liest
  `TESTAPP_URL`/`TESTAPP_TOKEN` oder läuft gegen eine lokale DB-Verbindung).
- Auth über Bearer-Token wie die API. **Ausbaustufe:** OAuth 2.1 mit
  Dynamic Client Registration, damit sich claude.ai-Connectors ohne
  manuelles Token verbinden können.
- **Tools werden pro Benutzer gefiltert:** `tools/list` zeigt nur Tools, für
  die der Token die Rechte hat. Die Prüfung beim Aufruf erfolgt trotzdem
  zusätzlich in der Service-Schicht.
- Schreibende Tools sind als `destructiveHint`/`readOnlyHint` annotiert.

| Tool | Recht |
|---|---|
| `whoami` | angemeldet |
| `notes_search`, `notes_get` | `notes.read[.own]` |
| `notes_create`, `notes_update` | `notes.write[.own]` |
| `notes_delete` | `notes.write[.own]` |
| `users_list` | `users.read` |

Ressourcen: `testapp://notes/{id}` liefert eine Notiz als Markdown.

### 7.4 Web-UI

- Server-gerendertes HTML (askama), Interaktivität mit htmx, ein kleines
  eigenes CSS ohne Framework. Alles im Binary eingebettet.
- Seiten: Login · Dashboard · Notizen (Liste, Detail, Bearbeiten) ·
  Mein Profil (Passwort ändern, API-Tokens) · Administration (Benutzer,
  Rollen & Rechte, Audit-Log).
- Navigationspunkte erscheinen nur, wenn der Benutzer das Recht hat.
- Responsive ab 360 px (gemäß `_shared/platform-web.md`).
- CSRF-Schutz: Token in der Session, als Hidden-Field bzw. htmx-Header.

---

## 8. Sicherheit

- Argon2id mit Standardparametern der Crate, Passwort-Mindestlänge 12.
- Login-Rate-Limit pro IP und pro E-Mail (`tower_governor`), gleiche
  Antwortzeit und Meldung bei unbekannter E-Mail und falschem Passwort.
- Sessions: 7 Tage gleitend, Rotation der Session-ID nach Login,
  serverseitiger Logout.
- API-Tokens: mit Präfix `tapp_` (erkennbar für Secret-Scanner), optionales
  Ablaufdatum, widerrufbar.
- Security-Header über `tower-http` (CSP ohne Inline-Scripts, HSTS hinter
  TLS, `X-Content-Type-Options`, `Referrer-Policy`).
- TLS terminiert ein Reverse Proxy (Traefik/Caddy/NPM im Homelab); die App
  wertet `X-Forwarded-*` nur von konfigurierten Proxies aus.
- Audit-Log für: Login/Logout/Fehlversuch, Benutzer-/Rollenänderungen,
  Token-Erstellung/-Widerruf, alle schreibenden Fachaktionen und – abschaltbar
  per `TESTAPP_AUDIT_READS=false` – Lesezugriffe (`note.read`, `note.list`,
  `user.read`, `user.list`). Jeder Eintrag nennt den Kanal (`web`, `api`,
  `cli`, `mcp`, `cli-admin`) und bei Token-Zugriffen die Token-Id.
- Der Kanal `cli` wird am User-Agent `test-app-cli/…` erkannt. Er ist vom
  Client selbst erklärt und daher nur eine Kennzeichnung, keine
  Sicherheitsentscheidung.

---

## 9. Docker und Betrieb

**Dockerfile** (Multi-Stage):
1. `cargo-chef` cached Abhängigkeiten.
2. Build mit `SQLX_OFFLINE=true` (Query-Metadaten im Repo unter `.sqlx/`).
3. Laufzeit-Image `gcr.io/distroless/cc-debian12:nonroot`, nur das Binary.
   Erwartete Größe ~30–40 MB. Multi-Arch `amd64` + `arm64`.

**docker-compose.yml:**

```yaml
services:
  app:
    image: test-app:local
    env_file: .env
    ports: ["8080:8080"]
    depends_on:
      db: { condition: service_healthy }
    healthcheck:
      test: ["CMD", "/test-app", "healthcheck"]
  db:
    image: postgres:17-alpine
    environment:
      POSTGRES_DB: test
      POSTGRES_USER: test
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
    volumes: [db-data:/var/lib/postgresql/data]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U test"]
volumes:
  db-data:
```

**Konfiguration** (`.env.example`):

```
TESTAPP_DATABASE_URL=postgres://test:change-me@db/test
TESTAPP_BIND=0.0.0.0:8080
TESTAPP_PUBLIC_URL=https://notes.example.org
TESTAPP_TRUSTED_PROXIES=172.16.0.0/12
TESTAPP_LOG_FORMAT=json
TESTAPP_ADMIN_EMAIL=                # optional: legt beim ersten Start einen Admin an
TESTAPP_ADMIN_PASSWORD=
POSTGRES_PASSWORD=change-me
```

**Erster Start:** Migrationen laufen automatisch beim Start. Existiert noch
kein Benutzer, wird entweder der Admin aus `TESTAPP_ADMIN_*` angelegt oder ein
einmaliger Setup-Link ins Log geschrieben.

**Backup:** `pg_dump` per Cron/Sidecar; im README dokumentiert.

---

## 10. Teststrategie

- **Unit-Tests** in `test-core`: Rechtelogik (`require`, `require_on`,
  Token-Scope-Schnittmenge) mit Tabellen-Tests.
- **Integrationstests** mit `#[sqlx::test]` (frische DB pro Test) und
  `axum`-Router per `tower::ServiceExt::oneshot`, ohne echten Port.
- **Rechte-Matrix-Test:** jeder Endpunkt × jede System-Rolle → erwarteter
  Statuscode. Schützt davor, dass ein neuer Endpunkt die Prüfung vergisst.
- **MCP-Test:** `tools/list` je Rolle, ein Tool-Aufruf mit und ohne Recht.
- CI: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`,
  Docker-Build.

---

## 11. Umsetzungsplan (Beispielimplementierung)

| Schritt | Inhalt | Ergebnis |
|---|---|---|
| 1 | Workspace, Konfiguration, Logging, `/healthz`, Dockerfile, Compose | Container startet, DB verbunden |
| 2 | Migrationen, Users/Roles/Permissions, `test-app admin` | Admin per CLI anlegbar |
| 3 | Login, Sessions, API-Tokens, `Principal`, Rechteprüfung | Auth funktioniert über API |
| 4 | Notes-Service + REST-Endpunkte + OpenAPI | API first steht |
| 5 | CLI-Client (`login`, `notes`, `users`, `roles`, `tokens`) | alles per Kommandozeile bedienbar |
| 6 | MCP-Server (HTTP + stdio) mit gefilterten Tools | Claude kann Notizen lesen/schreiben |
| 7 | Web-UI: Login, Notizen, Profil, Admin-Bereich | alles im Browser bedienbar |
| 8 | Audit-Log, Rate-Limit, Security-Header, Rechte-Matrix-Test | produktionsreif für Homelab |

Später: OAuth 2.1 für MCP, OIDC-Login (z. B. gegen Nextcloud/Authentik),
Passwort-Reset per Mail, SQLite als Alternative für Mini-Setups.

---

## 12. Entscheidungen (2026-10-17)

1. **Name:** `test` → Repo `~/dev/test/test-api`, Binary `test-app`.
2. **Domäne:** Notizen.
3. **Datenbank:** PostgreSQL.
4. **Web-UI:** serverseitig gerendert mit htmx.
5. **Registry:** keine, Image wird lokal gebaut.

---

## 13. Abweichungen in der Umsetzung

| Konzept | Umsetzung | Grund |
|---|---|---|
| Binary `test` | Binary `test-app` | `test` ist ein Shell-Builtin und würde das Binary verdecken |
| sqlx mit compile-time-geprüften Queries (`query!`) und `.sqlx/`-Offline-Daten | Laufzeit-Queries (`query_as`) | kein DB-Zugriff beim Docker-Build nötig; die Integrationstests decken jede Query ab |
| Token im System-Schlüsselbund (`keyring`) | Datei `~/.config/test-app/config.toml` mit Rechten `0600` | einfacher und plattformneutral; Schlüsselbund bleibt Ausbaustufe |
| Rate-Limit mit `tower_governor` | eigener In-Memory-Limiter im Kern | gilt so für Web- und API-Login gleichermaßen; zählt pro E-Mail und pro IP nur Fehlversuche |
| – | Zusätzlich: Eskalationsschutz | wer Rollen oder Rechte vergibt, darf nur vergeben (und entziehen), was er selbst hat |
| – | Zusätzlich: Schutz des letzten Admins | mindestens ein aktiver Benutzer behält `users.manage` |
| `ILIKE`-Suche ohne Index | Trigramm-Indizes (`pg_trgm`), Treffer werden zuerst gesammelt und dann sortiert | Ergebnis des Lasttests vom 17.10.2026; Teilwortsuche bleibt erhalten, was eine Volltextsuche nicht leisten würde |
| – | Zusätzlich: Argon2 höchstens einmal pro CPU-Kern gleichzeitig | begrenzt den Speicher bei vielen parallelen Logins |
| – | Zusätzlich: Aufbewahrungsfrist für das Audit-Log, Poolgröße konfigurierbar | Lasttest: Lese-Audit erzeugt einen Eintrag pro Seitenaufruf |
| – | Zusätzlich: Token-Erbschaft | ein eingeschränkter Token kann nur gleich oder stärker eingeschränkte Tokens erzeugen |
