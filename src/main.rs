#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use {
    std::{
        collections::HashMap,
        env,
        time::Duration,
    },
    base64::engine::{
        Engine as _,
        general_purpose::STANDARD as BASE64,
    },
    collect_mac::collect,
    futures::future::FutureExt as _,
    hmac::{
        Hmac,
        Mac as _,
    },
    itermore::IterArrayChunks as _,
    itertools::Itertools as _,
    rocket::{
        Rocket,
        State,
        async_trait,
        config::SecretKey,
        data::{
            self,
            Data,
            FromData,
            ToByteUnit as _,
        },
        http::{
            ContentType,
            Status,
        },
        outcome::Outcome,
        request::Request,
        response::content::{
            RawCss,
            RawHtml,
        },
        uri,
    },
    rocket_ws::WebSocket,
    rocket_util::{
        Doctype,
        ToHtml,
        html,
    },
    serde::Deserialize,
    serde_json::json,
    sha2::Sha256,
    tokio::{
        select,
        time::sleep,
    },
    wheel::traits::IoResultExt as _,
    crate::supervisor::{
        GefolgeWebCommitStatus,
        RepoName,
        StatusCommitStatus,
        Supervisor,
    },
};

mod supervisor;

include!(concat!(env!("OUT_DIR"), "/version.rs"));

fn page(title: &str, content: impl ToHtml) -> RawHtml<String> {
    html! {
        : Doctype;
        html {
            head {
                meta(charset = "utf-8");
                title : title;
                meta(name = "viewport", content = "width=device-width, initial-scale=1, shrink-to-fit=no");
                meta(name = "description", content = "Das Gefolge");
                meta(name = "author", content = "Fenhl & contributors");
                link(rel = "preconnect", href = "https://fonts.googleapis.com");
                link(rel = "preconnect", href = "https://fonts.gstatic.com", crossorigin);
                link(rel = "icon", sizes = "16x16", type = "image/png", href = "https://gefolge.org/static/favicon-16.png");
                link(rel = "icon", sizes = "32x32", type = "image/png", href = "https://gefolge.org/static/favicon-32.png");
                link(rel = "icon", sizes = "64x64", type = "image/png", href = "https://gefolge.org/static/favicon-64.png");
                link(rel = "icon", sizes = "128x128", type = "image/png", href = "https://gefolge.org/static/favicon-128.png");
                link(rel = "icon", sizes = "256x256", type = "image/png", href = "https://gefolge.org/static/favicon-256.png");
                link(rel = "stylesheet", href = "https://gefolge.org/static/riir.css");
                link(rel = "stylesheet", href = uri!(dejavu_css));
                link(rel = "stylesheet", href = "https://fonts.googleapis.com/css2?family=Noto+Sans:wght@400;700&display=swap");
            }
            body {
                div {
                    nav(class = "breadcrumbs") {
                        a(class = "nav") {
                            : "status . ";
                            a(href = "https://gefolge.org/") {
                                img(class = "logo", src = "https://gefolge.org/static/gefolge.png");
                            }
                        }
                    }
                    main : content;
                }
                footer {
                    p {
                        : "hosted by ";
                        a(href = "https://fenhl.net/") : "Fenhl";
                        : " • ";
                        a(href = "https://fenhl.net/disc") : "disclaimer";
                        : " • ";
                        a(href = "https://github.com/dasgefolge/status.gefolge.org") : "source code";
                    }
                    p {
                        : "Bild ";
                        a(href = "https://creativecommons.org/licenses/by-sa/2.5/deed.en") : "CC-BY-SA 2.5";
                        : " Ronald Preuss, aus Wikimedia Commons (";
                        a(href = "https://commons.wikimedia.org/wiki/File:Ritter_gefolge.jpg") : "Link";
                        : ")";
                    }
                }
            }
        }
    }
}

#[rocket::get("/")]
async fn index(supervisor: &State<Supervisor>) -> RawHtml<String> {
    let supervisor::Status { watch: _, ref gefolge_web_running, ref gefolge_web_future, ref status_future } = *supervisor.status().await;
    page("gefolge.org Status", html! {
        p(id = "websocket-state") : "Aktiviere bitte JavaScript, damit diese Seite sich automatisch aktuell halten kann, oder lade sie ab und zu neu, um den aktuellen Status zu sehen.";
        h1 {
            a(href = "https://gefolge.org/") : "gefolge.org";
        }
        p {
            : "Aktuell läuft: ";
            code(id = "gefolge-web-current") {
                a(href = format!("https://github.com/dasgefolge/gefolge.org/commit/{gefolge_web_running}")) : gefolge_web_running.to_hex_with_len(7).to_string();
            }
            : " • ";
            a(id = "gefolge-web-history", href = format!("https://github.com/dasgefolge/gefolge.org/commits/{gefolge_web_running}")) : "Verlauf";
        }
        p(id = "gefolge-web-future-empty", style? = (!gefolge_web_future.is_empty()).then_some("display: none;")) : "gefolge.org ist auf dem neusten Stand.";
        div(id = "gefolge-web-future-nonempty", style? = gefolge_web_future.is_empty().then_some("display: none;")) {
            p : "Bevorstehende Updates:";
            table {
                thead {
                    tr {
                        th : "Commit";
                        th : "Zusammenfassung";
                        th : "Status";
                    }
                }
                tbody(id = "gefolge-web-future-tbody") {
                    @for (commit_hash, commit_msg, status) in gefolge_web_future {
                        tr {
                            td {
                                code {
                                    a(href = format!("https://github.com/dasgefolge/gefolge.org/commit/{commit_hash}")) : commit_hash.to_hex_with_len(7).to_string();
                                }
                            }
                            td : commit_msg;
                            td {
                                @match status {
                                    GefolgeWebCommitStatus::Pending => : "wartet auf Abschluss anderer Builds";
                                    GefolgeWebCommitStatus::Bundled => : "übersprungen (mit dem nächsten Commit gebündelt)";
                                    GefolgeWebCommitStatus::Deploy => : "wird installiert";
                                }
                            }
                        }
                    }
                }
            }
        }
        h1 : "status.gefolge.org";
        p {
            : "Aktuell läuft: ";
            code {
                a(href = format!("https://github.com/dasgefolge/status.gefolge.org/commit/{GIT_COMMIT_HASH}")) : GIT_COMMIT_HASH.to_hex_with_len(7).to_string();
            }
            : " • ";
            a(id = "status-history", href = format!("https://github.com/dasgefolge/status.gefolge.org/commits/{GIT_COMMIT_HASH}")) : "Verlauf";
        }
        p(id = "status-future-empty", style? = (!status_future.is_empty()).then_some("display: none;")) : "status.gefolge.org ist auf dem neusten Stand.";
        div(id = "status-future-nonempty", style? = status_future.is_empty().then_some("display: none;")) {
            p : "Bevorstehende Updates:";
            table {
                thead {
                    tr {
                        th : "Commit";
                        th : "Zusammenfassung";
                        th : "Status";
                    }
                }
                tbody(id = "status-future-tbody") {
                    @for (commit_hash, commit_msg, status) in status_future {
                        tr {
                            td {
                                code {
                                    a(href = format!("https://github.com/dasgefolge/gefolge.org/commit/{commit_hash}")) : commit_hash.to_hex_with_len(7).to_string();
                                }
                            }
                            td : commit_msg;
                            td {
                                @match status {
                                    StatusCommitStatus::Pending => : "wartet auf Abschluss anderer Builds";
                                    StatusCommitStatus::Bundled => : "übersprungen (mit dem nächsten Commit gebündelt)";
                                    StatusCommitStatus::Build => : "NixOS-Konfiguration wird aktiviert";
                                }
                            }
                        }
                    }
                }
            }
        }
        script : RawHtml(include_str!("../assets/common.js"));
    })
}

#[rocket::get("/websocket")]
async fn websocket(supervisor: &State<Supervisor>, ws: WebSocket, mut shutdown: rocket::Shutdown) -> rocket_ws::Stream![] {
    let (mut gefolge_web_running, mut gefolge_web_future, mut status_future, mut watch) = {
        let supervisor::Status { watch, gefolge_web_running, gefolge_web_future, status_future } = &*supervisor.status().await;
        (gefolge_web_running.clone(), gefolge_web_future.clone(), status_future.clone(), watch.subscribe())
    };
    let supervisor = (**supervisor).clone();
    rocket_ws::Stream! { ws =>
        let _ = ws;
        // repopulate page data with payload in case it changed in between the server-side page render and the WebSocket connection
        yield rocket_ws::Message::Text(serde_json::to_string(&json!({
            "type": "change",
            "gefolgeWebRunning": gefolge_web_running.to_string(),
            "gefolgeWebFuture": gefolge_web_future.iter().map(|(commit_hash, commit_msg, status)| json!({
                "commitHash": commit_hash.to_string(),
                "commitMsg": commit_msg,
                "status": status,
            })).collect_vec(),
            "statusFuture": status_future.iter().map(|(commit_hash, commit_msg, status)| json!({
                "commitHash": commit_hash.to_string(),
                "commitMsg": commit_msg,
                "status": status,
            })).collect_vec(),
        })).unwrap());
        loop {
            select! {
                () = sleep(Duration::from_secs(30)) => {
                    yield rocket_ws::Message::Text(serde_json::to_string(&json!({
                        "type": "ping",
                    })).unwrap());
                }
                res = watch.changed() => {
                    if res.is_err() { break }
                    let status = supervisor.status().await;
                    let mut payload = collect![as HashMap<_, _>: format!("type") => json!("change")];
                    if status.gefolge_web_running != gefolge_web_running {
                        payload.insert(format!("gefolgeWebRunning"), json!(status.gefolge_web_running.to_string()));
                        gefolge_web_running = status.gefolge_web_running;
                    }
                    if status.gefolge_web_future != gefolge_web_future {
                        payload.insert(format!("gefolgeWebFuture"), json!(status.gefolge_web_future.iter().map(|(commit_hash, commit_msg, status)| json!({
                            "commitHash": commit_hash.to_string(),
                            "commitMsg": commit_msg,
                            "status": status,
                        })).collect_vec()));
                        gefolge_web_future = status.gefolge_web_future.clone();
                    }
                    if status.status_future != status_future {
                        payload.insert(format!("statusFuture"), json!(status.status_future.iter().map(|(commit_hash, commit_msg, status)| json!({
                            "commitHash": commit_hash.to_string(),
                            "commitMsg": commit_msg,
                            "status": status,
                        })).collect_vec()));
                        status_future = status.status_future.clone();
                    }
                    yield rocket_ws::Message::Text(serde_json::to_string(&payload).unwrap());
                }
                () = &mut shutdown => {
                    yield rocket_ws::Message::Text(serde_json::to_string(&json!({
                        "type": "refresh",
                    })).unwrap());
                    break
                }
            }
        }
    }
}

#[rocket::get("/dejavu-sans.css")]
fn dejavu_css() -> RawCss<&'static str> {
    RawCss(include_str!("../assets/dejavu-sans.css"))
}

#[rocket::get("/dejavu-sans-webfont.eot")]
fn dejavu_eot() -> (ContentType, &'static [u8]) {
    (ContentType::new("application", "vnd.ms-fontobject"), include_bytes!("../assets/fonts/dejavu-sans-webfont.eot"))
}

#[rocket::get("/dejavu-sans-webfont.woff2")]
fn dejavu_woff2() -> (ContentType, &'static [u8]) {
    (ContentType::WOFF2, include_bytes!("../assets/fonts/dejavu-sans-webfont.woff2"))
}

#[rocket::get("/dejavu-sans-webfont.woff")]
fn dejavu_woff() -> (ContentType, &'static [u8]) {
    (ContentType::WOFF, include_bytes!("../assets/fonts/dejavu-sans-webfont.woff"))
}

#[rocket::get("/dejavu-sans-webfont.ttf")]
fn dejavu_ttf() -> (ContentType, &'static [u8]) {
    (ContentType::TTF, include_bytes!("../assets/fonts/dejavu-sans-webfont.ttf"))
}

#[rocket::get("/dejavu-sans-webfont.svg")]
fn dejavu_svg() -> (ContentType, &'static [u8]) {
    (ContentType::SVG, include_bytes!("../assets/fonts/dejavu-sans-webfont.svg"))
}


macro_rules! guard_try {
    ($res:expr) => {
        match $res {
            Ok(x) => x,
            Err(e) => return Outcome::Error((Status::InternalServerError, e.into())),
        }
    };
}

struct SignedPayload(String);

fn is_valid_signature(signature: &str, body: &str, secret: &str) -> bool {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(body.as_bytes());
    let Some((prefix, code)) = signature.split_once('=') else { return false };
    let Ok(code) = code.chars().arrays().map(|[c1, c2]| u8::from_str_radix(&format!("{c1}{c2}"), 16)).collect::<Result<Vec<_>, _>>() else { return false };
    prefix == "sha256" && mac.verify_slice(&code).is_ok()
}

#[test]
fn test_valid_signature() {
    assert!(is_valid_signature("sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17", "Hello, World!", "It's a Secret to Everybody"))
}

#[test]
fn test_invalid_signature() {
    assert!(!is_valid_signature("sha256=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", "Hello, World!", "It's a Secret to Everybody"))
}

#[derive(Debug, thiserror::Error)]
enum PayloadError {
    #[error(transparent)] Env(#[from] env::VarError),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error("value of X-Hub-Signature-256 header is not valid")]
    InvalidSignature,
    #[error("missing X-Hub-Signature-256 header")]
    MissingSignature,
}

#[async_trait]
impl<'r> FromData<'r> for SignedPayload {
    type Error = PayloadError;

    async fn from_data(req: &'r Request<'_>, data: Data<'r>) -> data::Outcome<'r, Self, Self::Error> {
        if let Some(signature) = req.headers().get_one("X-Hub-Signature-256") {
            let body = guard_try!(data.open(2.mebibytes()).into_string().await.at_unknown());
            if is_valid_signature(signature, &body, &guard_try!(env::var("GEFOLGE_STATUS_GITHUB_WEBHOOK_SECRET"))) {
                Outcome::Success(Self(body.value))
            } else {
                Outcome::Error((Status::Unauthorized, PayloadError::InvalidSignature))
            }
        } else {
            Outcome::Error((Status::BadRequest, PayloadError::MissingSignature))
        }
    }
}

#[rocket::post("/github-webhook", data = "<payload>")]
async fn github_webhook(supervisor: &State<Supervisor>, payload: SignedPayload) -> Result<(), rocket_util::Error<serde_json::Error>> {
    #[derive(Deserialize)]
    struct GithubWebhook {
        repository: Repo,
    }

    #[derive(Deserialize)]
    struct Repo {
        name: RepoName,
    }

    let GithubWebhook { repository: Repo { name } } = serde_json::from_str(&payload.0)?;
    supervisor.handle_webhook(name).await;
    Ok(())
}

#[rocket::catch(404)]
fn not_found() -> RawHtml<String> {
    page("Not Found — gefolge.org Status", html! {
        h1 : "Fehler 404: Not Found";
        p : "Diese Seite existiert nicht.";
        p {
            a(href = uri!(index)) : "Zurück zur Hauptseite von status.gefolge.org";
        }
    })
}

#[rocket::catch(500)]
async fn internal_server_error() -> RawHtml<String> {
    let is_reported = wheel::night_report("/net/gefolge/status/error", Some("internal server error")).await.is_ok();
    page("Internal Server Error — gefolge.org Status", html! {
        h1 : "Fehler 500: Internal Server Error";
        p {
            : "Beim Laden dieser Seite ist ein Fehler aufgetreten. ";
            @if is_reported {
                : "Fenhl wurde informiert.";
            } else {
                : "Bitte melde diesen Fehler im ";
                a(href = "https://discord.com/channels/355761290809180170/397832322432499712") : "#dev";
                : ".";
            }
        }
    })
}

#[rocket::catch(default)]
async fn fallback_catcher(status: Status, request: &Request<'_>) -> RawHtml<String> {
    eprintln!("responding with unexpected HTTP status code {} {} to request {request:?}", status.code, status.reason_lossy());
    let is_reported = wheel::night_report("/net/gefolge/status/error", Some(&format!("responding with unexpected HTTP status code: {} {}", status.code, status.reason_lossy()))).await.is_ok();
    page(&format!("{} — gefolge.org Status", status.reason_lossy()), html! {
        h1 {
            : "Fehler ";
            : status.code;
            : ": ";
            : status.reason_lossy();
        }
        p {
            : "Beim Laden dieser Seite ist ein Fehler aufgetreten. ";
            @if is_reported {
                : "Fenhl wurde informiert.";
            } else {
                : "Bitte melde diesen Fehler im ";
                a(href = "https://discord.com/channels/355761290809180170/397832322432499712") : "#dev";
                : ".";
            }
        }
    })
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Base64(#[from] base64::DecodeError),
    #[error(transparent)] Env(#[from] env::VarError),
    #[error(transparent)] Rocket(#[from] rocket::Error),
    #[error(transparent)] Supervisor(#[from] supervisor::Error),
    #[error(transparent)] Task(#[from] tokio::task::JoinError),
}

#[wheel::main(rocket)]
async fn main() -> Result<(), Error> {
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = wheel::night_report_sync("/net/fenhl/status/error", Some("thread panic"));
        default_panic_hook(info)
    }));
    let (supervisor, run_supervisor) = Supervisor::new().await?;
    let rocket = rocket::custom(rocket::Config {
        secret_key: SecretKey::from(&BASE64.decode(env::var("GEFOLGE_STATUS_SECRET_KEY")?)?),
        log_level: rocket::config::LogLevel::Critical,
        port: 24826,
        ..rocket::Config::default()
    })
    .mount("/", rocket::routes![
        index,
        websocket,
        dejavu_css,
        dejavu_eot,
        dejavu_woff2,
        dejavu_woff,
        dejavu_ttf,
        dejavu_svg,
        github_webhook,
    ])
    .register("/", rocket::catchers![
        not_found,
        internal_server_error,
        fallback_catcher,
    ])
    .manage(supervisor.clone())
    .ignite().await?;
    let shutdown = rocket.shutdown();
    let rocket_task = tokio::spawn(rocket.launch()).map(|res| match res {
        Ok(Ok(Rocket { .. })) => Ok(()),
        Ok(Err(e)) => Err(Error::from(e)),
        Err(e) => Err(Error::from(e)),
    });
    let supervisor_task = tokio::spawn(run_supervisor(shutdown)).map(|res| match res {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(Error::from(e)),
        Err(e) => Err(Error::from(e)),
    });
    let ((), ()) = tokio::try_join!(rocket_task, supervisor_task)?;
    Ok(())
}
