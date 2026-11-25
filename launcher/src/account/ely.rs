//! Ely.by's documented Yggdrasil API. Passwords are used once and never persisted.
//! Account IDs are namespaced so an Ely UUID cannot replace a Microsoft identity.
use super::*;
pub mod oauth;

pub const AUTH_SERVER: &str = "https://authserver.ely.by";
pub const TWO_FACTOR: &str = "This Ely.by account requires a two-factor authentication code.";
pub(super) const EXPIRED: &str = "Your Ely.by sign-in expired. Sign in again.";

fn endpoint() -> String {
    AUTH_SERVER.into()
}

fn error(status: u16, value: &Value) -> String {
    use crate::i18n::tr;
    tr(
        if value.get("errorMessage").and_then(Value::as_str)
            == Some("Account protected with two factor auth.")
        {
            TWO_FACTOR
        } else if status == 400 || status == 401 || status == 403 {
            "Ely.by refused the sign-in. Check your login, password and two-factor code."
        } else {
            "Ely.by is unavailable. Try again later."
        },
    )
    .into()
}

fn call(base: &str, action: &str, body: Value) -> Result<(u16, Value), String> {
    send(
        request_to("POST", &format!("{base}/auth/{action}")),
        Some(Body::Json(body)),
        "Ely.by",
    )
    .map_err(|_| {
        crate::i18n::tr("Could not reach Ely.by. Check your internet connection.").to_owned()
    })
}

#[cfg(test)]
fn authenticate(
    base: &str,
    login: &str,
    password: &str,
    totp: &str,
    client: &str,
) -> Result<(Account, Secrets), String> {
    let password = if totp.trim().is_empty() {
        password.to_owned()
    } else {
        format!("{password}:{}", totp.trim())
    };
    let (status, value) = call(
        base,
        "authenticate",
        json!({
            "username": login.trim(), "password": password, "clientToken": client,
            "agent": {"name": "Minecraft", "version": 1},
        }),
    )?;
    if status != 200 {
        return Err(error(status, &value));
    }
    let result = decode(&value, client)?;
    Ok(with_textures(base, result))
}

fn decode(value: &Value, client: &str) -> Result<(Account, Secrets), String> {
    let malformed = || crate::i18n::tr("Could not read the Ely.by profile.").to_owned();
    let token = str_field(value, "accessToken")
        .filter(|s| !s.is_empty())
        .ok_or_else(malformed)?;
    if value.get("clientToken").and_then(Value::as_str) != Some(client) {
        return Err(malformed());
    }
    let profile = value.get("selectedProfile").ok_or_else(malformed)?;
    let uuid = str_field(profile, "id")
        .ok_or_else(malformed)?
        .replace('-', "")
        .to_lowercase();
    let name = str_field(profile, "name").ok_or_else(malformed)?;
    if uuid.len() != 32
        || !uuid.bytes().all(|b| b.is_ascii_hexdigit())
        || !(1..=16).contains(&name.len())
        || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(malformed());
    }
    Ok((
        Account {
            uuid: format!("ely:{uuid}"),
            name,
            skin: None,
            cape: None,
        },
        Secrets {
            ely_oauth: false,
            refresh_token: client.into(),
            sign_in_required: false,
            minecraft_token: token,
            // Yggdrasil does not promise an expiry field. Refresh on every launch.
            minecraft_expires_at: now() + 7200,
            xuid: None,
            owns: false,
            ely: true,
        },
    ))
}

fn with_textures(base: &str, (mut account, secrets): (Account, Secrets)) -> (Account, Secrets) {
    use base64::Engine;
    let textures = (|| {
        let (status, profile) = send(
            request_to(
                "GET",
                &format!("{base}/session/profile/{}", account.profile_uuid()),
            ),
            None,
            "Ely.by",
        )
        .ok()?;
        if status != 200 {
            return None;
        }
        let property = profile
            .get("properties")?
            .as_array()?
            .iter()
            .find(|p| p.get("name").and_then(Value::as_str) == Some("textures"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(property.get("value")?.as_str()?)
            .ok()?;
        serde_json::from_slice::<Value>(&bytes).ok()
    })();
    if let Some(textures) = textures.and_then(|v| v.get("textures").cloned()) {
        let url = |kind: &str| {
            textures
                .get(kind)?
                .get("url")?
                .as_str()
                .filter(|url| texture_url(url))
                .map(str::to_owned)
        };
        account.skin = url("SKIN").map(|url| {
            (
                url,
                if textures
                    .pointer("/SKIN/metadata/model")
                    .and_then(Value::as_str)
                    == Some("slim")
                {
                    "slim"
                } else {
                    "classic"
                }
                .into(),
            )
        });
        account.cape = url("CAPE");
    }
    (account, secrets)
}

/// Only Ely's public texture hosts, never arbitrary URLs from a profile response.
pub fn texture_url(url: &str) -> bool {
    [
        "https://textures.ely.by/",
        "http://textures.ely.by/",
        "https://ely.by/storage/",
        "http://ely.by/storage/",
        "https://skinsystem.ely.by/",
        "http://skinsystem.ely.by/",
        "https://textures.minecraft.net/texture/",
        "http://textures.minecraft.net/texture/",
    ]
    .iter()
    .any(|prefix| url.starts_with(prefix))
}

pub(super) fn refresh(account: &Account, secrets: Secrets) -> Check {
    if secrets.ely_oauth {
        return oauth::refresh(account, secrets);
    }
    refresh_at(&endpoint(), account, secrets)
}

fn refresh_at(base: &str, account: &Account, mut secrets: Secrets) -> Check {
    if secrets.sign_in_required {
        return Check::NoAnswer(crate::i18n::tr(EXPIRED).into(), secrets);
    }
    let result = call(
        base,
        "refresh",
        json!({"accessToken": secrets.minecraft_token, "clientToken": secrets.refresh_token}),
    );
    let (status, value) = match result {
        Ok(result) => result,
        Err(e) => return Check::NoAnswer(e, secrets),
    };
    if status != 200 {
        secrets.sign_in_required = matches!(status, 400 | 401 | 403);
        return Check::NoAnswer(
            if secrets.sign_in_required {
                crate::i18n::tr(EXPIRED).into()
            } else {
                error(status, &value)
            },
            secrets,
        );
    }
    match decode(&value, &secrets.refresh_token) {
        Ok(result) if result.0.uuid == account.uuid => {
            let (profile, renewed) = with_textures(base, result);
            Check::Owns(profile, renewed)
        }
        _ => {
            secrets.sign_in_required = true;
            Check::NoAnswer(crate::i18n::tr(EXPIRED).into(), secrets)
        }
    }
}

/// Revoke this launcher's token only. Other launchers' sessions stay signed in.
pub(super) fn invalidate(secrets: &Secrets) -> Result<(), String> {
    // Ely.by documents no OAuth token-revocation endpoint. OAuth access can be
    // revoked in the account's applications page; local sign-out still forgets it.
    if secrets.ely_oauth {
        return Ok(());
    }
    let (status, value) = call(
        &endpoint(),
        "invalidate",
        json!({"accessToken": secrets.minecraft_token, "clientToken": secrets.refresh_token}),
    )?;
    if status == 200 || status == 204 {
        Ok(())
    } else {
        Err(error(status, &value))
    }
}

/// The services discovery document the unmodified game reads from `HORIZON_API_DISCOVERY`
/// (docs/microsoft-account.md: "the counterpart of authlib's `-Dminecraft.api.discovery.host`"):
/// Ely.by's session server and skin hosts where the game would use Mojang's. It has no `player`
/// service, so the game's account-status and social calls, which read this document, find
/// nothing to ask.
fn discovery_doc() -> String {
    let profile = format!("{AUTH_SERVER}/session/profile/{{profileId}}");
    serde_json::json!({"discovery": {
        "session": {"endpoints": {
            "join": {"uri": format!("{AUTH_SERVER}/session/join")},
            "verify": {"uri": format!("{AUTH_SERVER}/session/hasJoined")},
            "getProfileById": {"uri": profile}
        }},
        "profiles": {"endpoints": {
            "getTexture": {"uri": profile, "validUris": [
                "https://textures.ely.by/",
                "http://textures.ely.by/",
                "https://ely.by/storage/",
                "http://ely.by/storage/",
                "https://skinsystem.ely.by/",
                "http://skinsystem.ely.by/",
                "https://textures.minecraft.net/texture/{textureId}",
                "http://textures.minecraft.net/texture/{textureId}"
            ]},
            "getByName": {"uri": format!("{AUTH_SERVER}/api/users/profiles/minecraft/{{name}}")}
        }}
    }})
    .to_string()
}

/// The URL of a loopback server for that document, started on first use and kept for this
/// launcher's run. The game reads the document once, at its first need, so the launcher has to be
/// alive until then.
pub fn discovery_url() -> Result<String, String> {
    use std::io::{Read, Write};
    use std::sync::OnceLock;
    static URL: OnceLock<Result<String, String>> = OnceLock::new();
    URL.get_or_init(|| {
        let unavailable =
            || crate::i18n::tr("Could not start the Ely.by services for the game.").to_string();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|_| unavailable())?;
        let port = listener.local_addr().map_err(|_| unavailable())?.port();
        let body = discovery_doc();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                };
                let body = body.clone();
                std::thread::spawn(move || {
                    let mut stream = stream;
                    // The request is not looked at: this server has one document.
                    let timeout = Some(std::time::Duration::from_secs(2));
                    let _ = stream.set_read_timeout(timeout);
                    let _ = stream.set_write_timeout(timeout);
                    let _ = stream.read(&mut [0u8; 2048]);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                });
            }
        });
        Ok(format!("http://127.0.0.1:{port}/minecraft/client"))
    })
    .clone()
}

/// All of `reader` when it is at most 1 MiB (a skin is far smaller), else None: a skin host, or
/// whoever sits on the way to an `http://` one, can't make the launcher read without end.
fn capped(reader: impl std::io::Read) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader.take((1 << 20) + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= 1 << 20).then_some(bytes)
}

/// The Ely.by skin and cape as the game's offline-skin files (`skins::SKIN_ENV` and the other
/// two). The game asks Mojang's services for its own profile, which an Ely.by token can't use; it
/// takes these where neither the server nor the account gives the local player a skin, which is
/// single-player. Only Ely.by's own hosts are fetched ([`texture_url`]); a download that fails
/// leaves that file out.
pub fn skin_env(account: &Account, game_home: &Path) -> Vec<(&'static str, String)> {
    let dir = game_home.join("ely-skins");
    let fetch = |url: &str, name: &str| -> Option<String> {
        if !texture_url(url) {
            return None;
        }
        let bytes = capped(crate::backend::get(url).ok()?.into_reader())?;
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(name);
        std::fs::write(&path, bytes).ok()?;
        Some(path.display().to_string())
    };
    let mut env = Vec::new();
    if let Some((url, model)) = &account.skin {
        if let Some(path) = fetch(url, "skin.png") {
            env.push((crate::skins::SKIN_ENV, path));
            env.push((crate::skins::MODEL_ENV, model.clone()));
        }
    }
    if let Some(path) = account
        .cape
        .as_deref()
        .and_then(|url| fetch(url, "cape.png"))
    {
        env.push((crate::skins::CAPE_ENV, path));
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};

    const UUID: &str = "ffc8fdc95824509e8a57c99b940fb996";

    #[test]
    fn the_discovery_document_has_what_the_game_reads() {
        let doc: Value = serde_json::from_str(&discovery_doc()).unwrap();
        let uri = |pointer: &str| {
            doc.pointer(pointer)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        assert_eq!(
            uri("/discovery/session/endpoints/join/uri").as_deref(),
            Some("https://authserver.ely.by/session/join")
        );
        assert_eq!(
            uri("/discovery/session/endpoints/verify/uri").as_deref(),
            Some("https://authserver.ely.by/session/hasJoined")
        );
        assert_eq!(
            uri("/discovery/session/endpoints/getProfileById/uri").as_deref(),
            Some("https://authserver.ely.by/session/profile/{profileId}")
        );
        let valid = doc
            .pointer("/discovery/profiles/endpoints/getTexture/validUris")
            .and_then(Value::as_array)
            .unwrap();
        for host in [
            "https://textures.ely.by/",
            "https://textures.minecraft.net/texture/{textureId}",
        ] {
            assert!(valid.iter().any(|u| u == host), "{host}");
        }
        // No `player` service: the game's attributes and social calls find nothing to ask.
        assert!(doc.pointer("/discovery/player").is_none());
    }

    #[test]
    fn the_discovery_server_answers_the_game() {
        let url = discovery_url().unwrap();
        let (host, path) = url
            .strip_prefix("http://")
            .unwrap()
            .split_once('/')
            .unwrap();
        assert!(host.starts_with("127.0.0.1:"));
        let mut stream = std::net::TcpStream::connect(host).unwrap();
        write!(
            stream,
            "GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut text = String::new();
        stream.read_to_string(&mut text).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK"));
        assert_eq!(
            text.split("\r\n\r\n").nth(1),
            Some(discovery_doc().as_str())
        );
        assert_eq!(discovery_url().unwrap(), url, "one server per launcher run");
    }

    #[test]
    fn skin_env_fetches_nothing_from_a_foreign_host() {
        let account = Account {
            uuid: format!("ely:{UUID}"),
            name: "Notch".into(),
            skin: Some(("https://evil.example/skin.png".into(), "classic".into())),
            cape: Some("http://127.0.0.1:9/cape.png".into()),
        };
        assert!(skin_env(&account, &std::env::temp_dir()).is_empty());
    }

    #[test]
    fn skin_downloads_are_capped_at_one_mebibyte() {
        assert_eq!(capped(&b"png"[..]), Some(b"png".to_vec()));
        assert_eq!(
            capped(std::io::repeat(7).take(1 << 20)).map(|b| b.len()),
            Some(1 << 20)
        );
        assert_eq!(capped(std::io::repeat(7).take((1 << 20) + 1)), None);
        assert_eq!(
            capped(std::io::repeat(7)),
            None,
            "an endless body ends at the cap"
        );
    }

    fn answer(token: &str) -> Value {
        json!({"accessToken": token, "clientToken": "client", "selectedProfile": {"id": UUID, "name": "ElyPlayer"}})
    }

    /// Real HTTP requests, with all captured bodies returned to the test for protocol assertions.
    fn mock(
        responses: Vec<(u16, Value)>,
    ) -> (String, std::thread::JoinHandle<Vec<(String, Value)>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, response) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(&stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_owned();
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                requests.push((path, serde_json::from_slice(&body).unwrap_or(Value::Null)));
                drop(reader);
                let body = response.to_string();
                write!(stream, "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (base, thread)
    }

    #[test]
    fn sign_in_sends_totp_loads_slim_skin_and_never_stores_password() {
        use base64::Engine;
        let textures = base64::engine::general_purpose::STANDARD.encode(json!({"textures": {
            "SKIN": {"url": "https://ely.by/storage/skins/abc.png", "metadata": {"model": "slim"}},
            "CAPE": {"url": "https://ely.by/storage/cloaks/abc.png"}
        }}).to_string());
        let (base, thread) = mock(vec![
            (200, answer("access-secret")),
            (
                200,
                json!({"properties": [{"name": "textures", "value": textures}]}),
            ),
        ]);
        let (account, secrets) = authenticate(
            &base,
            " user@example.org ",
            "password-secret",
            "123456",
            "client",
        )
        .unwrap();
        let requests = thread.join().unwrap();
        assert_eq!(requests[0].0, "/auth/authenticate");
        assert_eq!(requests[0].1["username"], "user@example.org");
        assert_eq!(requests[0].1["password"], "password-secret:123456");
        assert_eq!(requests[1].0, format!("/session/profile/{UUID}"));
        assert_eq!(account.skin.as_ref().unwrap().1, "slim");
        assert!(account.cape.is_some());
        let store = MemoryStore::default();
        let mut accounts = Accounts::default();
        accounts.add(&store, account.clone(), &secrets).unwrap();
        assert!(accounts.selected.is_some());
        assert!(!accounts.has_owner(&store));
        let public = serde_json::to_string(&accounts).unwrap();
        assert!(!public.contains("access-secret"));
        let saved = store.get(&account.uuid).unwrap().unwrap();
        assert!(!saved.contains("password-secret") && !saved.contains("123456"));
        assert_eq!(account.profile_uuid(), UUID);
    }

    #[test]
    fn refresh_rotates_access_token_and_refuses_changed_identity() {
        let (account, secrets) = decode(&answer("old"), "client").unwrap();
        let (base, thread) = mock(vec![(200, answer("renewed")), (200, json!({}))]);
        let Check::Owns(_, renewed) = refresh_at(&base, &account, secrets.clone()) else {
            panic!("refresh failed")
        };
        assert_eq!(renewed.minecraft_token, "renewed");
        let requests = thread.join().unwrap();
        assert_eq!(
            requests[0],
            (
                "/auth/refresh".into(),
                json!({"accessToken": "old", "clientToken": "client"})
            )
        );
        let mut changed = answer("wrong-profile");
        changed["selectedProfile"]["id"] = json!("00000000000000000000000000000001");
        let (base, thread) = mock(vec![(200, changed)]);
        assert!(
            matches!(refresh_at(&base, &account, secrets), Check::NoAnswer(_, s) if s.sign_in_required && s.minecraft_token == "old")
        );
        thread.join().unwrap();
    }

    #[test]
    fn two_factor_errors_are_actionable_and_provider_errors_cannot_echo_secrets() {
        let (base, thread) = mock(vec![(
            401,
            json!({"errorMessage": "Account protected with two factor auth."}),
        )]);
        assert_eq!(
            authenticate(&base, "user", "secret", "", "client").unwrap_err(),
            crate::i18n::tr(TWO_FACTOR)
        );
        thread.join().unwrap();
        assert!(!error(401, &json!({"errorMessage": "echoed-secret"})).contains("echoed-secret"));
        let (account, secrets) = decode(&answer("old"), "client").unwrap();
        let (base, thread) = mock(vec![(401, json!({"errorMessage": "expired"}))]);
        assert!(
            matches!(refresh_at(&base, &account, secrets), Check::NoAnswer(_, s) if s.sign_in_required)
        );
        thread.join().unwrap();
    }

    #[test]
    fn providers_with_the_same_uuid_remain_separate_and_ely_never_unlocks_downloads() {
        let (ely, secrets) = decode(&answer("ely-token"), "client").unwrap();
        let microsoft = Account {
            uuid: UUID.into(),
            ..ely.clone()
        };
        let store = MemoryStore::default();
        let mut accounts = Accounts::default();
        accounts
            .add(
                &store,
                microsoft.clone(),
                &Secrets {
                    ely: false,
                    owns: false,
                    ..secrets.clone()
                },
            )
            .unwrap();
        accounts.add(&store, ely.clone(), &secrets).unwrap();
        assert_eq!(accounts.accounts.len(), 2);
        assert_eq!(accounts.selected(), Some(&ely));
        assert_eq!(accounts.find(&format!("ely:{UUID}")), Some(&ely));
        accounts.accounts.swap(0, 1);
        assert_eq!(accounts.find(UUID), Some(&microsoft));
        let endpoints = Endpoints {
            microsoft: String::new(),
            xbox_user: String::new(),
            xsts: String::new(),
            minecraft: "http://127.0.0.1:1".into(),
        };
        assert!(launch(&endpoints, None, &store, &mut accounts).is_err());
        assert!(store.get(UUID).unwrap().is_some());
    }
}
