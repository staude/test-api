mod cli;

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use ipnet::IpNet;
use test_core::{BootstrapOutcome, Core};

#[derive(Parser)]
#[command(
    name = "test-app",
    version,
    about = "Notizen mit Benutzern, Rollen und Rechten – Server, CLI und MCP in einem Binary"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// HTTP-Server starten (Web-UI, REST-API, MCP)
    Serve(ServeArgs),
    /// MCP über stdin/stdout (für lokale MCP-Clients)
    Mcp(McpArgs),
    /// Prüft, ob der lokale Server antwortet (für Docker-Healthchecks)
    Healthcheck {
        #[arg(long, env = "TESTAPP_BIND", default_value = "0.0.0.0:8080")]
        bind: SocketAddr,
    },
    /// Verwaltung direkt auf der Datenbank (Bootstrap, Notfall)
    Admin(AdminArgs),
    #[command(flatten)]
    Client(cli::ClientCommand),
}

#[derive(clap::Args)]
struct DbArgs {
    #[arg(long, env = "TESTAPP_DATABASE_URL", hide_env_values = true)]
    database_url: String,
    /// Lesezugriffe im Audit-Log protokollieren
    #[arg(long, env = "TESTAPP_AUDIT_READS", default_value_t = true, action = clap::ArgAction::Set)]
    audit_reads: bool,
    /// Audit-Einträge nach so vielen Tagen löschen (0 = nie)
    #[arg(long, env = "TESTAPP_AUDIT_RETENTION_DAYS", default_value_t = 90)]
    audit_retention_days: u32,
    /// Größe des Datenbank-Verbindungspools
    #[arg(long, env = "TESTAPP_DB_MAX_CONNECTIONS", default_value_t = 20)]
    db_max_connections: u32,
}

#[derive(clap::Args)]
struct ServeArgs {
    #[command(flatten)]
    db: DbArgs,
    #[arg(long, env = "TESTAPP_BIND", default_value = "0.0.0.0:8080")]
    bind: SocketAddr,
    #[arg(
        long,
        env = "TESTAPP_PUBLIC_URL",
        default_value = "http://localhost:8080"
    )]
    public_url: String,
    /// Kommagetrennte CIDRs, z. B. 172.16.0.0/12
    #[arg(
        long,
        env = "TESTAPP_TRUSTED_PROXIES",
        value_delimiter = ',',
        default_value = ""
    )]
    trusted_proxies: Vec<String>,
    /// Legt beim ersten Start einen Admin an
    #[arg(long, env = "TESTAPP_ADMIN_EMAIL")]
    admin_email: Option<String>,
    #[arg(long, env = "TESTAPP_ADMIN_PASSWORD", hide_env_values = true)]
    admin_password: Option<String>,
}

#[derive(clap::Args)]
struct McpArgs {
    /// stdio-Transport (derzeit der einzige lokale Modus)
    #[arg(long, default_value_t = true)]
    stdio: bool,
    #[command(flatten)]
    db: DbArgs,
    #[arg(long, env = "TESTAPP_TOKEN", hide_env_values = true)]
    token: String,
}

#[derive(clap::Args)]
struct AdminArgs {
    #[command(flatten)]
    db: DbArgs,
    #[command(subcommand)]
    command: AdminCommand,
}

#[derive(Subcommand)]
enum AdminCommand {
    /// Migrationen ausführen und Rechte/Systemrollen synchronisieren
    Migrate,
    /// Benutzer anlegen (Passwort wird abgefragt, wenn nicht angegeben)
    CreateUser {
        #[arg(long)]
        email: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long = "role", default_value = "admin")]
        roles: Vec<String>,
        #[arg(long, env = "TESTAPP_NEW_PASSWORD", hide_env_values = true)]
        password: Option<String>,
    },
    /// Passwort setzen und Benutzer entsperren
    ResetPassword {
        #[arg(long)]
        email: String,
        #[arg(long, env = "TESTAPP_NEW_PASSWORD", hide_env_values = true)]
        password: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("Fehler: {e:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Serve(args) => serve(args).await,
        Command::Mcp(args) => {
            init_tracing(LogTarget::Stderr);
            if !args.stdio {
                bail!("Nur --stdio wird unterstützt; MCP über HTTP läuft mit `serve` unter /mcp");
            }
            let core = open_core(&args.db, false).await?;
            test_mcp::serve_stdio(core, args.token)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        Command::Healthcheck { bind } => healthcheck(bind).await,
        Command::Admin(args) => admin(args).await,
        Command::Client(cmd) => cli::run(cmd).await,
    }
}

enum LogTarget {
    Stdout,
    Stderr,
}

fn init_tracing(target: LogTarget) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,tower_http=info,sqlx=warn".into());
    let json = std::env::var("TESTAPP_LOG_FORMAT").is_ok_and(|v| v == "json");
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match (target, json) {
        (LogTarget::Stdout, true) => builder.json().init(),
        (LogTarget::Stdout, false) => builder.init(),
        (LogTarget::Stderr, true) => builder.json().with_writer(std::io::stderr).init(),
        (LogTarget::Stderr, false) => builder.with_writer(std::io::stderr).init(),
    }
}

async fn open_core(db: &DbArgs, migrate: bool) -> anyhow::Result<Core> {
    let pool = test_db::connect(&db.database_url, db.db_max_connections)
        .await
        .context("Datenbankverbindung fehlgeschlagen")?;
    if migrate {
        test_db::migrate(&pool)
            .await
            .context("Migration fehlgeschlagen")?;
    }
    let options = test_core::CoreOptions {
        audit_reads: db.audit_reads,
        audit_retention_days: (db.audit_retention_days > 0).then_some(db.audit_retention_days),
    };
    Ok(Core::with_options(pool, options).await?)
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    init_tracing(LogTarget::Stdout);
    let trusted_proxies = args
        .trusted_proxies
        .iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().parse::<IpNet>())
        .collect::<Result<Vec<_>, _>>()
        .context("TESTAPP_TRUSTED_PROXIES enthält ein ungültiges CIDR")?;

    let core = open_core(&args.db, true).await?;
    // Empty values from an env file count as unset.
    let non_empty = |v: Option<String>| v.filter(|s| !s.trim().is_empty());
    let admin = non_empty(args.admin_email).zip(non_empty(args.admin_password));
    match core.bootstrap(admin).await? {
        BootstrapOutcome::Ready => {}
        BootstrapOutcome::NeedsSetup { code } => tracing::warn!(
            "Noch kein Benutzer vorhanden. Ersteinrichtung: {}/setup?code={code}",
            args.public_url.trim_end_matches('/')
        ),
    }

    let cleanup_core = core.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            match cleanup_core.cleanup().await {
                Ok(r) if r.sessions + r.audit_entries > 0 => tracing::info!(
                    sessions = r.sessions,
                    audit_entries = r.audit_entries,
                    "cleanup"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "cleanup failed"),
            }
        }
    });

    let cfg = test_app::HttpConfig {
        public_url: args.public_url,
        trusted_proxies,
    };
    let app = test_app::build_router(core, &cfg);
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    tracing::info!(bind = %args.bind, public_url = %cfg.public_url, "listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
    tracing::info!("shutting down");
}

async fn healthcheck(bind: SocketAddr) -> anyhow::Result<()> {
    let url = format!("http://127.0.0.1:{}/healthz", bind.port());
    let res = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(3))
        .send()
        .await?;
    if !res.status().is_success() {
        bail!("{url} antwortet mit {}", res.status());
    }
    Ok(())
}

async fn admin(args: AdminArgs) -> anyhow::Result<()> {
    let core = open_core(&args.db, true).await?;
    core.bootstrap(None).await?;
    match args.command {
        AdminCommand::Migrate => println!("Datenbank ist auf dem aktuellen Stand."),
        AdminCommand::CreateUser {
            email,
            name,
            roles,
            password,
        } => {
            let password = match password {
                Some(p) => p,
                None => cli::prompt_new_password()?,
            };
            let display_name =
                name.unwrap_or_else(|| email.split('@').next().unwrap_or("user").to_owned());
            let user = core
                .create_user(
                    &test_core::Principal::system(),
                    test_core::dto::NewUser {
                        email,
                        display_name,
                        password,
                        roles,
                    },
                )
                .await?;
            println!(
                "Benutzer {} angelegt ({}).",
                user.email,
                user.roles.join(", ")
            );
        }
        AdminCommand::ResetPassword { email, password } => {
            let password = match password {
                Some(p) => p,
                None => cli::prompt_new_password()?,
            };
            core.admin_reset_password(&email, password).await?;
            println!("Passwort für {email} gesetzt, alle Sitzungen beendet.");
        }
    }
    Ok(())
}
