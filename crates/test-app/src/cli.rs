//! Client commands: talk to a running server through the REST API.

use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use test_client::Client;
use test_core::dto::*;
use uuid::Uuid;

#[derive(Args)]
pub struct Global {
    /// Server-URL (überschreibt die gespeicherte Konfiguration)
    #[arg(long, global = true, env = "TESTAPP_URL")]
    url: Option<String>,
    /// API-Token (überschreibt die gespeicherte Konfiguration)
    #[arg(long, global = true, env = "TESTAPP_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Ausgabe als JSON
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
pub enum ClientCommand {
    /// Am Server anmelden und Token speichern
    Login {
        #[command(flatten)]
        global: Global,
        #[arg(long)]
        email: Option<String>,
    },
    /// Gespeicherten Token widerrufen und löschen
    Logout {
        #[command(flatten)]
        global: Global,
    },
    /// Angemeldeten Benutzer und Rechte anzeigen
    Whoami {
        #[command(flatten)]
        global: Global,
    },
    /// Notizen
    Notes {
        #[command(flatten)]
        global: Global,
        #[command(subcommand)]
        command: NotesCommand,
    },
    /// Benutzer verwalten
    Users {
        #[command(flatten)]
        global: Global,
        #[command(subcommand)]
        command: UsersCommand,
    },
    /// Rollen verwalten
    Roles {
        #[command(flatten)]
        global: Global,
        #[command(subcommand)]
        command: RolesCommand,
    },
    /// Alle Rechte auflisten
    Permissions {
        #[command(flatten)]
        global: Global,
    },
    /// Eigene API-Tokens verwalten
    Tokens {
        #[command(flatten)]
        global: Global,
        #[command(subcommand)]
        command: TokensCommand,
    },
    /// Audit-Log anzeigen
    Audit {
        #[command(flatten)]
        global: Global,
        #[arg(long, default_value_t = 30)]
        limit: i64,
    },
}

#[derive(Subcommand)]
pub enum NotesCommand {
    List {
        #[arg(long, short)]
        query: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    Get {
        id: Uuid,
    },
    Create {
        #[arg(long)]
        title: String,
        /// Text; `-` liest von stdin
        #[arg(long, default_value = "")]
        body: String,
    },
    Update {
        id: Uuid,
        #[arg(long)]
        title: Option<String>,
        /// Text; `-` liest von stdin
        #[arg(long)]
        body: Option<String>,
    },
    Delete {
        id: Uuid,
    },
}

#[derive(Subcommand)]
pub enum UsersCommand {
    List,
    Create {
        #[arg(long)]
        email: String,
        #[arg(long)]
        name: String,
        #[arg(long = "role")]
        roles: Vec<String>,
        #[arg(long, env = "TESTAPP_NEW_PASSWORD", hide_env_values = true)]
        password: Option<String>,
    },
    /// Rolle zuweisen
    Grant {
        #[arg(long)]
        email: String,
        #[arg(long)]
        role: String,
    },
    /// Rolle entziehen
    Revoke {
        #[arg(long)]
        email: String,
        #[arg(long)]
        role: String,
    },
    Disable {
        #[arg(long)]
        email: String,
    },
    Enable {
        #[arg(long)]
        email: String,
    },
}

#[derive(Subcommand)]
pub enum RolesCommand {
    List,
    Create {
        name: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long = "perm")]
        permissions: Vec<String>,
    },
    /// Rechte einer Rolle ersetzen
    Update {
        name: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long = "perm")]
        permissions: Option<Vec<String>>,
    },
    Delete {
        name: String,
    },
}

#[derive(Subcommand)]
pub enum TokensCommand {
    List,
    Create {
        #[arg(long)]
        name: String,
        /// Auf diese Rechte beschränken (mehrfach angeben)
        #[arg(long = "scope")]
        scopes: Vec<String>,
        #[arg(long)]
        days: Option<u32>,
    },
    Revoke {
        id: Uuid,
    },
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
    url: Option<String>,
    token: Option<String>,
}

fn config_path() -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir().context("Home-Verzeichnis nicht gefunden")?;
    Ok(home.join(".config").join("test-app").join("config.toml"))
}

fn load_config() -> Config {
    config_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(cfg: &Config) -> anyhow::Result<()> {
    let path = config_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, toml::to_string(cfg)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

impl Global {
    fn url(&self, cfg: &Config) -> String {
        self.url
            .clone()
            .or_else(|| cfg.url.clone())
            .unwrap_or_else(|| "http://localhost:8080".into())
    }

    fn client(&self) -> anyhow::Result<Client> {
        let cfg = load_config();
        let token = self.token.clone().or_else(|| cfg.token.clone());
        if token.is_none() {
            bail!("Nicht angemeldet. Erst `test-app login` ausführen oder TESTAPP_TOKEN setzen.");
        }
        Ok(Client::new(&self.url(&cfg), token))
    }
}

pub fn prompt_new_password() -> anyhow::Result<String> {
    let first = rpassword::prompt_password("Neues Passwort: ")?;
    let second = rpassword::prompt_password("Wiederholen: ")?;
    if first != second {
        bail!("Die Passwörter stimmen nicht überein");
    }
    Ok(first)
}

fn read_body(value: String) -> anyhow::Result<String> {
    if value == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        Ok(s)
    } else {
        Ok(value)
    }
}

fn print_json<T: Serialize>(v: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn print_table(headers: &[&str], rows: Vec<Vec<String>>) {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: Vec<String>| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };
    println!("{}", line(headers.iter().map(|h| h.to_string()).collect()));
    for row in rows {
        println!("{}", line(row));
    }
}

fn short(id: Uuid) -> String {
    id.to_string()
}

fn date(dt: Option<time_fmt::Dt>) -> String {
    dt.map(time_fmt::fmt).unwrap_or_else(|| "–".into())
}

mod time_fmt {
    pub type Dt = test_core::dto::OffsetDateTime;

    pub fn fmt(dt: Dt) -> String {
        let d = dt.date();
        let t = dt.time();
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            d.year(),
            u8::from(d.month()),
            d.day(),
            t.hour(),
            t.minute()
        )
    }
}

pub async fn run(cmd: ClientCommand) -> anyhow::Result<()> {
    match cmd {
        ClientCommand::Login { global, email } => {
            let mut cfg = load_config();
            let url = global.url(&cfg);
            let email = match email {
                Some(e) => e,
                None => {
                    eprint!("E-Mail: ");
                    let mut s = String::new();
                    std::io::stdin().read_line(&mut s)?;
                    s.trim().to_owned()
                }
            };
            let password = rpassword::prompt_password("Passwort: ")?;
            let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "cli".into());
            let res = Client::new(&url, None)
                .login(&email, &password, &format!("CLI ({host})"))
                .await?;
            cfg.url = Some(url.clone());
            cfg.token = res.token;
            save_config(&cfg)?;
            println!(
                "Angemeldet als {} an {url}. Token gespeichert in {}.",
                res.me.email,
                config_path()?.display()
            );
        }
        ClientCommand::Logout { global } => {
            if let Ok(client) = global.client() {
                // Best effort: the token may already be revoked.
                let _ = client.logout().await;
            }
            let mut cfg = load_config();
            cfg.token = None;
            save_config(&cfg)?;
            println!("Abgemeldet.");
        }
        ClientCommand::Whoami { global } => {
            let me = global.client()?.me().await?;
            if global.json {
                return print_json(&me);
            }
            println!("{} <{}>", me.display_name, me.email);
            println!("Rollen: {}", me.roles.join(", "));
            println!("Rechte: {}", me.permissions.join(", "));
        }
        ClientCommand::Notes { global, command } => notes(&global, command).await?,
        ClientCommand::Users { global, command } => users(&global, command).await?,
        ClientCommand::Roles { global, command } => roles(&global, command).await?,
        ClientCommand::Permissions { global } => {
            let perms = global.client()?.list_permissions().await?;
            if global.json {
                return print_json(&perms);
            }
            print_table(
                &["RECHT", "BESCHREIBUNG"],
                perms
                    .into_iter()
                    .map(|p| vec![p.key, p.description])
                    .collect(),
            );
        }
        ClientCommand::Tokens { global, command } => tokens(&global, command).await?,
        ClientCommand::Audit { global, limit } => {
            let page = global.client()?.list_audit(limit, None).await?;
            if global.json {
                return print_json(&page);
            }
            print_table(
                &["ZEIT (UTC)", "WER", "KANAL", "AKTION", "ZIEL"],
                page.items
                    .into_iter()
                    .map(|e| {
                        vec![
                            time_fmt::fmt(e.at),
                            e.actor_email.unwrap_or_else(|| "System".into()),
                            e.channel,
                            e.action,
                            e.target_id.unwrap_or_default(),
                        ]
                    })
                    .collect(),
            );
        }
    }
    Ok(())
}

async fn notes(g: &Global, cmd: NotesCommand) -> anyhow::Result<()> {
    let c = g.client()?;
    match cmd {
        NotesCommand::List { query, limit } => {
            let page = c.list_notes(query.as_deref(), limit).await?;
            if g.json {
                return print_json(&page);
            }
            print_table(
                &["ID", "TITEL", "BESITZER", "GEÄNDERT"],
                page.items
                    .into_iter()
                    .map(|n| {
                        vec![
                            short(n.id),
                            n.title,
                            n.owner_name,
                            time_fmt::fmt(n.updated_at),
                        ]
                    })
                    .collect(),
            );
        }
        NotesCommand::Get { id } => {
            let n = c.get_note(id).await?;
            if g.json {
                return print_json(&n);
            }
            println!("# {}\n", n.title);
            println!("{}", n.body);
            println!("\n— {} · {}", n.owner_name, time_fmt::fmt(n.updated_at));
        }
        NotesCommand::Create { title, body } => {
            let n = c
                .create_note(&NewNote {
                    title,
                    body: read_body(body)?,
                })
                .await?;
            if g.json {
                return print_json(&n);
            }
            println!("Notiz angelegt: {}", n.id);
        }
        NotesCommand::Update { id, title, body } => {
            let body = body.map(read_body).transpose()?;
            let n = c.update_note(id, &NoteUpdate { title, body }).await?;
            if g.json {
                return print_json(&n);
            }
            println!("Notiz {} gespeichert.", n.id);
        }
        NotesCommand::Delete { id } => {
            c.delete_note(id).await?;
            println!("Notiz {id} gelöscht.");
        }
    }
    Ok(())
}

async fn users(g: &Global, cmd: UsersCommand) -> anyhow::Result<()> {
    let c = g.client()?;
    match cmd {
        UsersCommand::List => {
            let users = c.list_users().await?;
            if g.json {
                return print_json(&users);
            }
            print_table(
                &["E-MAIL", "NAME", "ROLLEN", "AKTIV", "LETZTE ANMELDUNG"],
                users
                    .into_iter()
                    .map(|u| {
                        vec![
                            u.email,
                            u.display_name,
                            u.roles.join(","),
                            if u.is_active { "ja" } else { "nein" }.into(),
                            date(u.last_login_at),
                        ]
                    })
                    .collect(),
            );
        }
        UsersCommand::Create {
            email,
            name,
            roles,
            password,
        } => {
            let password = match password {
                Some(p) => p,
                None => prompt_new_password()?,
            };
            let u = c
                .create_user(&NewUser {
                    email,
                    display_name: name,
                    password,
                    roles,
                })
                .await?;
            if g.json {
                return print_json(&u);
            }
            println!("Benutzer {} angelegt.", u.email);
        }
        UsersCommand::Grant { email, role } => {
            let u = c.user_by_email(&email).await?;
            let mut roles = u.roles.clone();
            if !roles.contains(&role) {
                roles.push(role);
            }
            let u = c.set_user_roles(u.id, roles).await?;
            println!("{}: {}", u.email, u.roles.join(", "));
        }
        UsersCommand::Revoke { email, role } => {
            let u = c.user_by_email(&email).await?;
            let roles = u.roles.iter().filter(|r| **r != role).cloned().collect();
            let u = c.set_user_roles(u.id, roles).await?;
            println!("{}: {}", u.email, u.roles.join(", "));
        }
        UsersCommand::Disable { email } => {
            let u = c.user_by_email(&email).await?;
            c.update_user(
                u.id,
                &UserUpdate {
                    is_active: Some(false),
                    ..Default::default()
                },
            )
            .await?;
            println!("{email} gesperrt.");
        }
        UsersCommand::Enable { email } => {
            let u = c.user_by_email(&email).await?;
            c.update_user(
                u.id,
                &UserUpdate {
                    is_active: Some(true),
                    ..Default::default()
                },
            )
            .await?;
            println!("{email} entsperrt.");
        }
    }
    Ok(())
}

async fn roles(g: &Global, cmd: RolesCommand) -> anyhow::Result<()> {
    let c = g.client()?;
    match cmd {
        RolesCommand::List => {
            let roles = c.list_roles().await?;
            if g.json {
                return print_json(&roles);
            }
            print_table(
                &["ROLLE", "SYSTEM", "RECHTE"],
                roles
                    .into_iter()
                    .map(|r| {
                        vec![
                            r.name,
                            if r.is_system { "ja" } else { "" }.into(),
                            r.permissions.join(", "),
                        ]
                    })
                    .collect(),
            );
        }
        RolesCommand::Create {
            name,
            description,
            permissions,
        } => {
            let r = c
                .create_role(&NewRole {
                    name,
                    description,
                    permissions,
                })
                .await?;
            println!("Rolle {} angelegt: {}", r.name, r.permissions.join(", "));
        }
        RolesCommand::Update {
            name,
            description,
            permissions,
        } => {
            let role = c.role_by_name(&name).await?;
            let r = c
                .update_role(
                    role.id,
                    &RoleUpdate {
                        description,
                        permissions,
                    },
                )
                .await?;
            println!("Rolle {}: {}", r.name, r.permissions.join(", "));
        }
        RolesCommand::Delete { name } => {
            let role = c.role_by_name(&name).await?;
            c.delete_role(role.id).await?;
            println!("Rolle {name} gelöscht.");
        }
    }
    Ok(())
}

async fn tokens(g: &Global, cmd: TokensCommand) -> anyhow::Result<()> {
    let c = g.client()?;
    match cmd {
        TokensCommand::List => {
            let tokens = c.list_tokens().await?;
            if g.json {
                return print_json(&tokens);
            }
            print_table(
                &["ID", "NAME", "PRÄFIX", "RECHTE", "ZULETZT", "STATUS"],
                tokens
                    .into_iter()
                    .map(|t| {
                        vec![
                            short(t.id),
                            t.name,
                            t.prefix,
                            t.scopes
                                .map(|s| s.join(","))
                                .unwrap_or_else(|| "alle".into()),
                            date(t.last_used_at),
                            if t.revoked_at.is_some() {
                                "widerrufen"
                            } else {
                                "aktiv"
                            }
                            .into(),
                        ]
                    })
                    .collect(),
            );
        }
        TokensCommand::Create { name, scopes, days } => {
            let created = c
                .create_token(&NewToken {
                    name,
                    scopes: (!scopes.is_empty()).then_some(scopes),
                    expires_in_days: days,
                })
                .await?;
            if g.json {
                return print_json(&created);
            }
            println!(
                "Token erstellt (wird nur jetzt angezeigt):\n{}",
                created.secret
            );
        }
        TokensCommand::Revoke { id } => {
            c.revoke_token(id).await?;
            println!("Token {id} widerrufen.");
        }
    }
    Ok(())
}
