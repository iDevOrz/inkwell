use anyhow::{Context, Result};
use inkwell::{app, config::Config, db, health, mcp};
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("inkwell=info,tower_http=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "serve".to_string());
    let config = Config::from_env()?;

    match command.as_str() {
        "serve" => serve(config).await,
        "mcp" => {
            let state = app::build_state(config).await?;
            mcp::run_stdio(state).await
        }
        "healthcheck" => health::probe(config.bind_addr),
        "migrate" => {
            let _pool = db::connect(&config).await?;
            println!("database is up to date");
            Ok(())
        }
        "--help" | "-h" | "help" => {
            print_help();
            Ok(())
        }
        "--version" | "-V" | "version" => {
            println!("inkwell {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => anyhow::bail!("unknown command: {other}\n\nRun `inkwell --help` for usage."),
    }
}

async fn serve(config: Config) -> Result<()> {
    let bind_addr = config.bind_addr;
    let state = app::build_state(config).await?;
    app::spawn_maintenance(state.clone());
    let router = app::router(state);
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind {bind_addr}"))?;

    tracing::info!("inkwell listening on http://{bind_addr}");
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("run http server")?;
    Ok(())
}

fn print_help() {
    println!(
        "inkwell {}\n\
         \n\
         Usage:\n\
           inkwell serve        Start the web server (default)\n\
           inkwell mcp          Start the MCP server over stdio\n\
           inkwell migrate      Create/upgrade the database and exit\n\
           inkwell healthcheck  Exit 0 if the server at BIND_ADDR answers /healthz\n\
         \n\
         Environment:\n\
           BIND_ADDR        default 127.0.0.1:3000\n\
           DATA_DIR         default ./data\n\
           DATABASE_URL     default sqlite://<DATA_DIR>/inkwell.db (existing typecho.db is kept)\n\
           SITE_URL         public site URL, default http://127.0.0.1:3000\n\
           ADMIN_USER       default admin\n\
           ADMIN_PASSWORD   initial password; generated if omitted\n\
            MCP_TOKEN        optional HTTP MCP bearer token\n\
            MCP_TOKEN_SCOPES default read,write,review\n\
            COOKIE_SECURE    force Secure cookies (auto for https SITE_URL)\n\
            TRUST_PROXY_HEADERS default false; trust forwarded IP headers only behind a trusted proxy\n\
         \n\
         MCP over HTTP is available at POST /mcp with a Bearer token.\n",
        env!("CARGO_PKG_VERSION")
    );
}
